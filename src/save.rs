//! Voyage persistence: store the captain's progress so a voyage survives quitting
//! the game — on the desktop *and* in the browser (itch.io's localStorage) alike.
//!
//! Only *voyage state* is saved: the seed (the world itself is regenerated from
//! it, bit-identically — see `world.rs`), the [`GameState`] (gold, cargo, hull,
//! missions, the booked race, where the captain is), the ship's [`Kinematics`]
//! (position + trim), and a little ambient context (the day/night clock, the sail
//! notch, the wind's quarter and its shift schedule). Transient scenery — traders, flotsam, the eased
//! weather — is reseeded fresh on load and needn't be stored.
//!
//! The format is a tiny versioned, line-based `key=value` text (no serde, to keep
//! the wasm lean): a magic header line, then one field per line, with `mission=`
//! lines for each accepted contract and a `race=` line for a booked race. An
//! unrecognised or truncated save simply fails to parse and is ignored, so a stale
//! or corrupt entry never crashes a boot — the captain just starts a fresh voyage.
//!
//! Storage backend (see [`backend`]): native writes a `.sav` file beside the
//! executable; the web build calls three `localStorage` shims imported from the JS
//! loader (see `web/index.html`). Both are keyed by [`KEY`].

use std::collections::HashMap;

use crate::game_state::{GameState, Good, Inventory, Location, Stats};
use crate::geometry::Vec2;
use crate::mission::Mission;
use crate::race::Race;
use crate::sailing::Kinematics;
use crate::tavern::SpecialItem;

/// The storage key (a `localStorage` entry on the web, a `<KEY>.sav` file natively).
const KEY: &str = "sailrs_save";
/// A separate key for global preferences that aren't voyage state (so they survive a
/// change of world and don't ride the per-seed save): the audio and graphics
/// settings from the pause menu's Options (see [`Settings`]).
const SETTINGS_KEY: &str = "sailrs_settings";
/// Marks that the captain has seen the guide at least once, so it only pops up by
/// itself on a first voyage (see `crate::guide`). A global flag, not voyage state:
/// it must outlive any one chart and not ride the per-seed save.
const GUIDE_KEY: &str = "sailrs_guide_seen";
/// First line of a save; bumped if the format changes so old saves are rejected.
const MAGIC: &str = "sailrs-save-v1";

/// A captured voyage, enough to resume exactly where the captain left off. The
/// world is *not* stored — it is regenerated from `seed`.
#[derive(Clone, Debug)]
pub struct Save {
    pub seed: i64,
    pub gs: GameState,
    pub kin: Kinematics,
    /// Day/night clock in [0, 1).
    pub tod: f32,
    /// The discrete sail notch (0 None · 1 Half · 2 Full).
    pub sail_mode: usize,
    /// The prevailing wind's bearing (the quarter it blows *toward*, radians).
    pub wind_toward: f32,
    /// The wind RNG's raw state, so the shift schedule continues across a reload
    /// instead of replaying from the world seed. `None` in an older save; the
    /// RNG is then freshly reseeded, as it always was.
    pub wind_rng: Option<u64>,
    /// Seconds of the current wind period already sailed when saved, so the next
    /// shift lands on schedule rather than a full period after loading. Zero when
    /// an older save doesn't carry it.
    pub wind_since: f32,
    /// The racing rival's live kinematics, if it is on the water (a booked race that
    /// has been cast off from a port). `None` when no race is afoot or it hasn't
    /// started — the rival is respawned at the line when the captain next sets sail.
    pub rival: Option<Kinematics>,
    /// The race's two-stage phase (see `main`): the player has drawn up alongside
    /// the waiting rival (`race_ready`), and the gun has fired (`race_running`). Both
    /// false unless `rival` is `Some`; restored so a running race resumes mid-course
    /// rather than rewinding to the approach.
    pub race_ready: bool,
    pub race_running: bool,
}

