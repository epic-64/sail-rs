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
mod game_state;
mod geometry;
mod isle_features;
mod islands_render;
mod minimap;
mod mission;
mod ocean;
mod ocean_renderer;
mod palette;
mod port_view;
mod projection;
mod race;
mod rival_render;
mod rng;
mod sailing;
mod ship_render;
mod sound;
mod trader;
mod weather;
mod world;

use macroquad::prelude::*;

use game_state::{upgrades, GameState, Market};
use geometry::{clamp, wrap_angle, Vec2};
use port_view::Harbor;
use ocean_renderer::OceanRenderer;
use projection::MAX_VIEW;
use rng::Rng;
use sailing::{Helm, Kinematics, Wind};
use ship_render::{RigInput, ShipRenderer};
use weather::{Weather, WeatherState};
use world::Island;

fn window_conf() -> Conf {
    Conf {
        window_title: "sail-rs".to_owned(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
        sample_count: 4, // MSAA: smooth the wave-quad edges
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

/// Paint the sky as a vertical three-stop gradient (top → mid → horizon), eased
/// toward the storm overcast by `storm`. `sky` is the clock's fair-weather gradient.
/// Drawn as horizontal strips since macroquad has no built-in gradient.
fn draw_sky(sky: [(f32, f32, f32); 3], storm: f32, w: f32, horizon: f32, m: f32) {
    let top = lerp3(sky[0], palette::STORM_SKY[0], storm);
    let mid = lerp3(sky[1], palette::STORM_SKY[1], storm);
    let hor = lerp3(sky[2], palette::STORM_SKY[2], storm);

    // Over-scan base: paint the top sky colour across the whole region above the
    // horizon (and out past every edge) so the camera ride's tilt/translate never
    // reveals the cleared background above or beside the gradient.
    draw_rectangle(-m, -m, w + 2.0 * m, horizon + m, rgb(top));

    let strips = 96;
    let strip_h = horizon / strips as f32;
    for i in 0..strips {
        let t = i as f32 / (strips - 1) as f32;
        let c = if t < 0.5 {
            lerp3(top, mid, t * 2.0)
        } else {
            lerp3(mid, hor, (t - 0.5) * 2.0)
        };
        let y = i as f32 * strip_h;
        // +1px overlap so no seams show between strips; over-scanned sideways.
        draw_rectangle(-m, y, w + 2.0 * m, strip_h + 1.0, rgb(c));
    }
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

/// A short distance readout for the race standings: kilometres past 1 km, metres
/// below it. (`SailingView.formatDist`.)
fn format_dist(m: f32) -> String {
    if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{} m", m.round() as i32)
    }
}

/// 8-point compass label for a bearing (radians, 0 = N, CW).
fn compass(bearing_rad: f32) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = (bearing_rad.to_degrees().rem_euclid(360.0) + 22.5) / 45.0;
    POINTS[(deg as usize) % 8]
}

#[macroquad::main(window_conf)]
async fn main() {
    // The day/night clock: a value in [0,1) that runs continuously (0 = midnight,
    // ¼ sunrise, ½ noon, ¾ sunset), wrapping every `DAY_LENGTH` seconds. The sky,
    // sea, sun, moon and stars are all derived from it.
    const DAY_LENGTH: f32 = 540.0;
    let mut tod: f32 = 0.40; // start mid-morning
    let mut renderer = OceanRenderer::new(tod);
    // Post-process bloom over the whole scene (sun, moon, stars, glints, sky and
    // the water's reflections). Toggle with B.
    let mut bloom = bloom::Bloom::new();
    let mut bloom_on = true;

    // Build the world and start the captain just off the home cluster's shipyard
    // port, bow pointed at it, so there's land in view from the first frame.
    let world = world::generate(1);
    // A fixed dome of stars, seeded off the world so it's the same each run.
    let stars = celestial::StarField::new(world.seed ^ 0x5741, 170);
    // Each island's scenery is deterministic; generate it once. `islands` is in id
    // order (index == id), so this Vec aligns by index.
    let features: Vec<Vec<isle_features::IsleFeature>> = world
        .islands
        .iter()
        .map(|i| isle_features::generate(world.seed, i))
        .collect();
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

    // The audio bed: three ambient loops (sailing/calm/storm) cross-faded by sea
    // state, plus one-shot cues for wind shifts and trades. Loaded once up front.
    let mut sounds = sound::SoundBank::load().await;

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

    // The racing rival's live kinematics once it is on the water (`None` = no race
    // afoot or not yet cast off). The race runs in two stages so the rival never
    // gets a head start: while *waiting* it sits at the line and the player must
    // draw up alongside and heave to (`race_ready`); raising sail then fires the gun
    // (`race_running`) and only then does the rival begin to sail.
    let mut rival: Option<Kinematics> = None;
    let mut race_ready = false;
    let mut race_running = false;

    // The local cluster's wandering traders: a small fleet of merchant craft that
    // ply fixed circuits of nearby ports, tacking up to those that lie upwind and
    // lying to for a minute or so on arrival before the next leg. Only the fleet of
    // the cluster the ship is in is ever simulated; it re-spawns as the captain
    // crosses to new waters (see `trader.rs`).
    let mut traders = trader::TraderFleet::new(&world, kin.pos);

    // Floating salvage drifting on the swell: crates, barrels and the rare
    // strongbox the captain scoops by sailing over them. Per-frame and seeded off
    // the world, topped up to keep fresh salvage ahead of the bow. A pickup flashes
    // a fading toast (`salvage_flash` seconds) reading what came aboard.
    let mut flotsam = flotsam::FlotsamField::from_seed(world.seed ^ 0x5a17_f00d);
    let mut salvage_flash: f32 = 0.0;
    let mut salvage_msg = String::new();
    const SALVAGE_FLASH_TIME: f32 = 1.6;
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

        // --- Input -------------------------------------------------------------
        // The wind backs/veers to a fresh random quarter every WIND_PERIOD seconds
        // whether sailing or docked, so the chart's breeze keeps drifting.
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
        sea = weather.sea;
        storm = weather.fury();

        // Sail the local traders along their circuits (whether the player is at sea
        // or docked), re-spawning the fleet if the ship has crossed into new waters.
        traders.update(&world, kin.pos, wind, dt);

        // Advance the day/night clock (wraps at 1), then resolve the sky it implies:
        // the moving sun/moon and light, the blended sea palette and sky gradient,
        // a nearest discrete phase for the HUD/log, and how lit the deck is.
        tod = (tod + dt / DAY_LENGTH).rem_euclid(1.0);
        let sky = celestial::sky_state(tod);
        let sea_pal = palette::sea_palette(tod);
        let sky_grad = palette::sky_gradient(tod);
        let day = palette::daytime_at(tod);
        let day_lit = 0.5 + 0.5 * clamp(sky.sun_alt, 0.0, 1.0);

        // While docked the trading board owns input and the ship lies parked;
        // otherwise the helm and sail are live and we may dock a port in range.
        let mut helm = Helm::IDLE;
        if harbor.is_open() {
            sail_mode = 0;
            kin.vel = Vec2::ZERO;
            kin.yaw_rate = 0.0;
            if let Some(id) = gs.docked_island_id() {
                let market = Market::for_island(&world.islands[id as usize], world.seed);
                let set_sail = harbor
                    .screen
                    .as_mut()
                    .map(|s| s.handle_input(&mut gs, &world, &market, &sounds))
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
            if is_key_pressed(KeyCode::W) || (!log_open && is_key_pressed(KeyCode::Up)) {
                sail_mode = (sail_mode + 1).min(SAIL_FRACTIONS.len() - 1);
            }
            if is_key_pressed(KeyCode::S) || (!log_open && is_key_pressed(KeyCode::Down)) {
                sail_mode = sail_mode.saturating_sub(1);
            }
            // A canvas flap only when the sail actually moved a notch (not when a
            // key is pressed at the end stops). (`SailingView` flapUp/flapDown.)
            if sail_mode > prev_sail {
                sounds.sail_up();
            } else if sail_mode < prev_sail {
                sounds.sail_down();
            }
            helm = Helm {
                turn: read_turn(log_open),
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
            // a stronger rig runs faster, an overladen hull crawls.
            let scale = upgrades::speed_scale(gs.sail_level, gs.cargo_used());
            kin = sailing::step_scaled(kin, helm, wind, dt, scale);
            // Keep the hull out of every nearby island.
            let near = world.islands_near(kin.pos, 400.0);
            kin = sailing::resolve_grounding(kin, &near);

            // --- Salvage -------------------------------------------------------
            // Scoop up any flotsam the ship has sailed over (gold straight to the
            // purse, with a chime + a fading toast), then keep fresh salvage drifting
            // ahead of the bow for the next stretch of open water.
            let haul = flotsam.collect_near(kin.pos, flotsam::REACH);
            if haul.gold > 0 {
                gs.gold += haul.gold;
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
                        let scale = upgrades::speed_scale(gs.sail_level, 0);
                        let rhelm = race::rival_helm(&rk, target.pos, wind);
                        let stepped = sailing::step_scaled(rk, rhelm, wind, dt, scale);
                        let rnear = world.islands_near(stepped.pos, 400.0);
                        rival = Some(sailing::resolve_grounding(stepped, &rnear));
                        if race::reached(&kin, target) {
                            gs.win_race();
                            rival = None;
                            race_ready = false;
                            race_running = false;
                            sounds.race_won();
                        } else if rival.map_or(false, |rk| race::reached(&rk, target)) {
                            gs.lose_race();
                            rival = None;
                            race_ready = false;
                            race_running = false;
                            sounds.race_lost();
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
            if is_key_pressed(KeyCode::Space) && sail_mode == 0 && harbor.try_dock(&mut gs) {
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
            // Skip the clock forward ~3 hours, to jump ahead through the cycle.
            if is_key_pressed(KeyCode::T) {
                tod = (tod + 0.12).rem_euclid(1.0);
            }
            if is_key_pressed(KeyCode::L) {
                log_open = !log_open;
                // Open the book to its first spread each time (the original rewinds
                // to spread 0 on close).
                if log_open {
                    log_spread = 0;
                }
            }
            // Page the open log with the left/right arrows (no mouse to click the
            // original's nav arrows). Clamped at the covers — no wrap-around.
            if log_open {
                if is_key_pressed(KeyCode::Right) {
                    log_spread = (log_spread + 1).min(captains_log::NUM_SPREADS - 1);
                }
                if is_key_pressed(KeyCode::Left) {
                    log_spread = log_spread.saturating_sub(1);
                }
            }
            if is_key_pressed(KeyCode::B) {
                bloom_on = !bloom_on;
            }
            if is_key_pressed(KeyCode::Escape) {
                if log_open {
                    log_open = false;
                } else {
                    break;
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
        let mut world_cam = Camera2D::from_display_rect(Rect::new(0.0, 0.0, w, h));
        if bloom_on {
            // Render the world into the bloom's scene texture. Rendering to a target
            // flips `invert_y`, which cancels the screen flip below — so leave `zoom.y`
            // as `from_display_rect` set it and the net matrix matches the screen path.
            world_cam.render_target = Some(bloom.scene_target(w, h));
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

        draw_sky(sky_grad, storm, w, horizon, overscan);
        // Stars, then the moon and sun arcing over on the clock.
        celestial::draw(&sky, &stars, t, kin.heading_rad, half_fov_h_view, w, h, horizon, storm);

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
                let rel = wrap_angle(kin.pos.bearing_to(i.pos) - kin.heading_rad);
                d <= MAX_VIEW && d >= i.radius && rel.abs() <= half_fov_h_view * 1.6
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
                let rel = wrap_angle(kin.pos.bearing_to(f.pos) - kin.heading_rad);
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

        // --- Waves -------------------------------------------------------------
        renderer.render(
            &kin,
            t,
            sea,
            motion.heave,
            &sea_pal,
            sky_grad,
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
        // viewport rather than riding the swell. With bloom on, this also extracts and
        // blurs the bright parts of the scene texture and composites them to the screen.
        if bloom_on {
            bloom.render_to_screen(w, h);
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
        ship.render(&rig, dt, t, day_lit, storm, w, h);

        // --- HUD ---------------------------------------------------------------
        // Wind is shown by the quarter it blows *from* (the seaman's convention).
        let knots = kin.speed() / sailing::KNOT;
        let wind_from = compass(wrap_angle(wind.toward_rad + std::f32::consts::PI));
        let point = wind.point_of_sail(kin.heading_rad).label();
        let hud = format!(
            "{}  Sail: {}  {:.1} kn  ·  Wind {}  ({})",
            day.label(),
            SAIL_NAMES[sail_mode],
            knots,
            wind_from,
            point,
        );
        draw_text(&hud, 16.0, 28.0, 24.0, WHITE);
        draw_text(
            "W/S sail · A/D helm · Space dock · Q/E weather · T time · [ ] wind · L log · B bloom · Esc quit",
            16.0,
            52.0,
            20.0,
            Color::new(1.0, 1.0, 1.0, 0.7),
        );
        // Purse + hold, so the captain can read his fortunes from the helm too.
        let purse = format!("Gold {}  ·  Hold {}/{}", gs.gold, gs.hold_used(), gs.hold_capacity);
        draw_text(&purse, 16.0, 76.0, 22.0, Color::new(1.0, 0.92, 0.6, 1.0));

        // Salvage pickup toast: a gold note that floats up and fades over the deck
        // when a piece is hauled aboard.
        salvage_flash = (salvage_flash - dt).max(0.0);
        if salvage_flash > 0.0 && !harbor.is_open() && !log_open {
            let p = salvage_flash / SALVAGE_FLASH_TIME; // 1 → 0 as it fades
            let fs = 30;
            let dims = measure_text(&salvage_msg, None, fs, 1.0);
            let tx = w * 0.5 - dims.width / 2.0;
            let ty = h * 0.34 - (1.0 - p) * 36.0; // drifts upward as it fades
            draw_text(
                &salvage_msg,
                tx + 1.0,
                ty + 1.0,
                fs as f32,
                Color::new(0.0, 0.0, 0.0, 0.5 * p),
            );
            draw_text(&salvage_msg, tx, ty, fs as f32, Color::new(1.0, 0.9, 0.5, p));
        }

        // The destinations marked on the charts: every accepted contract, plus the
        // race mark while one is booked.
        let mut chart_marks: Vec<i32> = gs.active_missions.iter().map(|m| m.target_id).collect();
        if let Some(r) = gs.race {
            chart_marks.push(r.target_id);
        }

        // Always-on corner chart: the local cluster, top-right.
        let map_size = (h * 0.24).clamp(140.0, 200.0);
        let map_rect = Rect::new(w - map_size - 16.0, 16.0, map_size, map_size);
        minimap::render(
            &world,
            &kin,
            wind,
            map_rect,
            &minimap_pal,
            &chart_marks,
            None,
            &traders.positions(),
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
                let fs = 24;
                let dims = measure_text(&text, None, fs, 1.0);
                let bx = w * 0.5 - dims.width / 2.0;
                let by = h * 0.14;
                draw_rectangle(
                    bx - 16.0,
                    by - 26.0,
                    dims.width + 32.0,
                    38.0,
                    Color::new(0.10, 0.06, 0.02, 0.6),
                );
                draw_text(&text, bx, by, fs as f32, Color::new(1.0, 0.92, 0.6, 1.0));
            }
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
                log_spread,
                dt,
                w,
                h,
            );
        }

        // The docking call-to-action (while sailing), then the port board (docked).
        port_view::render_prompt(&harbor, &world, sail_mode == 0, w, h);
        if harbor.is_open() {
            if let Some(id) = gs.docked_island_id() {
                let market = Market::for_island(&world.islands[id as usize], world.seed);
                if let Some(screen) = harbor.screen.as_ref() {
                    screen.render(&gs, &world, &market, &kin, wind, w, h);
                }
            }
        }

        next_frame().await
    }
}
