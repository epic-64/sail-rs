//! sail-rs — a macroquad port of the ScalaJS sailing game.
//!
//! This stage sets up the first-person scene (sky gradient, sun, horizon) and the
//! world-anchored wave system, and lets you sail a free camera around the swell.
//! Islands, the ship deck/rig, HUD and weather come in later stages.

mod bloom;
mod captains_log;
mod celestial;
mod flotsam;
mod flotsam_render;
mod font;
mod game_state;
mod geometry;
mod isle_features;
mod islands_render;
mod minimap;
mod mission;
mod ocean;
mod ocean_renderer;
mod palette;
mod pause_menu;
mod port_view;
mod projection;
mod race;
mod rival_render;
mod rng;
mod sailing;
mod save;
mod scene;
mod ship_render;
mod sound;
mod spray;
mod touch;
mod touch_ui;
mod trader;
mod ui;
mod weather;
mod world;

use macroquad::prelude::*;

use game_state::{hull, upgrades, GameState, Market};
use geometry::{clamp, compass, wrap_angle, Vec2};
use port_view::Harbor;
use ocean_renderer::OceanRenderer;
use projection::MAX_VIEW;
use rng::Rng;
use sailing::{Helm, Kinematics, Wind};
use ship_render::{RigInput, ShipRenderer};
use spray::{Spray, SprayInput};
use ui::{format_dist, px};
use weather::{Weather, WeatherState};
use world::Island;

fn window_conf() -> Conf {
    // MSAA needs a WebGL2 context on the web (its resolve uses the WebGL2-only
    // `blitFramebuffer`/`readBuffer`). But `getContext("webgl2")` is not granted
    // everywhere — itch.io embeds the game in a cross-origin iframe and some
    // browser/extension setups return a *null* WebGL2 context, which crashes on
    // the first GL query. So the web build uses a plain WebGL1 context (the
    // miniquad default, far more widely granted) and skips MSAA — the maximally
    // compatible config. Native is unaffected: it keeps 4× MSAA, and
    // `webgl_version` is ignored off-web anyway.
    #[cfg(target_arch = "wasm32")]
    let sample_count = 1; // no MSAA on the web → no WebGL2 dependency
    #[cfg(not(target_arch = "wasm32"))]
    let sample_count = 4; // MSAA: smooth the wave-quad edges (native)

    Conf {
        window_title: "sail-rs".to_owned(),
        window_width: 1280,
        window_height: 720,
        fullscreen: true, // launch full-screen (toggle in the pause menu's Options)
        high_dpi: true,
        sample_count,
        ..Default::default()
    }
}

#[inline]
fn lerp3(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

#[inline]
fn rgb(c: (f32, f32, f32)) -> Color {
    Color::new(c.0 / 255.0, c.1 / 255.0, c.2 / 255.0, 1.0)
}

/// Smoothstep: 0 below `e0`, 1 above `e1`, eased in between.
#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Paint the sky as a vertical three-stop gradient (top → mid → horizon), eased
/// toward the storm overcast by `storm`. `sky` is the clock's fair-weather gradient.
///
/// The sky is treated as a skybox locked to the sun's bearing: a warm "lit" dome
/// toward the sun and a cool, dark dome away from it, the split sharpening as the
/// sun nears the horizon. So at sunrise the eastern sky around the sun glows while
/// the west, the sides and the zenith stay night-dark; at sunset the red sits on
/// the sun's side and the opposite sky has already gone dark. The directional split
/// is gated to the low-sun `twilight` window (active whether the sun is just above
/// *or* just below the horizon) and weighted toward the horizon, so high-noon and
/// deep-night skies stay uniform. Because the bearing is taken relative to the
/// heading, the bright side stays pinned to the sun as the helm swings the view.
/// Built as one mesh, since macroquad has no built-in gradient.
fn draw_sky(
    view: &scene::SkyView,
    sky: [(f32, f32, f32); 3],
    storm: f32,
    m: f32,
    sun_az: f32,
    sun_alt: f32,
) {
    let scene::SkyView {
        heading,
        half_fov_h: half_fov,
        w,
        horizon,
    } = *view;
    // The two gradients blended between horizontally: the clock's "lit" sky and the
    // cool night sky stood in for the un-sunlit side. Both eased toward the overcast.
    let storm_blend = |g: [(f32, f32, f32); 3]| {
        [
            lerp3(g[0], palette::STORM_SKY[0], storm),
            lerp3(g[1], palette::STORM_SKY[1], storm),
            lerp3(g[2], palette::STORM_SKY[2], storm),
        ]
    };
    let lit = storm_blend(sky);
    let dark = storm_blend(palette::fair_sky(palette::Daytime::Night));

    // Vertical three-stop sample (top → mid → horizon) at `t` in [0, 1].
    let grad = |g: &[(f32, f32, f32); 3], t: f32| {
        if t < 0.5 {
            lerp3(g[0], g[1], t * 2.0)
        } else {
            lerp3(g[1], g[2], (t - 0.5) * 2.0)
        }
    };

    // The whole sky darkens to the night gradient as the sun sinks past the horizon,
    // so no warm tint lingers overhead once it's down (a uniform vertical fall).
    let base_night = palette::night_factor(sun_alt); // 0 by day, 1 once set
    // The strength of the *directional* split: a bell centred on the sun sitting on
    // the horizon, so it rises as the sun approaches the sea line (from above on the
    // way down, from below on the way up) and fades both at high noon and at the dead
    // of night. This is what makes the warm side appear while the sun is still up —
    // the old code keyed the split off `base_night`, which is zero until the sun has
    // already set, so a sunrise lit every bearing equally.
    const TWILIGHT_WIDTH: f32 = 0.34;
    let twilight = (-(sun_alt / TWILIGHT_WIDTH).powi(2)).exp();
    // The sun's bearing across the view (relative to the heading).
    let rel_sun = wrap_angle(sun_az - heading);
    // Angular half-width of the warm glow around the sun's bearing.
    const GLOW_WIDTH: f32 = 0.85;

    // Backstop fill (covered by the mesh) so a hard camera tilt never bares the
    // cleared background past the gradient's edges.
    let back = lerp3(grad(&lit, 0.0), grad(&dark, 0.0), base_night);
    draw_rectangle(-m, -m, w + 2.0 * m, horizon + m, rgb(back));

    if twilight <= 0.001 {
        // No twilight split (high day, or deep night): a plain vertical gradient —
        // eased uniformly toward night by `base_night` — is enough.
        let strips = 96;
        let strip_h = horizon / strips as f32;
        for i in 0..strips {
            let t = i as f32 / (strips - 1) as f32;
            let y = i as f32 * strip_h;
            let c = lerp3(grad(&lit, t), grad(&dark, t), base_night);
            draw_rectangle(-m, y, w + 2.0 * m, strip_h + 1.0, rgb(c));
        }
        return;
    }

    // Directional gradient as a grid mesh: rows give the vertical gradient, columns
    // the sideways lit→dark blend by angle from the sun. Kept small enough that the
    // index count stays under macroquad's per-drawcall limit (max_indices = 5000;
    // 24×32×6 = 4608); the per-vertex colours interpolate smoothly across each quad.
    let cols = 24usize;
    let rows = 32usize;
    let x0 = -m;
    let x1 = w + m;
    let y0 = -m;
    let y1 = horizon;
    let mut vertices: Vec<Vertex> = Vec::with_capacity((cols + 1) * (rows + 1));
    for r in 0..=rows {
        let fy = r as f32 / rows as f32;
        let y = y0 + (y1 - y0) * fy;
        // Vertical gradient parameter: clamp the over-scan above y=0 to the top stop.
        let t = clamp(y / horizon, 0.0, 1.0);
        let lit_c = grad(&lit, t);
        let dark_c = grad(&dark, t);
        // The split is confined to the lower sky (the zenith stays uniform, tracking
        // `base_night` only) and fades out toward the very top.
        let horizon_band = smoothstep(0.30, 0.95, t);
        for c in 0..=cols {
            let fx = c as f32 / cols as f32;
            let x = x0 + (x1 - x0) * fx;
            // This column's bearing relative to the heading, and its angle from the
            // sun; the warm glow falls off as a soft bell around the sun's bearing.
            let rel_col = (x - w * 0.5) / (w * 0.5) * half_fov;
            let sep = wrap_angle(rel_col - rel_sun);
            let glow = (-(sep / GLOW_WIDTH).powi(2)).exp();
            // Two directional pushes, both scaled by the twilight strength and limited
            // to the lower sky:
            //   warm_keep — near the sun, hold the warm "lit" sky even after the sun
            //               has dipped (spares the afterglow from `base_night`);
            //   dark_push — away from the sun, pull toward the cool night sky even
            //               while the sun is still up, so the anti-solar sky and the
            //               sides darken at sunrise/sunset instead of brightening.
            let warm_keep = glow * horizon_band * twilight;
            let dark_push = (1.0 - glow) * horizon_band * twilight;
            let night_amt =
                clamp(base_night * (1.0 - warm_keep) + dark_push * (1.0 - base_night), 0.0, 1.0);
            let col = lerp3(lit_c, dark_c, night_amt);
            vertices.push(Vertex::new(x, y, 0.0, fx, fy, rgb(col)));
        }
    }
    let stride = (cols + 1) as u16;
    let mut indices: Vec<u16> = Vec::with_capacity(cols * rows * 6);
    for r in 0..rows as u16 {
        for c in 0..cols as u16 {
            let i0 = r * stride + c;
            let i1 = i0 + 1;
            let i2 = i0 + stride;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i1, i2, i1, i3, i2]);
        }
    }
    draw_mesh(&Mesh {
        vertices,
        indices,
        texture: None,
    });
}