impl Save {
    /// Serialise to the line-based text format.
    pub fn serialize(&self) -> String {
        let gs = &self.gs;
        let k = &self.kin;
        let mut o = String::new();
        o.push_str(MAGIC);
        o.push('\n');
        kv(&mut o, "seed", &self.seed.to_string());
        kv(&mut o, "gold", &gs.gold.to_string());
        kv(&mut o, "hold", &gs.hold_capacity.to_string());
        kv(&mut o, "hull_level", &gs.hull_level.to_string());
        kv(&mut o, "sail_level", &gs.sail_level.to_string());
        kv(&mut o, "hull", &gs.hull.to_string());
        kv(&mut o, "hull_wear", &gs.hull_wear.to_string());
        let cargo = gs
            .cargo
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        kv(&mut o, "cargo", &cargo);
        match gs.location {
            Location::Sailing => kv(&mut o, "loc", "sailing"),
            Location::Docked(id) => kv(&mut o, "loc", &format!("docked,{id}")),
        }
        kv(&mut o, "tod", &self.tod.to_string());
        kv(&mut o, "sail_mode", &self.sail_mode.to_string());
        kv(&mut o, "wind", &self.wind_toward.to_string());
        if let Some(state) = self.wind_rng {
            kv(&mut o, "wind_rng", &state.to_string());
        }
        kv(&mut o, "wind_since", &self.wind_since.to_string());
        kv(&mut o, "kin", &fmt_kin(k));
        // The lifetime tally (see `Stats`). A single comma-joined line; absent from
        // pre-stats saves, which load with a zeroed ledger (see `deserialize`).
        let s = &gs.stats;
        kv(
            &mut o,
            "stats",
            &format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                s.contracts_fulfilled,
                s.contract_earnings,
                s.races_won,
                s.races_lost,
                s.race_winnings,
                s.meters_traveled,
                s.flotsam_collected,
                s.flotsam_gold,
                s.days_passed,
                s.times_docked,
                s.hull_repairs,
                s.upgrades_bought,
                s.log_opened,
                s.sails_fully_opened,
                s.cargo_lost_overboard
            ),
        );
        // Tavern wares (see `crate::tavern`): the owned set as a list of slot indices,
        // and every ware's last-used day (for the active wares' daily cooldown). Both
        // absent from pre-tavern saves, which load an empty kit (see `deserialize`).
        let owned: String = gs
            .items
            .owned
            .iter()
            .enumerate()
            .filter(|&(_, &on)| on)
            .map(|(i, _)| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        kv(&mut o, "items", &owned);
        let item_days = gs
            .items
            .last_used_day
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(",");
        kv(&mut o, "item_days", &item_days);

        // A rival on the water: its position and the race phase, so a running race
        // resumes exactly rather than restarting the approach.
        if let Some(rk) = self.rival {
            kv(&mut o, "rival", &fmt_kin(&rk));
            kv(&mut o, "race_ready", if self.race_ready { "1" } else { "0" });
            kv(&mut o, "race_running", if self.race_running { "1" } else { "0" });
        }
        for m in &gs.active_missions {
            kv(
                &mut o,
                "mission",
                &format!(
                    "{},{},{},{},{},{},{},{}",
                    m.id,
                    m.good.index(),
                    m.quantity,
                    m.origin_id,
                    m.target_id,
                    m.reward,
                    m.deposit,
                    m.lost
                ),
            );
        }
        if let Some(r) = gs.race {
            kv(
                &mut o,
                "race",
                &format!("{},{},{},{}", r.origin_id, r.target_id, r.stake, r.required_level),
            );
        }
        o
    }

    /// Parse a save from its text, or `None` if the header is wrong or any field is
    /// missing/malformed — a corrupt or older save is simply ignored.
    pub fn deserialize(s: &str) -> Option<Save> {
        let mut lines = s.lines();
        if lines.next()?.trim() != MAGIC {
            return None;
        }
        let mut map: HashMap<&str, &str> = HashMap::new();
        let mut missions: Vec<Mission> = Vec::new();
        let mut race: Option<Race> = None;
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, val) = line.split_once('=')?;
            match key {
                "mission" => missions.push(parse_mission(val)?),
                "race" => race = Some(parse_race(val)?),
                _ => {
                    map.insert(key, val);
                }
            }
        }

        let mut cargo = [0i32; 8];
        let parts: Vec<&str> = map.get("cargo")?.split(',').collect();
        if parts.len() != cargo.len() {
            return None;
        }
        for (slot, p) in cargo.iter_mut().zip(parts) {
            *slot = p.parse().ok()?;
        }

        let location = match *map.get("loc")? {
            "sailing" => Location::Sailing,
            other => {
                let id = other.strip_prefix("docked,")?.parse().ok()?;
                Location::Docked(id)
            }
        };

        let gs = GameState {
            gold: map.get("gold")?.parse().ok()?,
            cargo,
            hold_capacity: map.get("hold")?.parse().ok()?,
            location,
            hull_level: map.get("hull_level")?.parse().ok()?,
            sail_level: map.get("sail_level")?.parse().ok()?,
            hull: map.get("hull")?.parse().ok()?,
            hull_wear: map.get("hull_wear")?.parse().ok()?,
            active_missions: missions,
            race,
            // The tally is optional: a save written before stats existed (or any
            // malformed line) loads a fresh, zeroed ledger rather than failing.
            stats: map.get("stats").map(|v| parse_stats(v)).unwrap_or_default(),
            // Tavern wares are optional too: a pre-tavern save loads an empty kit.
            items: parse_inventory(map.get("items"), map.get("item_days")),
        };

        let kin = parse_kin(map.get("kin")?)?;
        // The rival and race phase are optional (absent unless a race is on the
        // water, and absent from older saves): default to no rival, race not started.
        let rival = match map.get("rival") {
            Some(v) => Some(parse_kin(v)?),
            None => None,
        };
        let race_ready = map.get("race_ready").map(|v| *v == "1").unwrap_or(false);
        let race_running = map.get("race_running").map(|v| *v == "1").unwrap_or(false);

        Some(Save {
            seed: map.get("seed")?.parse().ok()?,
            gs,
            kin,
            tod: map.get("tod")?.parse().ok()?,
            sail_mode: map.get("sail_mode")?.parse().ok()?,
            wind_toward: map.get("wind")?.parse().ok()?,
            // The wind schedule is optional (absent from older saves): without it
            // the RNG reseeds and the period timer starts fresh, as before.
            wind_rng: map.get("wind_rng").and_then(|v| v.parse().ok()),
            wind_since: map
                .get("wind_since")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            rival,
            race_ready,
            race_running,
        })
    }

    /// Write this save to the storage backend.
    pub fn store(&self) {
        backend::write(KEY, &self.serialize());
    }

    /// Read the saved voyage, if any (and if it parses).
    pub fn load() -> Option<Save> {
        Save::deserialize(&backend::read(KEY)?)
    }
}

/// Capture the live voyage and persist it in one call — the loop's autosave/quit
/// hooks hand it the pieces directly so it needn't reach into the game module.
#[allow(clippy::too_many_arguments)]
pub fn store(
    seed: i64,
    gs: &GameState,
    kin: &Kinematics,
    tod: f32,
    sail_mode: usize,
    wind_toward: f32,
    wind_rng: u64,
    wind_since: f32,
    rival: Option<Kinematics>,
    race_ready: bool,
    race_running: bool,
) {
    Save {
        seed,
        gs: gs.clone(),
        kin: *kin,
        tod,
        sail_mode,
        wind_toward,
        wind_rng: Some(wind_rng),
        wind_since,
        rival,
        race_ready,
        race_running,
    }
    .store();
}

/// Forget the saved voyage (e.g. a future "abandon voyage" affordance).
#[allow(dead_code)]
pub fn clear() {
    backend::remove(KEY);
}

// --- Global settings (the pause menu's audio + graphics preferences) ----------

/// First line of the settings blob; bumped if the format changes. The pre-v1
/// settings file was a bare scenery-density number; [`Settings::parse`] still
/// reads that, so an old file keeps its density preference.
const SETTINGS_MAGIC: &str = "sailrs-settings-v1";

/// The persisted preferences, one field per Options row that should outlive a
/// launch. Every field is optional: `None` means the stored blob doesn't carry it
/// (an older file, a hand-edited one, or no file at all), so the caller keeps its
/// built-in default rather than being forced onto ours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Scenery-density level (the performance slider).
    pub feat_density: Option<usize>,
    /// Master volume, 0..1.
    pub volume: Option<f32>,
    /// Post-process bloom (native only; forced off on the web regardless).
    pub bloom: Option<bool>,
    /// 4x MSAA on the scene (native only, likewise).
    pub msaa: Option<bool>,
    /// Whether the window opens full-screen (applied by `window_conf`).
    pub fullscreen: Option<bool>,
}

impl Settings {
    /// Serialise to the same line-based `key=value` text the voyage save uses.
    /// Only the fields that are `Some` are written, so a partial record
    /// round-trips as itself.
    fn serialize(&self) -> String {
        let mut o = String::new();
        o.push_str(SETTINGS_MAGIC);
        o.push('\n');
        if let Some(v) = self.feat_density {
            kv(&mut o, "density", &v.to_string());
        }
        if let Some(v) = self.volume {
            kv(&mut o, "volume", &v.to_string());
        }
        for (key, val) in [("bloom", self.bloom), ("msaa", self.msaa), ("fullscreen", self.fullscreen)] {
            if let Some(v) = val {
                kv(&mut o, key, if v { "1" } else { "0" });
            }
        }
        o
    }