/// Discrete sail settings: W raises a notch, S lowers one. Each maps to a sail
/// fraction (the throttle) the hull accelerates toward — it sets a *target* speed,
/// never the speed itself, so you set sail once and the ship keeps going.
/// (`SailingView.sailFractions` / `sailNames`.)
const SAIL_FRACTIONS: [f32; 3] = [0.0, 0.5, 1.0];
const SAIL_NAMES: [&str; 3] = ["None", "Half", "Full"];

// --- Camera "ride": how the viewpoint rocks with the boat ---------------------
// The whole world (sky, sun, waves, islands) is drawn through a camera that tilts
// *opposite* the boat's lean (the sea stays level while the captain heels), nods
// as the bow pitches, and swings as the hull yaws — so sailing reads as motion
// rather than a static cockpit. All tunable; flip a sign if a motion reads
// backwards on screen.
const CAM_ROLL_GAIN: f32 = 0.55; // horizon tilt as a fraction of (swell roll + heel)
const CAM_ROLL_MAX_DEG: f32 = 14.0; // clamp, so the over-scan margin always covers
// Camera pitch "look": the bow climbing a wave face tips the view skyward (the
// horizon drops) and sliding down the back tips it down. Driven by the *shaped*
// fore-aft pitch (`ocean::pitch_response`), pushed well past a true camera pitch so
// a swell reads as a drastic glance up the face and a steep plunge down the back.
const PITCH_LOOK_GAIN: f32 = 650.0; // px the horizon shifts per rad of shaped pitch
const CAMERA_DIVE_EXTRA: f32 = 1.0; // extra camera-only gain on the downward glance (1.0 = symmetric)
const CAM_PITCH_MAX: f32 = 170.0; // clamp, so the over-scan margin always covers
const CAM_YAW_PX: f32 = 70.0; // px the view swings per rad of hull yaw
const CAM_YAW_MAX: f32 = 42.0;
// Wind heel: the press of the sails leans the boat away from the wind, hardest on
// a beam reach (most side-force) and nil dead before the wind or in irons.
const HEEL_GAIN: f32 = 0.11; // rad of lean at full sail on a hard beam reach (kept gentle)

/// The rudder demand from the helm keys: A/D (or arrows) held, [-1, 1].
/// (`SailingView.heldTurn`.) While the log is open the arrows turn its pages, so
/// only A/D steer — the helm stays live so the captain can hold a course mid-read.
fn read_turn(log_open: bool) -> f32 {
    let mut turn = 0.0;
    if is_key_down(KeyCode::D) || (!log_open && is_key_down(KeyCode::Right)) {
        turn += 1.0;
    }
    if is_key_down(KeyCode::A) || (!log_open && is_key_down(KeyCode::Left)) {
        turn -= 1.0;
    }
    turn
}

/// How a single voyage ended: the captain quit outright, or entered a new world
/// seed in the options (begin a fresh voyage on that seed).
enum GameExit {
    Quit,
    NewWorld(i64),
}

#[macroquad::main(window_conf)]
async fn main() {
    // The audio bed and the pause menu outlive any one world: the bank is costly to
    // decode (do it once), and the menu carries the options plus the world-seed
    // field the captain edits between charts. Bloom owns offscreen render targets,
    // so it persists too rather than reallocating them on every new world.
    let mut sounds = sound::SoundBank::load().await;
    let mut pause = pause_menu::PauseMenu::new();
    // Apply the saved scenery-density preference (the performance slider), if any, so
    // it persists across launches and worlds (see `save::store_feat_density`).
    if let Some(level) = save::load_feat_density() {
        pause.set_feat_density(level);
    }
    let mut bloom = bloom::Bloom::new();

    // Replace macroquad's default ProggyClean (no symbols, blurry when scaled) with
    // the DejaVu faces. Loaded once here (it uploads to the GPU); panels swap between
    // the sans and serif faces via `font::use_*`. See `font.rs`.
    font::init();

    // Continue a saved voyage if one is on disk / in localStorage: its seed picks
    // the chart and the save is handed to `run_game` to restore the captain's
    // progress. Otherwise start a fresh voyage on seed 1. Entering a new seed in
    // the options begins a fresh chart (no restore); each voyage autosaves as it
    // runs (see `run_game`), so the continued state is always current.
    let mut restore = save::Save::load();
    let mut seed: i64 = restore.as_ref().map(|s| s.seed).unwrap_or(1);
    loop {
        match run_game(seed, restore.take(), &mut sounds, &mut pause, &mut bloom).await {
            GameExit::Quit => break,
            GameExit::NewWorld(s) => seed = s,
        }
    }
}

/// Run one voyage on `seed` until the captain quits or enters a new world seed.
/// `sounds`/`pause`/`bloom` are owned by `main` and persist across worlds.
async fn run_game(
    seed: i64,
    restore: Option<save::Save>,
    sounds: &mut sound::SoundBank,
    pause: &mut pause_menu::PauseMenu,
    bloom: &mut bloom::Bloom,
) -> GameExit {
    // The day/night clock: a value in [0,1) that runs continuously (0 = midnight,
    // ¼ sunrise, ½ noon, ¾ sunset), wrapping every `DAY_LENGTH` seconds. The sky,
    // sea, sun, moon and stars are all derived from it.
    const DAY_LENGTH: f32 = 540.0;
    let mut tod: f32 = 0.40; // start mid-morning
    let mut renderer = OceanRenderer::new(tod);
    // Post-process bloom over the whole scene (sun, moon, stars, glints, sky and
    // the water's reflections). Bloom and the 4× MSAA on the scene are toggled in the
    // pause menu's Options (`pause.bloom()` / `pause.msaa()`), defaulting on natively.
    //
    // Both render the scene into an offscreen target: bloom blurs the bright parts,
    // and MSAA resolves multisamples on the scene texture. Either way miniquad issues
    // a WebGL2/MRT `drawBuffers` call for the render-to-texture pass, which is
    // unavailable in many web environments (itch's iframe / fingerprint blockers don't
    // expose `WEBGL_draw_buffers`). So on the web both are OFF and unsupported — the
    // scene draws straight to the default framebuffer (no RTT, no WebGL2 calls) — and
    // the pause menu shows them as "Not supported" there. (`bloom` is owned by
    // `main` and handed in, so it survives a change of world.)

    // Build the world and start the captain just off the home cluster's shipyard
    // port, bow pointed at it, so there's land in view from the first frame.
    let world = world::generate(seed);
    // A fixed dome of stars, seeded off the world so it's the same each run.
    let stars = celestial::StarField::new(world.seed ^ 0x5741, 260);
    // Each island's scenery is deterministic; generate it once at the current
    // scenery-density setting (the pause-menu performance slider). `islands` is in id
    // order (index == id), so this Vec aligns by index. Rebuilt in the loop whenever
    // the captain changes the density level.
    let mut feat_density_level = pause.feat_density();
    let regen_features = |level: usize| -> Vec<Vec<isle_features::IsleFeature>> {
        let d = isle_features::density_mul(level);
        world
            .islands
            .iter()
            .map(|i| isle_features::generate(world.seed, i, d))
            .collect()
    };
    let mut features = regen_features(feat_density_level);
    let home = world.cluster_at(Vec2::ZERO);
    let start_isle = home
        .island_ids
        .iter()
        .map(|&id| &world.islands[id as usize])
        .find(|i| i.is_shipyard)
        .unwrap_or(&world.islands[home.island_ids[0] as usize]);
    // Sit to the south of the isle (so the SE sun lights its face) and look north,
    // just inside dock range (radius + 250) so the captain can tie up right away.
    let start = Vec2::new(start_isle.pos.x, start_isle.pos.y - (start_isle.radius + 200.0));
    let mut kin = Kinematics::still(start, start.bearing_to(start_isle.pos));

    // The persisted voyage: gold, cargo, the hold, the hull, and where we are.
    // The captain starts at sea just off the home shipyard (the view above), with
    // a starting purse and larder, free to sail in and dock.
    let mut gs = GameState::start();
    let mut harbor = Harbor::new();
    // The pause menu (`pause`, Esc in open water) and the audio bed (`sounds`) are
    // owned by `main` and handed in so they survive a change of world.

    // The weather drifts automatically along a calm→storm ladder, biased toward
    // calm so fair seas dominate (see weather.rs). It drives the eased sea-state
    // (wave height + deck roll) and the sky gloom the storm/fury blend reads off.
    // Seeded off the world so a chart's weather is reproducible; Q/E nudge it a
    // step. `sea`/`storm` are refreshed from it at the top of every frame.
    let mut weather = WeatherState::new(Weather::Clear, world.seed ^ 0x57e4_c107);
    let mut sea: f32; // sea-state scalar (0 glassy … ~1.3 storm), refreshed each frame
    let mut storm: f32; // gale fury [0,1], refreshed each frame

    // Discrete sail setting, set once with W/S and held. Start furled (None) so the
    // captain raises sail to get under way, just like the original.
    let mut sail_mode: usize = 0;

    // Whether the captain's log is flipped open over the scene (toggled with L),
    // and which two-page spread it is turned to (paged with the arrow keys).
    let mut log_open = false;
    let mut log_spread: usize = 0;
    // Which button on the open spread the cursor is on (Up/Down move it, Enter
    // presses it); reset whenever the book opens or a page is turned.
    let mut log_sel: usize = 0;
    // The always-on corner chart's ink scheme.
    let minimap_pal = minimap::MinimapPalette::hud();

    // The prevailing wind. The opening breeze is rolled within a reach of the bow
    // (Wind::favorable) so a fresh captain never spawns in irons; from there it
    // backs/veers to a fresh random quarter every WIND_PERIOD seconds.
    let mut wind_rng = Rng::from_seed(world.seed);
    let mut wind = Wind::favorable(kin.heading_rad, &mut wind_rng);
    const WIND_PERIOD: f32 = 300.0; // seconds between auto wind shifts
    let mut clock: f32 = 0.0; // elapsed seconds, for the wind drift
    let mut last_wind_shift: f32 = 0.0; // the opening breeze holds one full period

    // The ship's foreground (deck + rig). Roll/yaw are low-passed so the deck
    // rocks with the long swell rather than buzzing with the chop.
    let mut ship = ShipRenderer::new();
    let mut smooth_roll: f32 = 0.0;
    let mut smooth_yaw: f32 = 0.0;
    let mut smooth_pitch: f32 = 0.0;
    let mut smooth_heel: f32 = 0.0;
    const ROLL_EASE: f32 = 2.2;
    const PITCH_EASE: f32 = 2.6;
    const HEEL_EASE: f32 = 1.1; // the boat leans into / out of the heel gradually

    // Bow spray: foam off the stem and shoulders, stronger with speed and bursting
    // when the bow slams into a sea. `prev_bow_lift`/`prev_lean` give the frame-to-
    // frame rates that read as a frontal / side wave impact (`SLAM_REF` = m/s of
    // bow drop that counts as a full frontal slam).
    let mut spray = Spray::new();
    let mut prev_bow_lift: f32 = 0.0;
    let mut prev_lean: f32 = 0.0;
    const SLAM_REF: f32 = 7.0;

    // The racing rival's live kinematics once it is on the water (`None` = no race
    // afoot or not yet cast off). The race runs in two stages so the rival never
    // gets a head start: while *waiting* it sits at the line and the player must
    // draw up alongside and heave to (`race_ready`); raising sail then fires the gun
    // (`race_running`) and only then does the rival begin to sail.
    let mut rival: Option<Kinematics> = None;
    let mut race_ready = false;
    let mut race_running = false;

    // --- Continue a saved voyage -------------------------------------------------
    // If `main` handed us a save (its seed already chose this chart), overwrite the
    // fresh-start defaults with the persisted progress: the purse/cargo/hull/missions/
    // booked race, the ship's position and trim, the day clock, the sail notch, and
    // the wind's quarter. The world itself was regenerated from the seed, so only
    // voyage state is restored — and this runs before the traders, flotsam and the
    // main loop read the ship's position, so they spawn around where she really is.
    if let Some(s) = restore {
        gs = s.gs;
        kin = s.kin;
        tod = s.tod;
        sail_mode = s.sail_mode.min(SAIL_FRACTIONS.len() - 1);
        wind.toward_rad = s.wind_toward;
        // The rival and race phase ride along, so a race that was already under way
        // resumes mid-course (rival where it was, still running) rather than rewinding
        // to the approach.
        rival = s.rival;
        race_ready = s.race_ready;
        race_running = s.race_running;
        // Was docked when saved: reopen the trading board so she reads as in port,
        // not parked at sea on a port's coordinates.
        if let Some(id) = gs.docked_island_id() {
            harbor.reopen_docked(id);
        } else if let Some(r) = gs.race {
            // A race was booked and she was at sea but the rival wasn't saved (an
            // older save, or a race booked but never cast off): put the rival back at
            // the line so the (paid-for) mark can still be contested — the approach
            // restarts. A rival saved on the water above is left untouched.
            if rival.is_none() {
                let target = &world.islands[r.target_id as usize];
                rival = Some(race::rival_start(&kin, target, RACE_START_GAP));
                race_ready = false;
                race_running = false;
            }
        }
    }
    // Make the on-disk save match the voyage actually being played from the first
    // frame — a fresh start, a new-seed chart, or this restored save — so a quick
    // quit can't leave a stale save from a previous seed behind.
    save::store(
        seed,
        &gs,
        &kin,
        tod,
        sail_mode,
        wind.toward_rad,
        rival,
        race_ready,
        race_running,
    );

    // Seconds since the last autosave; the voyage is persisted every
    // `AUTOSAVE_PERIOD` so closing the browser tab (which fires no quit) loses at
    // most that much progress.
    let mut autosave_timer: f32 = 0.0;
    const AUTOSAVE_PERIOD: f32 = 15.0;

    // The day/night clock at the end of the previous frame, watched for the forward
    // crossing of sunrise (¼) that ticks the "days passed" tally over. Seeded from the
    // restored/initial clock so the first comparison can't misfire.
    let mut prev_tod = tod;

    // The local cluster's wandering traders: a small fleet of merchant craft that
    // ply fixed circuits of nearby ports, tacking up to those that lie upwind and
    // lying to for a minute or so on arrival before the next leg. Only the fleet of
    // the cluster the ship is in is ever simulated; it re-spawns as the captain
    // crosses to new waters (see `trader.rs`).
    let mut traders = trader::TraderFleet::new(&world, kin.pos);

    // The mobile touch-control layer: turns finger taps/holds/drags into the same
    // verbs the keyboard emits (see `touch.rs` / `touch_ui.rs`). Dormant until a
    // real touch is seen, so desktop play is unchanged.
    let mut touch = touch::TouchState::new();

    // Floating salvage drifting on the swell: crates, barrels and the rare
    // strongbox the captain scoops by sailing over them. Per-frame and seeded off
    // the world, topped up to keep fresh salvage ahead of the bow. A pickup flashes
    // a fading toast (`salvage_flash` seconds) reading what came aboard.
    let mut flotsam = flotsam::FlotsamField::from_seed(world.seed ^ 0x5a17_f00d);
    let mut salvage_flash: f32 = 0.0;
    let mut salvage_msg = String::new();
    const SALVAGE_FLASH_TIME: f32 = 1.6;
    // A race outcome banner: when the player or the rival reaches the mark, this
    // holds a win/loss message that flashes centre-screen and fades, so the result
    // is announced on screen as well as by the win/loss sting.
    let mut race_result_flash: f32 = 0.0;
    let mut race_result_msg = String::new();
    let mut race_result_won = false;
    const RACE_RESULT_FLASH_TIME: f32 = 4.5;
    // The rival waits this far abeam at the off; the player counts as alongside
    // within `RACE_START_RANGE` and "standing still" at or below `RACE_STILL_SPEED`.
    const RACE_START_GAP: f32 = 90.0;
    const RACE_START_RANGE: f32 = 220.0;
    const RACE_STILL_SPEED: f32 = 0.6;

    loop {
        let dt = get_frame_time().min(0.05);
        let t = get_time() as f32;
        let w = screen_width();
        let h = screen_height();
        let horizon = h * 0.54;

        // Refresh the touch pointers for this frame, then lay out the sailing HUD
        // (the same rects the draw below uses). Done before any input is read.
        touch.update(dt);
        let hud = touch_ui::sail_hud(w, h);

        // Default every surface to the sans face each frame; the captain's log and the
        // port boards re-skin themselves to serif as they draw (see `font.rs`). Reset
        // here so last frame's serif board doesn't leak into this frame's HUD.
        font::use_sans();

        // --- Input -------------------------------------------------------------
        // While the pause menu is up the voyage is frozen: handle only its input
        // (Resume / Options / Quit) and skip every world-advancing update below.
        // Capture the state at frame start so a resume *this* frame still counts as
        // paused for the rest of it — otherwise the Esc that resumes would be seen
        // again by the helm's own Esc handler and reopen the menu the same frame.
        let paused = pause.open;
        if paused {
            match pause.handle_input(sounds, &touch) {
                pause_menu::PauseAction::Resume => pause.open = false,
                pause_menu::PauseAction::Quit => {
                    // Persist the voyage before leaving so quitting resumes here.
                    save::store(
                        seed,
                        &gs,
                        &kin,
                        tod,
                        sail_mode,
                        wind.toward_rad,
                        rival,
                        race_ready,
                        race_running,
                    );
                    return GameExit::Quit;
                }
                // A new seed ends this voyage; `main` loops back to build the new
                // world. The menu has already closed itself on apply.
                pause_menu::PauseAction::NewWorld(s) => return GameExit::NewWorld(s),
                pause_menu::PauseAction::None => {}
            }
        }

        // The captain may have moved the scenery-density slider in the pause menu:
        // rebuild every island's features at the new level and persist the preference.
        if pause.feat_density() != feat_density_level {
            feat_density_level = pause.feat_density();
            features = regen_features(feat_density_level);
            save::store_feat_density(feat_density_level);
        }

        // The wind backs/veers to a fresh random quarter every WIND_PERIOD seconds
        // whether sailing or docked, so the chart's breeze keeps drifting.
        if !paused {
            clock += dt;
            if clock - last_wind_shift >= WIND_PERIOD {
                wind = Wind::random(&mut wind_rng);
                last_wind_shift = clock;
                sounds.wind_shift();
            }

            // Drift the weather (whether sailing or docked) and ease the sea-state and
            // sky gloom it drives, so the waves build/lay down and the sky greys/clears
            // smoothly across a scenario change rather than snapping.
            weather.update(dt);
        }
        sea = weather.sea;
        storm = weather.fury();

        // Sail the local traders along their circuits (whether the player is at sea
        // or docked), re-spawning the fleet if the ship has crossed into new waters.
        if !paused {
            traders.update(&world, kin.pos, wind, dt);
        }

        // Advance the day/night clock (wraps at 1), then resolve the sky it implies:
        // the moving sun/moon and light, the blended sea palette and sky gradient,
        // a nearest discrete phase for the HUD/log, and how lit the deck is.
        if !paused {
            tod = (tod + dt / DAY_LENGTH).rem_euclid(1.0);
        }
        let sky = celestial::sky_state(tod);
        let sea_pal = palette::sea_palette(tod);
        let sky_grad = palette::sky_gradient(tod);
        let day = palette::daytime_at(tod);
        let day_lit = 0.5 + 0.5 * clamp(sky.sun_alt, 0.0, 1.0);

        // While docked the trading board owns input and the ship lies parked;
        // otherwise the helm and sail are live and we may dock a port in range.
        let mut helm = Helm::IDLE;
        if paused {
            // Frozen: no input, no physics. Keep the rig trimmed to the current
            // sail so the static scene still reads as a boat under way.
            helm = Helm {
                turn: 0.0,
                throttle: SAIL_FRACTIONS[sail_mode],
            };
        } else if harbor.is_open() {
            sail_mode = 0;
            kin.vel = Vec2::ZERO;
            kin.yaw_rate = 0.0;
            if let Some(id) = gs.docked_island_id() {
                let market = Market::for_island(&world.islands[id as usize], world.seed);
                let set_sail = harbor
                    .screen
                    .as_mut()
                    .map(|s| s.handle_input(&mut gs, &world, &market, sounds, &touch))
                    .unwrap_or(true);
                if set_sail {
                    harbor.set_sail(&mut gs);
                    // A booked race begins the instant the player sets sail: bring the
                    // rival up alongside now. Idempotent — a re-dock won't respawn one
                    // already out.
                    if let Some(r) = gs.race {
                        if rival.is_none() {
                            let target = &world.islands[r.target_id as usize];
                            rival = Some(race::rival_start(&kin, target, RACE_START_GAP));
                            race_ready = false;
                            race_running = false;
                        }
                    }
                }
            } else {
                harbor.set_sail(&mut gs);
            }
        } else {
            // Sails are set in discrete notches (W raises, S lowers) — set once, the
            // ship keeps going; only the *first* press of a held key steps the sail.
            // While the log is open the up/down arrows are reserved (alongside
            // left/right) for the book, so only W/S work the sail.
            let prev_sail = sail_mode;
            // The on-screen sail buttons are hidden while the log is open (the nav
            // cluster takes that corner), so their taps are gated on `!log_open`,
            // just like the arrow keys.
            if is_key_pressed(KeyCode::W)
                || (!log_open && (is_key_pressed(KeyCode::Up) || touch.tapped_in(hud.sail_up)))
            {
                sail_mode = (sail_mode + 1).min(SAIL_FRACTIONS.len() - 1);
            }
            if is_key_pressed(KeyCode::S)
                || (!log_open && (is_key_pressed(KeyCode::Down) || touch.tapped_in(hud.sail_down)))
            {
                sail_mode = sail_mode.saturating_sub(1);
            }
            // A canvas flap only when the sail actually moved a notch (not when a
            // key is pressed at the end stops). (`SailingView` flapUp/flapDown.)
            if sail_mode > prev_sail {
                sounds.sail_up();
            } else if sail_mode < prev_sail {
                sounds.sail_down();
            }
            // Steer with the keys, or with the touch wheel when a finger has it (the
            // wheel is hidden — and so ignored — while the log is open).
            let mut turn = read_turn(log_open);
            if !log_open {
                if let Some(v) = touch.steering(hud.wheel) {
                    turn = v;
                }
            }
            helm = Helm {
                turn,
                throttle: SAIL_FRACTIONS[sail_mode],
            };

            // Dev aid (not in the original): nudge the wind with [ and ] to feel the
            // points of sail and tacking on demand.
            if is_key_down(KeyCode::RightBracket) {
                wind.toward_rad = wrap_angle(wind.toward_rad + dt * 0.8);
            }
            if is_key_down(KeyCode::LeftBracket) {
                wind.toward_rad = wrap_angle(wind.toward_rad - dt * 0.8);
            }

            // Top speed scales with the rig's upgrades and the weight in the hold:
            // a stronger rig runs faster, an overladen hull crawls. The weight is the
            // whole hold — ordinary cargo *and* reserved mission goods riding along.
            let top_speed = upgrades::top_speed(gs.hull_level, gs.sail_level, gs.hold_used());
            let hull_debuff = hull::debuff(hull::fraction(&gs));
            let prev_pos = kin.pos;
            kin = sailing::step_debuffed(kin, helm, wind, dt, top_speed, hull_debuff);
            // Keep the hull out of every nearby island.
            let near = world.islands_near(kin.pos, 400.0);
            kin = sailing::resolve_grounding(kin, &near);
            // Hull decay: every kilometre sailed wears 1% off the hull, so the
            // drydock has something to mend.
            gs.wear_distance(kin.pos.distance_to(prev_pos) as f64);

            // --- Salvage -------------------------------------------------------
            // Scoop up any flotsam the ship has sailed over (gold straight to the
            // purse, with a chime + a fading toast), then keep fresh salvage drifting
            // ahead of the bow for the next stretch of open water.
            let haul = flotsam.collect_near(kin.pos, flotsam::REACH);
            if haul.gold > 0 {
                gs.gold += haul.gold;
                // Bank the salvage in the lifetime ledger (pieces + gold).
                gs.stats.flotsam_collected += haul.picked.len() as u32;
                gs.stats.flotsam_gold += haul.gold as i64;
                sounds.salvage();
                salvage_flash = SALVAGE_FLASH_TIME;
                // Name the best find so a rare strongbox feels like an event.
                let best = haul
                    .picked
                    .iter()
                    .max_by_key(|f| f.kind.gold())
                    .map(|f| f.kind.label())
                    .unwrap_or("Salvage");
                salvage_msg = if haul.picked.len() > 1 {
                    format!("Salvage! +{} gold  ({} & more)", haul.gold, best)
                } else {
                    format!("{}! +{} gold", best, haul.gold)
                };
            }
            flotsam.replenish(kin.pos, kin.heading_rad, &world);

            // --- Race ----------------------------------------------------------
            // While waiting the rival sits dead at the line; the start is armed once
            // the player draws up alongside and heaves to (sails struck, dead slow),
            // and the gun fires the moment they raise sail. Once running the rival
            // sails the very same physics — a pristine rig of the player's own boat
            // (its sail level, an empty hold) — beating for the mark but never into
            // the wind's eye, and the race settles the instant either reaches it.
            if let Some(r) = gs.race {
                let target = &world.islands[r.target_id as usize];
                match rival {
                    Some(rk) if race_running => {
                        // The rival sails a hull of the race's required tier (empty
                        // hold), so a higher-tier leg fields a genuinely faster boat.
                        let top_speed = upgrades::top_speed(r.required_level, 0, 0);
                        let rhelm = race::rival_helm(&rk, target.pos, wind);
                        let stepped = sailing::step_with(rk, rhelm, wind, dt, top_speed);
                        let rnear = world.islands_near(stepped.pos, 400.0);
                        rival = Some(sailing::resolve_grounding(stepped, &rnear));
                        if race::reached(&kin, target) {
                            let payout = r.stake * 2;
                            gs.win_race();
                            rival = None;
                            race_ready = false;
                            race_running = false;
                            sounds.race_won();
                            race_result_won = true;
                            race_result_msg =
                                format!("Race won — first to {}!  Purse: {} gold", target.name, payout);
                            race_result_flash = RACE_RESULT_FLASH_TIME;
                        } else if rival.is_some_and(|rk| race::reached(&rk, target)) {
                            let lost = r.stake;
                            gs.lose_race();
                            rival = None;
                            race_ready = false;
                            race_running = false;
                            sounds.race_lost();
                            race_result_won = false;
                            race_result_msg =
                                format!("Race lost — rival reached {} first.  {} gold forfeit", target.name, lost);
                            race_result_flash = RACE_RESULT_FLASH_TIME;
                        }
                    }
                    Some(rk) => {
                        let alongside = kin.pos.distance_to(rk.pos) <= RACE_START_RANGE;
                        if !race_ready {
                            if alongside && sail_mode == 0 && kin.speed() <= RACE_STILL_SPEED {
                                race_ready = true;
                            }
                        } else if !alongside {
                            race_ready = false;
                        }
                        if race_ready && sail_mode > 0 {
                            race_running = true;
                        }
                    }
                    None => {}
                }
            } else if rival.is_some() {
                // Race cleared (e.g. withdrawn at a port mid-voyage): retire the rival.
                rival = None;
                race_ready = false;
                race_running = false;
            }

            // Offer the port the bow is pointed at; tie up on Space, sails struck.
            harbor.update_dockable(&world, &kin);
            if (is_key_pressed(KeyCode::Space) || touch.tapped_in(hud.dock))
                && sail_mode == 0
                && harbor.try_dock(&mut gs)
            {
                log_open = false;
            }

            // Dev aid: nudge the weather a step calmer (Q) / stormier (E); it keeps
            // auto-drifting from there. The sea/sky then ease to the new scenario.
            if is_key_pressed(KeyCode::Q) {
                weather.nudge_calmer();
            }
            if is_key_pressed(KeyCode::E) {
                weather.nudge_stormier();
            }
            // Dev aid: stave in 10% of a full hull, to feel the damage debuffs
            // (no-go zone / turn / top speed) and the drydock without sailing it off.
            if is_key_pressed(KeyCode::X) {
                let blow = (gs.max_hull() as f64 * 0.10).ceil() as i32;
                gs.hull = (gs.hull - blow).max(0);
                gs.hull_wear = 0.0;
            }
            // Nudge the clock forward (T) / back (Y) ~30 min, to ease through the cycle.
            if is_key_pressed(KeyCode::T) {
                tod = (tod + 0.02).rem_euclid(1.0);
            }
            if is_key_pressed(KeyCode::Y) {
                tod = (tod - 0.02).rem_euclid(1.0);
            }
            if is_key_pressed(KeyCode::L) || touch.tapped_in(hud.log) {
                log_open = !log_open;
                // Open the book to its first spread each time (the original rewinds
                // to spread 0 on close).
                if log_open {
                    log_spread = 0;
                    log_sel = 0;
                }
            }
            // Page the open log with the left/right arrows (no mouse to click the
            // original's nav arrows), or the on-screen nav cluster on touch. Clamped
            // at the covers — no wrap-around.
            if log_open {
                let n = touch_ui::nav_cluster(w, h);
                // The nav cluster's back button closes the book on touch.
                if touch.tapped_in(n.back) {
                    log_open = false;
                }
                if is_key_pressed(KeyCode::Right) || touch.tapped_in(n.right) {
                    log_spread = (log_spread + 1).min(captains_log::NUM_SPREADS - 1);
                    log_sel = 0;
                }
                if is_key_pressed(KeyCode::Left) || touch.tapped_in(n.left) {
                    log_spread = log_spread.saturating_sub(1);
                    log_sel = 0;
                }
                // Up/Down move the selection cursor among the spread's buttons (left/
                // right turn the page), and Enter presses the focused one — the same
                // arrows-then-Enter flow as the port board. The Vessel spread's button
                // caulks the hull with a plank; a no-op without timber or on a sound hull.
                let buttons = captains_log::button_count(log_spread);
                if buttons > 0 {
                    if is_key_pressed(KeyCode::Up) || touch.tapped_in(n.up) {
                        log_sel = log_sel.saturating_sub(1);
                    }
                    if is_key_pressed(KeyCode::Down) || touch.tapped_in(n.down) {
                        log_sel = (log_sel + 1).min(buttons - 1);
                    }
                    if (is_key_pressed(KeyCode::Enter) || touch.tapped_in(n.confirm))
                        && log_spread == 1
                        && log_sel == 0
                    {
                        let _ = gs.caulk_with_plank();
                    }
                }
            }
            // The pause button raises the menu while sailing; it's hidden while the
            // log is open (the book is closed with Esc / the cluster's back instead).
            let pause_tap = !log_open && touch.tapped_in(hud.pause);
            if is_key_pressed(KeyCode::Escape) || pause_tap {
                if log_open {
                    log_open = false;
                } else {
                    // No other menu up: heave to and raise the pause menu.
                    pause.open();
                }
            }
        }
        // Ride the ambient beds to match the boat's speed and the gale's fury.
        sounds.update(dt, harbor.is_open(), kin.speed() / sailing::KNOT, storm);

        // --- Ship motion + camera ride -----------------------------------------
        // Sample how the swell throws the hull this frame, then ease the parts that
        // should rock with the long swell rather than buzz with the chop.
        let motion = ocean::ship_motion(kin.pos, kin.heading_rad, t, sea);
        // The bow's lift above the hull's mean (metres): the sea height where the bow
        // parts the water, relative to the helm's heave. Drives the deck/camera heave
        // bob, split between the two by `ocean::HEAVE_CAMERA_SHARE`.
        let bow_z = ocean::height(
            kin.pos + Vec2::from_heading(kin.heading_rad) * ocean::BOW_REACH,
            t,
            sea,
        );
        let bow_lift = bow_z - motion.heave;
        // Frontal slam: how fast the bow is dropping into the sea this frame (m/s of
        // downward bow travel), normalised — a hard plunge into a wave face bursts
        // the spray. Only the downward half counts (the bow climbing throws nothing).
        let dt_safe = dt.max(1.0 / 240.0);
        let bow_vel = (bow_lift - prev_bow_lift) / dt_safe;
        prev_bow_lift = bow_lift;
        let slam = clamp(-bow_vel / SLAM_REF, 0.0, 1.0);
        let wind_rel = wrap_angle(wind.toward_rad - kin.heading_rad);
        // Wind heel: the sails' press leans the boat away from the wind, hardest on
        // a beam reach (most side-force) and nil dead before it or in irons. The sign
        // of sin(wind_rel) picks the side — wind blowing to starboard heels her
        // starboard (positive roll) — and the drive curve scales it by the press.
        let heel_target =
            HEEL_GAIN * helm.throttle * sailing::wind_factor_rel(wind_rel) * wind_rel.sin();
        let ease = clamp(ROLL_EASE * dt, 0.0, 1.0);
        smooth_roll += (motion.roll - smooth_roll) * ease;
        smooth_yaw += (motion.yaw - smooth_yaw) * ease;
        smooth_pitch += (motion.pitch - smooth_pitch) * clamp(PITCH_EASE * dt, 0.0, 1.0);
        smooth_heel += (heel_target - smooth_heel) * clamp(HEEL_EASE * dt, 0.0, 1.0);

        // The hull's lean (swell roll + wind heel) tilts the horizon the *other* way
        // — the sea stays level while the captain leans into it — and the bow's pitch
        // and yaw nod and swing the view. This is the camera "ride" the whole world
        // is drawn through, so sky, sun, waves and islands rock together as one.
        let lean = smooth_roll + smooth_heel;
        // Side slam: how fast she is rolling this frame (rad/s, signed) — a hard,
        // quick roll into a sea throws spray off the lee shoulder.
        let heel_rate = (lean - prev_lean) / dt_safe;
        prev_lean = lean;
        let cam_roll_deg = clamp(
            -lean.to_degrees() * CAM_ROLL_GAIN,
            -CAM_ROLL_MAX_DEG,
            CAM_ROLL_MAX_DEG,
        );
        // Camera pitch look + the camera's share of the bow's heave bob. The shaped
        // pitch glances up the wave face and down the back (with extra gain diving);
        // the bob's remaining share cranes the horizon so the near water and the deck's
        // matched bob travel together. The deck (drawn in screen space) carries the rest.
        let shaped_pitch = ocean::pitch_response(smooth_pitch);
        let dive_depth = clamp(-smooth_pitch / ocean::PITCH_DIVE_KNEE, 0.0, 1.0);
        let look_shift =
            shaped_pitch * PITCH_LOOK_GAIN * (1.0 + (CAMERA_DIVE_EXTRA - 1.0) * dive_depth);
        let cam_vert = clamp(
            look_shift - ocean::deck_heave_px(bow_lift) * ocean::HEAVE_CAMERA_SHARE,
            -CAM_PITCH_MAX,
            CAM_PITCH_MAX,
        );
        let cam_sway = clamp(CAM_YAW_PX * smooth_yaw, -CAM_YAW_MAX, CAM_YAW_MAX);

        // --- Scene (drawn through the camera ride) -----------------------------
        let px_per_rad = h * 0.85;
        let half_fov_h_view = projection::MAX_HALF_FOV_H.min((w * 0.5) / px_per_rad);

        // The tilted/translated view must never reveal background past the painted
        // sea and sky, so everything world-anchored is over-scanned past the screen
        // edges by this much (sized to cover the clamped roll + translate).
        let overscan = w.max(h) * 0.25 + 60.0;
        // Graphics settings for this frame (always off on the web — see the pause menu).
        let bloom_on = pause.bloom();
        let msaa_on = pause.msaa();
        // Natively the world is always rendered into an offscreen scene target, so the
        // MSAA setting governs the wave AA directly (a plain, single-sampled target
        // means MSAA-off truly aliases, rather than the window's default-framebuffer
        // MSAA leaking in). Bloom then reads that target, or it's blitted straight over.
        // On the web there's no render-to-texture, so the world draws to the screen.
        #[cfg(target_arch = "wasm32")]
        let to_target = false;
        #[cfg(not(target_arch = "wasm32"))]
        let to_target = true;
        let mut world_cam = Camera2D::from_display_rect(Rect::new(0.0, 0.0, w, h));
        if to_target {
            // Render the world into the scene texture (MSAA-resolved when enabled).
            // Rendering to a target flips `invert_y`, which cancels the screen flip
            // below — so leave `zoom.y` as `from_display_rect` set it and the net matrix
            // matches the screen path. The bloom composite / plain blit flips it back.
            world_cam.render_target = Some(bloom.scene_target(w, h, msaa_on));
        } else {
            // `from_display_rect` builds its zoom for render-to-texture; drawn straight
            // to the screen it comes out vertically flipped. Flip `zoom.y` back so the
            // world is upright and matches the screen-space ship/HUD drawn after.
            world_cam.zoom.y = -world_cam.zoom.y;
        }
        world_cam.rotation = cam_roll_deg;
        world_cam.target = vec2(w * 0.5 + cam_sway, h * 0.5 - cam_vert);
        set_camera(&world_cam);
        clear_background(BLACK);

        // Look astern: hold C to spin the *view* 180° (the physics/helm are untouched)
        // so the captain can glance back over the wake. Only the projected world turns;
        // a view-only copy of the kinematics carries the flipped heading into every
        // bearing-relative draw below, and the forward deck/spray are hidden while it's
        // held. Suppressed when a board or the log owns the screen.
        let look_back = (is_key_down(KeyCode::C) || touch.held_in(hud.astern))
            && !log_open
            && !harbor.is_open()
            && !pause.open;
        let view_heading = if look_back {
            wrap_angle(kin.heading_rad + std::f32::consts::PI)
        } else {
            kin.heading_rad
        };
        let mut view_kin = kin;
        view_kin.heading_rad = view_heading;

        let sky_view = scene::SkyView {
            heading: view_heading,
            half_fov_h: half_fov_h_view,
            w,
            horizon,
        };
        draw_sky(&sky_view, sky_grad, storm, overscan, sky.sun_az, sky.sun_alt);
        // Stars, then the moon and sun arcing over on the clock.
        celestial::draw(&sky, &stars, t, &sky_view, h, storm);

        // Distant-water backdrop behind the wave mesh, over-scanned so a rolled view
        // still finds deep sea in the corners.
        let far = lerp3(
            (sea_pal[6], sea_pal[7], sea_pal[8]),
            (palette::STORM_PALETTE[6], palette::STORM_PALETTE[7], palette::STORM_PALETTE[8]),
            clamp(storm, 0.0, 1.0) * 0.9,
        );
        draw_rectangle(-overscan, horizon, w + 2.0 * overscan, h - horizon + overscan, rgb(far));

        // --- Islands (drawn interleaved with the waves) ------------------------
        // Visible isles: in front of the camera and within view, sorted farthest-
        // first by near-shore distance so the wave renderer can slot each in at its
        // own depth.
        let key = |i: &Island| kin.pos.distance_to(i.pos) - i.radius;
        let mut visible: Vec<(&Island, &[isle_features::IsleFeature])> = world
            .islands
            .iter()
            .filter(|i| {
                let d = kin.pos.distance_to(i.pos);
                let rel = wrap_angle(kin.pos.bearing_to(i.pos) - view_heading);
                // Allow for the isle's angular half-width: close in, its near shore
                // fills the view even when the *centre* lies well off the bow, so
                // the off-axis limit must grow as `asin(radius/d)`. Otherwise a big
                // isle sailed alongside pops out the moment its centre clears the FOV.
                let ang_r = (i.radius / d).min(1.0).asin();
                d <= MAX_VIEW && d >= i.radius && rel.abs() <= half_fov_h_view * 1.6 + ang_r
            })
            .map(|i| (i, features[i.id as usize].as_slice()))
            .collect();
        visible.sort_by(|a, b| key(b.0).partial_cmp(&key(a.0)).unwrap());

        // Visible salvage: in view and within range, sorted farthest-first like the
        // isles so the wave renderer can slot each piece in at its own depth.
        let mut flot_vis: Vec<(Vec2, flotsam::FlotsamKind)> = flotsam
            .items
            .iter()
            .filter(|f| {
                let d = kin.pos.distance_to(f.pos);
                let rel = wrap_angle(kin.pos.bearing_to(f.pos) - view_heading);
                d <= MAX_VIEW && rel.abs() <= half_fov_h_view * 1.3
            })
            .map(|f| (f.pos, f.kind))
            .collect();
        flot_vis.sort_by(|a, b| {
            kin.pos
                .distance_to(b.0)
                .partial_cmp(&kin.pos.distance_to(a.0))
                .unwrap()
        });

        // The local traders, sorted farthest-first so the wave march can slot each
        // into its own depth (nearer crests and islands then paint over it).
        let mut trader_kins = traders.kinematics();
        trader_kins.sort_by(|a, b| {
            kin.pos
                .distance_to(b.pos)
                .partial_cmp(&kin.pos.distance_to(a.pos))
                .unwrap()
        });

        // The sky the water mirrors (Fresnel) must match the *displayed* sky, which
        // darkens to night as the sun sets (draw_sky's `base_night`). Ease the
        // reflected gradient down the same way, else warm reflections linger on the
        // waves long after the painted sky has gone dark.
        let base_night = palette::night_factor(sky.sun_alt);
        let night_sky = palette::fair_sky(palette::Daytime::Night);
        let reflect_sky = [
            lerp3(sky_grad[0], night_sky[0], base_night),
            lerp3(sky_grad[1], night_sky[1], base_night),
            lerp3(sky_grad[2], night_sky[2], base_night),
        ];

        // --- Waves -------------------------------------------------------------
        renderer.render(
            &view_kin,
            t,
            sea,
            motion.heave,
            &sea_pal,
            reflect_sky,
            sky.light_az,
            sky.light_alt,
            sky.light_strength,
            storm,
            w,
            h,
            &visible,
            // The racing rival rides the same sea, depth-sorted into the wave march
            // so nearer crests and islands occlude it like any other world object.
            rival,
            day_lit,
            &flot_vis,
            &trader_kins,
        );

        // Back to screen space for the foreground + HUD, which stay bolted to the
        // viewport rather than riding the swell. If the world went to the scene target,
        // bring it to the screen now: bloom extracts/blurs the bright parts and
        // composites, otherwise (MSAA-only) the resolved scene is blitted straight over.
        if to_target {
            if bloom_on {
                bloom.render_to_screen(w, h);
            } else {
                bloom.blit_scene_to_screen(w, h);
            }
        } else {
            set_default_camera();
        }

        // --- Ship (deck + rig) -------------------------------------------------
        // The deck leans with the same swell roll *and* wind heel, so she visibly
        // heels under sail while the horizon tilts the other way; pitch/heave are
        // read straight off the swell.
        let rig = RigInput {
            motion: ocean::ShipMotion {
                roll: smooth_roll + smooth_heel,
                yaw: smooth_yaw,
                pitch: smooth_pitch,
                ..motion
            },
            set: helm.throttle,
            turn: helm.turn,
            wind_rel,
            bow_lift,
        };
        // --- Bow spray ---------------------------------------------------------
        // Foam torn off the bow, drawn *before* the deck so the hull occludes the
        // droplets behind it — only the spray rising above and outboard of the bow
        // shows. Strengthens with speed (the standing bow wave) and bursts on a
        // frontal slam or a hard roll into a sea.
        // The forward deck, rig and bow spray are the captain's-eye foreground looking
        // ahead; hide them while glancing astern so the wake and following sea read
        // clear over open water.
        if !look_back {
            spray.render(
                &SprayInput {
                    speed_frac: clamp(kin.speed() / sailing::BASE_TOP_SPEED, 0.0, 1.0),
                    slam,
                    heel_rate,
                    day_lit,
                },
                dt,
                w,
                h,
            );

            ship.render(&rig, dt, t, day_lit, storm, w, h);
        }

        // --- HUD ---------------------------------------------------------------
        // Pared back to the essentials: the purse, the speed, the wind's quarter
        // and the point of sail — plus a warning badge for any handling debuff.
        // Wind is shown by the quarter it blows *from* (the seaman's convention).
        let knots = kin.speed() / sailing::KNOT;
        let wind_from = compass(wrap_angle(wind.toward_rad + std::f32::consts::PI));
        let point = wind.point_of_sail(kin.heading_rad).label();
        // Everything in one row, at one font size, dot-separated: a coin icon and
        // the purse, then speed · wind quarter · point of sail.
        let fs = px(16.0);
        let baseline = px(26.0);
        // Coin icon, vertically centred on the text's cap height.
        let r = px(7.0);
        let cx = px(16.0) + r;
        let cy = baseline - fs * 0.34;
        let rim = Color::new(0.78, 0.58, 0.12, 1.0); // darker milled edge
        let face = Color::new(1.0, 0.84, 0.32, 1.0); // bright gold face
        let shine = Color::new(1.0, 0.97, 0.78, 1.0); // glint
        draw_circle(cx, cy, r, rim);
        draw_circle(cx, cy, r * 0.82, face);
        draw_circle_lines(cx, cy, r * 0.82, px(1.0), rim);
        draw_circle(cx - r * 0.3, cy - r * 0.3, r * 0.2, shine);
        // The rest of the row, starting just right of the coin.
        let line = format!(
            "{}  ·  {:.1} kn  ·  Wind {}  ({})",
            gs.gold, knots, wind_from, point
        );
        draw_text(&line, px(16.0) + 2.0 * r + px(8.0), baseline, fs, WHITE);

        // Active-debuff badges: a warning triangle (and a word) for a battered
        // hull and/or an overladen hold — the handling penalties in force.
        {
            let mut badges: Vec<&str> = Vec::new();
            if hull::fraction(&gs) <= 0.90 {
                badges.push("Hull");
            }
            if upgrades::overload_penalty(gs.sail_level, gs.hold_used()) > 0.0 {
                badges.push("Overladen");
            }
            let warn = Color::new(1.0, 0.78, 0.2, 1.0);
            let mut x = px(16.0);
            let y = px(56.0);
            let s = px(13.0); // triangle size
            for label in badges {
                draw_triangle(
                    vec2(x + s * 0.5, y - s),
                    vec2(x, y),
                    vec2(x + s, y),
                    warn,
                );
                draw_text("!", x + s * 0.5 - px(2.0), y - px(2.0), px(14.0), Color::new(0.1, 0.05, 0.0, 1.0));
                let lx = x + s + px(6.0);
                draw_text(label, lx, y, px(15.0), warn);
                x = lx + measure_text(label, None, px(15.0) as u16, 1.0).width + px(18.0);
            }
        }

        // Salvage pickup toast: a gold note that floats up and fades over the deck
        // when a piece is hauled aboard.
        salvage_flash = (salvage_flash - dt).max(0.0);
        if salvage_flash > 0.0 && !harbor.is_open() && !log_open {
            let p = salvage_flash / SALVAGE_FLASH_TIME; // 1 → 0 as it fades
            let fs = px(30.0) as u16;
            let dims = measure_text(&salvage_msg, None, fs, 1.0);
            let tx = w * 0.5 - dims.width / 2.0;
            let ty = h * 0.34 - (1.0 - p) * px(36.0); // drifts upward as it fades
            draw_text(
                &salvage_msg,
                tx + px(1.0),
                ty + px(1.0),
                fs as f32,
                Color::new(0.0, 0.0, 0.0, 0.5 * p),
            );
            draw_text(&salvage_msg, tx, ty, fs as f32, Color::new(1.0, 0.9, 0.5, p));
        }

        // The destinations marked on the charts: every accepted contract (yellow
        // "M"), and separately the booked race's mark (red "R").
        let chart_marks: Vec<i32> = gs.active_missions.iter().map(|m| m.target_id).collect();
        let race_marks: Vec<i32> = gs.race.iter().map(|r| r.target_id).collect();

        // Always-on corner chart: the local cluster, top-right.
        let map_size = (h * 0.24).clamp(px(140.0), px(200.0));
        let map_rect = Rect::new(w - map_size - px(16.0), px(16.0), map_size, map_size);
        minimap::render(
            &world,
            &kin,
            wind,
            map_rect,
            &minimap_pal,
            &chart_marks,
            &race_marks,
            None,
            &traders.positions(),
            rival.map(|r| (r.pos, r.heading_rad)),
        );

        // Race standings strip: the mark and how far the player and rival each have
        // still to sail (or the instructions to get the race under way), shown top-
        // centre while a rival is on the water and the helm is live.
        if !harbor.is_open() && !log_open {
            if let (Some(rk), Some(r)) = (rival, gs.race) {
                let target = &world.islands[r.target_id as usize];
                let text = if race_running {
                    let you = kin.pos.distance_to(target.pos);
                    let them = rk.pos.distance_to(target.pos);
                    let lead = if you <= them { "you lead" } else { "rival leads" };
                    format!(
                        "RACE -> {}    you {}    rival {}    ({})",
                        target.name,
                        format_dist(you),
                        format_dist(them),
                        lead,
                    )
                } else if race_ready {
                    "Alongside the rival — raise sail to start the race!".to_string()
                } else {
                    format!(
                        "Heave to alongside the rival to start  ·  {} away",
                        format_dist(kin.pos.distance_to(rk.pos)),
                    )
                };
                let fs = px(24.0) as u16;
                let dims = measure_text(&text, None, fs, 1.0);
                let bx = w * 0.5 - dims.width / 2.0;
                let by = h * 0.14;
                draw_rectangle(
                    bx - px(16.0),
                    by - px(26.0),
                    dims.width + px(32.0),
                    px(38.0),
                    Color::new(0.10, 0.06, 0.02, 0.6),
                );
                draw_text(&text, bx, by, fs as f32, Color::new(1.0, 0.92, 0.6, 1.0));
            }
        }

        // Race outcome banner: a bold win/loss announcement centre-screen that holds
        // then fades, so the player learns the result (and the gold) — not just the
        // sting. Green-gold for a win, dull red for a loss.
        race_result_flash = (race_result_flash - dt).max(0.0);
        if race_result_flash > 0.0 && !harbor.is_open() && !log_open {
            // Hold full opacity for most of the window, fading over the last second.
            let a = (race_result_flash / 1.0).min(1.0);
            let fs = px(40.0) as u16;
            let dims = measure_text(&race_result_msg, None, fs, 1.0);
            let bx = w * 0.5 - dims.width / 2.0;
            let by = h * 0.30;
            draw_rectangle(
                bx - px(22.0),
                by - px(38.0),
                dims.width + px(44.0),
                px(56.0),
                Color::new(0.08, 0.05, 0.02, 0.66 * a),
            );
            let accent = if race_result_won {
                Color::new(1.0, 0.86, 0.42, a) // gold for a win
            } else {
                Color::new(0.95, 0.45, 0.38, a) // dull red for a loss
            };
            draw_text(&race_result_msg, bx + px(2.0), by + px(2.0), fs as f32, Color::new(0.0, 0.0, 0.0, 0.5 * a));
            draw_text(&race_result_msg, bx, by, fs as f32, accent);
        }

        // The captain's log, flipped open over the scene.
        if log_open {
            captains_log::render(
                &world,
                &gs,
                &kin,
                wind,
                SAIL_NAMES[sail_mode],
                day,
                weather.weather.label(),
                &chart_marks,
                &race_marks,
                log_spread,
                log_sel,
                dt,
                w,
                h,
            );
        }

        // The docking call-to-action (while sailing), then the port board (docked).
        // Suppress it at a race's finish line so a novice doesn't strike sail short.
        let race_target = gs.race.map(|r| r.target_id);
        port_view::render_prompt(&harbor, &world, sail_mode == 0, race_target, w, h);
        if harbor.is_open() {
            if let Some(id) = gs.docked_island_id() {
                let market = Market::for_island(&world.islands[id as usize], world.seed);
                if let Some(screen) = harbor.screen.as_ref() {
                    screen.render(&gs, &world, &market, &kin, wind, w, h);
                }
            }
        }

        // The pause menu sits over everything (it only opens in open water).
        if pause.open {
            pause.render(sounds, w, h);
        }

        // --- Touch controls overlay --------------------------------------------
        // Drawn last so it sits over every surface, and only once the touch layer
        // has woken (a real touch, or SAIL_TOUCH on native) — so desktop play is
        // untouched. A menu shows the nav cluster (the board adds a Tab button);
        // open water shows the sailing helm.
        if touch.active() {
            if pause.open || log_open || harbor.is_open() {
                // The board can be tapped directly (tabs / rows / chips), but it also
                // gets the full d-pad + ✓/✕ cluster, like the pause menu and log, for
                // captains who'd rather step the cursor than tap precisely.
                touch_ui::draw_nav_cluster(&touch_ui::nav_cluster(w, h));
            } else {
                touch_ui::draw_sail_hud(
                    &hud,
                    helm.turn,
                    sail_mode,
                    SAIL_FRACTIONS.len() - 1,
                    harbor.dockable.is_some(),
                    look_back,
                );
            }
        }

        // A new day breaks at sunrise (the clock crossing ¼ going forward). The clock
        // only advances while live and moves a sliver per frame, so a plain threshold
        // crossing catches it without tripping on the midnight wrap. Banked into the
        // lifetime tally; `prev_tod` trails the clock for the next frame's test.
        if !paused && prev_tod < 0.25 && tod >= 0.25 {
            gs.stats.days_passed += 1;
        }
        prev_tod = tod;

        // Periodic autosave: while the world is live (not frozen by the pause menu),
        // persist the now-updated voyage every few seconds so an unexpected exit —
        // closing the browser tab, a crash — resumes within seconds of here.
        if !paused {
            autosave_timer += dt;
            if autosave_timer >= AUTOSAVE_PERIOD {
                autosave_timer = 0.0;
                save::store(
                    seed,
                    &gs,
                    &kin,
                    tod,
                    sail_mode,
                    wind.toward_rad,
                    rival,
                    race_ready,
                    race_running,
                );
            }
        }

        next_frame().await
    }
}