    /// Parse leniently: every line stands alone, and an unknown or malformed one
    /// is skipped, so a settings file from another build keeps whatever it can.
    /// A blob without the magic header is read as the legacy format (a bare
    /// scenery-density number); anything else parses to all-defaults.
    fn parse(s: &str) -> Settings {
        let mut lines = s.lines();
        if lines.next().map(str::trim) != Some(SETTINGS_MAGIC) {
            return Settings {
                feat_density: s.trim().parse().ok(),
                ..Settings::default()
            };
        }
        let mut out = Settings::default();
        for line in lines {
            let Some((key, val)) = line.trim().split_once('=') else {
                continue;
            };
            match key {
                "density" => out.feat_density = val.parse().ok(),
                "volume" => out.volume = val.parse().ok(),
                "bloom" => out.bloom = parse_flag(val),
                "msaa" => out.msaa = parse_flag(val),
                "fullscreen" => out.fullscreen = parse_flag(val),
                _ => {}
            }
        }
        out
    }
}

/// A settings boolean: exactly "1" or "0"; anything else is treated as absent.
fn parse_flag(v: &str) -> Option<bool> {
    match v {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// Persist the global preferences (see [`SETTINGS_KEY`]).
pub fn store_settings(s: &Settings) {
    backend::write(SETTINGS_KEY, &s.serialize());
}

/// Read the saved preferences; every field the store doesn't carry is `None`.
pub fn load_settings() -> Settings {
    backend::read(SETTINGS_KEY)
        .map(|t| Settings::parse(&t))
        .unwrap_or_default()
}

/// Whether the captain has already been shown the guide on a previous launch.
pub fn guide_seen() -> bool {
    backend::read(GUIDE_KEY).map(|s| s.trim() == "1").unwrap_or(false)
}

/// Remember that the guide has been shown, so it won't open itself again.
pub fn store_guide_seen() {
    backend::write(GUIDE_KEY, "1");
}

// --- Serialisation helpers ---------------------------------------------------

/// Append a `key=value` line.
fn kv(out: &mut String, key: &str, val: &str) {
    out.push_str(key);
    out.push('=');
    out.push_str(val);
    out.push('\n');
}

/// A `Kinematics` as six comma-separated floats: pos.x, pos.y, heading, vel.x,
/// vel.y, yaw_rate.
fn fmt_kin(k: &Kinematics) -> String {
    format!(
        "{},{},{},{},{},{}",
        k.pos.x, k.pos.y, k.heading_rad, k.vel.x, k.vel.y, k.yaw_rate
    )
}

/// Parse the six floats [`fmt_kin`] writes back into a `Kinematics`.
fn parse_kin(v: &str) -> Option<Kinematics> {
    let p: Vec<f32> = v
        .split(',')
        .map(|x| x.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    if p.len() != 6 {
        return None;
    }
    Some(Kinematics {
        pos: Vec2::new(p[0], p[1]),
        heading_rad: p[2],
        vel: Vec2::new(p[3], p[4]),
        yaw_rate: p[5],
    })
}

/// Parse a `mission=` line's comma-separated fields. The trailing `lost` tally
/// is a later addition: a seven-field line from an older save loads with
/// nothing lost.
fn parse_mission(v: &str) -> Option<Mission> {
    let p: Vec<&str> = v.split(',').collect();
    if p.len() < 7 || p.len() > 8 {
        return None;
    }
    let good = *Good::ALL.get(p[1].parse::<usize>().ok()?)?;
    Some(Mission {
        id: p[0].parse().ok()?,
        good,
        quantity: p[2].parse().ok()?,
        origin_id: p[3].parse().ok()?,
        target_id: p[4].parse().ok()?,
        reward: p[5].parse().ok()?,
        deposit: p[6].parse().ok()?,
        lost: match p.get(7) {
            Some(s) => s.parse().ok()?,
            None => 0,
        },
    })
}

/// Parse a `stats=` line (the lifetime tally) leniently: each field is read by
/// position, and any field that's missing or malformed defaults to zero. So a save
/// from a build with fewer tally fields (or a future one with more) keeps whatever
/// it can rather than discarding the whole record — a format bump never silently
/// wipes the captain's tally, it just zero-fills the parts it doesn't recognise.
fn parse_stats(v: &str) -> Stats {
    let p: Vec<&str> = v.split(',').collect();
    let field = |i: usize| p.get(i).copied().unwrap_or("");
    Stats {
        contracts_fulfilled: field(0).parse().unwrap_or(0),
        contract_earnings: field(1).parse().unwrap_or(0),
        races_won: field(2).parse().unwrap_or(0),
        races_lost: field(3).parse().unwrap_or(0),
        race_winnings: field(4).parse().unwrap_or(0),
        meters_traveled: field(5).parse().unwrap_or(0.0),
        flotsam_collected: field(6).parse().unwrap_or(0),
        flotsam_gold: field(7).parse().unwrap_or(0),
        days_passed: field(8).parse().unwrap_or(0),
        times_docked: field(9).parse().unwrap_or(0),
        hull_repairs: field(10).parse().unwrap_or(0),
        upgrades_bought: field(11).parse().unwrap_or(0),
        log_opened: field(12).parse().unwrap_or(0),
        sails_fully_opened: field(13).parse().unwrap_or(0),
        cargo_lost_overboard: field(14).parse().unwrap_or(0),
    }
}

/// Rebuild the tavern kit from its two save lines, leniently: `owned` is a list of
/// owned ware slot indices, `days` the per-ware last-used day (for the daily
/// cooldown). Either may be absent (a pre-tavern save) or carry indices a future/older
/// build doesn't know — unknown slots are simply skipped, so the kit never fails to
/// load, it just keeps the wares it recognises.
fn parse_inventory(owned: Option<&&str>, days: Option<&&str>) -> Inventory {
    let mut inv = Inventory::default();
    if let Some(owned) = owned {
        for tok in owned.split(',').filter(|s| !s.is_empty()) {
            if let Some(slot) = tok.parse::<usize>().ok().filter(|&i| i < SpecialItem::COUNT) {
                inv.owned[slot] = true;
            }
        }
    }
    if let Some(days) = days {
        for (slot, tok) in days.split(',').enumerate() {
            if slot < SpecialItem::COUNT {
                if let Ok(d) = tok.parse::<i32>() {
                    inv.last_used_day[slot] = d;
                }
            }
        }
    }
    inv
}

/// Parse a `race=` line's four comma-separated fields.
fn parse_race(v: &str) -> Option<Race> {
    let p: Vec<&str> = v.split(',').collect();
    if p.len() != 4 {
        return None;
    }
    Some(Race {
        origin_id: p[0].parse().ok()?,
        target_id: p[1].parse().ok()?,
        stake: p[2].parse().ok()?,
        required_level: p[3].parse().ok()?,
    })
}

// --- Storage backend ---------------------------------------------------------

/// Native: a plain text file beside the executable. The voyage is tiny, so a flat
/// file with no directories or extra crates is plenty.
#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::path::PathBuf;

    /// `<exe dir>/<key>.sav`, or `./<key>.sav` if the exe path can't be resolved.
    fn path(key: &str) -> PathBuf {
        let mut p = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        p.push(format!("{key}.sav"));
        p
    }

    pub fn write(key: &str, val: &str) {
        let _ = std::fs::write(path(key), val);
    }

    pub fn read(key: &str) -> Option<String> {
        std::fs::read_to_string(path(key)).ok()
    }

    pub fn remove(key: &str) {
        let _ = std::fs::remove_file(path(key));
    }
}

/// Web: the browser's `localStorage`, reached through three shims the JS loader
/// adds to the wasm import object (see the `miniquad_add_plugin` call in
/// `web/index.html`). Strings cross the boundary as `(ptr, len)` into wasm memory;
/// the read shim writes the value back into a buffer we own and returns its length
/// (or `-1` when the key is absent), so no allocator need be shared with JS.
#[cfg(target_arch = "wasm32")]
mod backend {
    unsafe extern "C" {
        fn localstorage_set(kp: *const u8, kl: u32, vp: *const u8, vl: u32);
        fn localstorage_get(kp: *const u8, kl: u32, op: *mut u8, oc: u32) -> i32;
        fn localstorage_remove(kp: *const u8, kl: u32);
    }

    pub fn write(key: &str, val: &str) {
        unsafe {
            localstorage_set(
                key.as_ptr(),
                key.len() as u32,
                val.as_ptr(),
                val.len() as u32,
            );
        }
    }

    pub fn read(key: &str) -> Option<String> {
        // The save is well under a kilobyte; 16 KiB is ample headroom and the read
        // happens once, at boot.
        let mut buf = vec![0u8; 16 * 1024];
        let n = unsafe {
            localstorage_get(
                key.as_ptr(),
                key.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if n < 0 {
            return None;
        }
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }

    pub fn remove(key: &str) {
        unsafe { localstorage_remove(key.as_ptr(), key.len() as u32) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::Good;

    fn sample() -> Save {
        let mut gs = GameState::start();
        gs.gold = 1234;
        gs.cargo[Good::Rum.index()] = 5;
        gs.cargo[Good::Plank.index()] = 3;
        gs.hull_level = 2;
        gs.sail_level = 1;
        gs.hold_capacity = 24;
        gs.hull = 200;
        gs.hull_wear = 0.42;
        gs.location = Location::Docked(7);
        gs.active_missions.push(Mission {
            id: 11,
            good: Good::Spice,
            quantity: 8,
            origin_id: 7,
            target_id: 9,
            reward: 300,
            deposit: 264,
            lost: 2,
        });
        gs.race = Some(Race {
            origin_id: 7,
            target_id: 3,
            stake: 450,
            required_level: 1,
        });
        gs.stats = Stats {
            contracts_fulfilled: 17,
            contract_earnings: 9_300,
            races_won: 4,
            races_lost: 2,
            race_winnings: -150,
            meters_traveled: 123_456.5,
            flotsam_collected: 31,
            flotsam_gold: 1_870,
            days_passed: 12,
            times_docked: 23,
            hull_repairs: 6,
            upgrades_bought: 5,
            log_opened: 9,
            sails_fully_opened: 7,
            cargo_lost_overboard: 3,
        };
        // Own a couple of wares, one of them an active used on day 5.
        gs.items.owned[SpecialItem::WorldMap.index()] = true;
        gs.items.owned[SpecialItem::WindWhistle.index()] = true;
        gs.items.last_used_day[SpecialItem::WindWhistle.index()] = 5;
        Save {
            seed: 42,
            gs,
            kin: Kinematics {
                pos: Vec2::new(-1234.5, 6789.0),
                heading_rad: 1.25,
                vel: Vec2::new(0.5, -0.25),
                yaw_rate: 0.1,
            },
            tod: 0.37,
            sail_mode: 2,
            wind_toward: -2.1,
            wind_rng: Some(0x1234_5678_9abc_def0),
            wind_since: 173.25,
            rival: Some(Kinematics {
                pos: Vec2::new(220.0, -90.0),
                heading_rad: -0.5,
                vel: Vec2::new(3.0, 1.0),
                yaw_rate: -0.05,
            }),
            race_ready: true,
            race_running: true,
        }
    }

    #[test]
    fn round_trips_a_full_voyage() {
        let s = sample();
        let back = Save::deserialize(&s.serialize()).expect("should parse");
        assert_eq!(back.seed, s.seed);
        assert_eq!(back.gs.gold, s.gs.gold);
        assert_eq!(back.gs.cargo, s.gs.cargo);
        assert_eq!(back.gs.hold_capacity, s.gs.hold_capacity);
        assert_eq!(back.gs.hull_level, s.gs.hull_level);
        assert_eq!(back.gs.sail_level, s.gs.sail_level);
        assert_eq!(back.gs.hull, s.gs.hull);
        assert!((back.gs.hull_wear - s.gs.hull_wear).abs() < 1e-9);
        assert_eq!(back.gs.location, s.gs.location);
        assert_eq!(back.gs.active_missions, s.gs.active_missions);
        assert_eq!(back.gs.race, s.gs.race);
        assert_eq!(back.gs.stats, s.gs.stats);
        assert_eq!(back.gs.items, s.gs.items);
        assert_eq!(back.sail_mode, s.sail_mode);
        assert_eq!(back.kin.pos, s.kin.pos);
        assert!((back.kin.heading_rad - s.kin.heading_rad).abs() < 1e-6);
        assert!((back.tod - s.tod).abs() < 1e-6);
        assert!((back.wind_toward - s.wind_toward).abs() < 1e-6);
        assert_eq!(back.wind_rng, s.wind_rng);
        assert!((back.wind_since - s.wind_since).abs() < 1e-6);
        assert_eq!(back.race_ready, s.race_ready);
        assert_eq!(back.race_running, s.race_running);
        assert_eq!(back.rival.map(|r| r.pos), s.rival.map(|r| r.pos));
        assert!(
            (back.rival.unwrap().heading_rad - s.rival.unwrap().heading_rad).abs() < 1e-6
        );
    }

    #[test]
    fn defaults_no_rival_when_absent() {
        // A save with no race on the water round-trips to no rival, race not started.
        let mut s = sample();
        s.rival = None;
        s.race_ready = false;
        s.race_running = false;
        let back = Save::deserialize(&s.serialize()).expect("should parse");
        assert!(back.rival.is_none());
        assert!(!back.race_ready);
        assert!(!back.race_running);
    }

    #[test]
    fn a_short_stats_line_keeps_what_it_can_and_zero_fills_the_rest() {
        // A stats line from an earlier build with fewer fields must not discard the
        // tally wholesale — it reads the leading fields and zeroes the unknown tail.
        let s = parse_stats("7,1200");
        assert_eq!(s.contracts_fulfilled, 7);
        assert_eq!(s.contract_earnings, 1200);
        assert_eq!(s.races_won, 0);
        assert_eq!(s.meters_traveled, 0.0);
    }

    #[test]
    fn a_save_without_a_stats_line_loads_a_zeroed_tally() {
        // Pre-stats saves carry no `stats=` line at all; they must still load.
        let mut s = sample();
        s.gs.stats = Stats::default();
        let text = s.serialize();
        let stripped: String = text
            .lines()
            .filter(|l| !l.starts_with("stats="))
            .collect::<Vec<_>>()
            .join("\n");
        let back = Save::deserialize(&stripped).expect("should parse");
        assert_eq!(back.gs.stats, Stats::default());
    }

    #[test]
    fn a_save_without_tavern_lines_loads_an_empty_kit() {
        // Pre-tavern saves carry no `items=`/`item_days=` lines; they must still load,
        // with the captain owning nothing.
        let mut s = sample();
        s.gs.items = Inventory::default();
        let text = s.serialize();
        let stripped: String = text
            .lines()
            .filter(|l| !l.starts_with("items=") && !l.starts_with("item_days="))
            .collect::<Vec<_>>()
            .join("\n");
        let back = Save::deserialize(&stripped).expect("should parse");
        assert_eq!(back.gs.items, Inventory::default());
    }

    #[test]
    fn a_save_without_a_wind_schedule_still_loads() {
        // Older saves carry only the wind's quarter: the RNG then reseeds and the
        // period timer starts fresh, exactly the pre-schedule behaviour.
        let s = sample();
        let text = s.serialize();
        let stripped: String = text
            .lines()
            .filter(|l| !l.starts_with("wind_rng=") && !l.starts_with("wind_since="))
            .collect::<Vec<_>>()
            .join("\n");
        let back = Save::deserialize(&stripped).expect("should parse");
        assert_eq!(back.wind_rng, None);
        assert_eq!(back.wind_since, 0.0);
        assert!((back.wind_toward - s.wind_toward).abs() < 1e-6);
    }

    #[test]
    fn settings_round_trip() {
        let s = Settings {
            feat_density: Some(3),
            volume: Some(0.4),
            bloom: Some(false),
            msaa: Some(true),
            fullscreen: Some(false),
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn partial_settings_round_trip_leaving_the_rest_absent() {
        let s = Settings {
            volume: Some(0.7),
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn legacy_bare_density_settings_still_read() {
        // The pre-v1 settings file was just the density level as a bare number.
        let s = Settings::parse("2");
        assert_eq!(s.feat_density, Some(2));
        assert_eq!(s.volume, None);
        assert_eq!(s.fullscreen, None);
    }

    #[test]
    fn garbage_settings_parse_to_defaults() {
        assert_eq!(Settings::parse("not a setting"), Settings::default());
        assert_eq!(
            Settings::parse("sailrs-settings-v1\nvolume=loud\nbloom=maybe\n"),
            Settings::default()
        );
    }

    #[test]
    fn rejects_a_bad_header() {
        assert!(Save::deserialize("not-a-save\ngold=5\n").is_none());
    }

    #[test]
    fn rejects_a_truncated_save() {
        // Header present but the required fields are missing.
        assert!(Save::deserialize("sailrs-save-v1\ngold=5\n").is_none());
    }
}
