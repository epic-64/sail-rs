//! sail-rs — a macroquad port of the ScalaJS sailing game.
//!
//! This stage sets up the first-person scene (sky gradient, sun, horizon) and the
//! world-anchored wave system, and lets you sail a free camera around the swell.
//! Islands, the ship deck/rig, HUD and weather come in later stages.

mod geometry;
mod isle_features;
mod islands_render;
mod ocean;
mod ocean_renderer;
mod palette;
mod projection;
mod rng;
mod sailing;
mod ship_render;
mod world;

use macroquad::prelude::*;

use geometry::{clamp, wrap_angle, Vec2};
use ocean_renderer::{OceanRenderer, SUN_BEARING};
use palette::Daytime;
use projection::MAX_VIEW;
use rng::Rng;
use sailing::{Helm, Kinematics, Wind};
use ship_render::{RigInput, ShipRenderer};
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
/// toward the storm overcast by `storm`. Drawn as horizontal strips since
/// macroquad has no built-in gradient.
fn draw_sky(day: Daytime, storm: f32, w: f32, horizon: f32, m: f32) {
    let fair = palette::fair_sky(day);
    let top = lerp3(fair[0], palette::STORM_SKY[0], storm);
    let mid = lerp3(fair[1], palette::STORM_SKY[1], storm);
    let hor = lerp3(fair[2], palette::STORM_SKY[2], storm);

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

/// How visible the sun is at each phase (`SailingView.sunVisibility`).
fn sun_visibility(d: Daytime) -> f32 {
    match d {
        Daytime::Dawn => 0.85,
        Daytime::Day => 0.95,
        Daytime::Dusk => 0.55,
        Daytime::Night => 0.0,
    }
}

/// Draw the sun as a soft disc, panning with the helm and fading by daytime/storm.
fn draw_sun(day: Daytime, storm: f32, heading: f32, half_fov_h: f32, w: f32, h: f32, horizon: f32) {
    let rel = wrap_angle(SUN_BEARING - heading);
    let sun_half_fov = half_fov_h * 1.1;
    if rel.abs() > sun_half_fov {
        return;
    }
    let vis = sun_visibility(day) * (1.0 - clamp(storm, 0.0, 1.0));
    if vis <= 0.01 {
        return;
    }
    let x = w * 0.5 + (rel / half_fov_h) * (w * 0.5);
    let y = horizon * 0.42;
    let r = h * 0.055;
    // Warm core with a faint halo, tinted by the daytime's sun colour.
    let sun = palette::palette_for(day);
    let core = Color::new(sun[12] / 255.0, sun[13] / 255.0, sun[14] / 255.0, vis);
    let halo = Color::new(sun[12] / 255.0, sun[13] / 255.0, sun[14] / 255.0, vis * 0.18);
    draw_circle(x, y, r * 2.1, halo);
    draw_circle(x, y, r * 1.4, halo);
    draw_circle(x, y, r, core);
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
const CAM_PITCH_PX: f32 = 230.0; // px the horizon drops per rad of bow-up pitch
const CAM_PITCH_MAX: f32 = 120.0;
const CAM_YAW_PX: f32 = 70.0; // px the view swings per rad of hull yaw
const CAM_YAW_MAX: f32 = 42.0;
// Wind heel: the press of the sails leans the boat away from the wind, hardest on
// a beam reach (most side-force) and nil dead before the wind or in irons.
const HEEL_GAIN: f32 = 0.22; // rad of lean at full sail on a hard beam reach

/// The rudder demand from the helm keys: A/D (or arrows) held, [-1, 1].
/// (`SailingView.heldTurn`.)
fn read_turn() -> f32 {
    let mut turn = 0.0;
    if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
        turn += 1.0;
    }
    if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
        turn -= 1.0;
    }
    turn
}

/// 8-point compass label for a bearing (radians, 0 = N, CW).
fn compass(bearing_rad: f32) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = (bearing_rad.to_degrees().rem_euclid(360.0) + 22.5) / 45.0;
    POINTS[(deg as usize) % 8]
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut day = Daytime::Day;
    let mut renderer = OceanRenderer::new(day);

    // Build the world and start the captain just off the home cluster's shipyard
    // port, bow pointed at it, so there's land in view from the first frame.
    let world = world::generate(1);
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
    // Sit to the south of the isle (so the SE sun lights its face) and look north.
    let start = Vec2::new(start_isle.pos.x, start_isle.pos.y - (start_isle.radius + 360.0));
    let mut kin = Kinematics::still(start, start.bearing_to(start_isle.pos));

    let mut sea: f32 = 0.6; // sea-state scalar (0 glassy … ~1.3 storm)
    let mut storm: f32 = 0.0; // gale fury [0,1]

    // Discrete sail setting, set once with W/S and held. Start furled (None) so the
    // captain raises sail to get under way, just like the original.
    let mut sail_mode: usize = 0;

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

    loop {
        let dt = get_frame_time().min(0.05);
        let t = get_time() as f32;
        let w = screen_width();
        let h = screen_height();
        let horizon = h * 0.54;

        // --- Input -------------------------------------------------------------
        // Sails are set in discrete notches (W raises, S lowers) — set once, the
        // ship keeps going; only the *first* press of a held key steps the sail.
        if is_key_pressed(KeyCode::W) || is_key_pressed(KeyCode::Up) {
            sail_mode = (sail_mode + 1).min(SAIL_FRACTIONS.len() - 1);
        }
        if is_key_pressed(KeyCode::S) || is_key_pressed(KeyCode::Down) {
            sail_mode = sail_mode.saturating_sub(1);
        }
        let helm = Helm {
            turn: read_turn(),
            throttle: SAIL_FRACTIONS[sail_mode],
        };

        // The wind backs/veers to a fresh random quarter every WIND_PERIOD seconds.
        clock += dt;
        if clock - last_wind_shift >= WIND_PERIOD {
            wind = Wind::random(&mut wind_rng);
            last_wind_shift = clock;
        }
        // Dev aid (not in the original): nudge the wind with [ and ] to feel the
        // points of sail and tacking on demand.
        if is_key_down(KeyCode::RightBracket) {
            wind.toward_rad = wrap_angle(wind.toward_rad + dt * 0.8);
        }
        if is_key_down(KeyCode::LeftBracket) {
            wind.toward_rad = wrap_angle(wind.toward_rad - dt * 0.8);
        }

        kin = sailing::step(kin, helm, wind, dt);
        // Keep the hull out of every nearby island.
        let near = world.islands_near(kin.pos, 400.0);
        kin = sailing::resolve_grounding(kin, &near);

        if is_key_down(KeyCode::E) {
            sea = (sea + dt * 0.4).min(1.3);
        }
        if is_key_down(KeyCode::Q) {
            sea = (sea - dt * 0.4).max(0.0);
        }
        if is_key_down(KeyCode::G) {
            storm = (storm + dt * 0.5).min(1.0);
        } else {
            storm = (storm - dt * 0.5).max(0.0);
        }
        if is_key_pressed(KeyCode::T) {
            day = day.next();
        }
        if is_key_pressed(KeyCode::Escape) {
            break;
        }

        // --- Ship motion + camera ride -----------------------------------------
        // Sample how the swell throws the hull this frame, then ease the parts that
        // should rock with the long swell rather than buzz with the chop.
        let motion = ocean::ship_motion(kin.pos, kin.heading_rad, t, sea);
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
        let cam_vert = clamp(CAM_PITCH_PX * smooth_pitch, -CAM_PITCH_MAX, CAM_PITCH_MAX);
        let cam_sway = clamp(CAM_YAW_PX * smooth_yaw, -CAM_YAW_MAX, CAM_YAW_MAX);

        // --- Scene (drawn through the camera ride) -----------------------------
        let px_per_rad = h * 0.85;
        let half_fov_h_view = projection::MAX_HALF_FOV_H.min((w * 0.5) / px_per_rad);

        clear_background(BLACK);
        // The tilted/translated view must never reveal background past the painted
        // sea and sky, so everything world-anchored is over-scanned past the screen
        // edges by this much (sized to cover the clamped roll + translate).
        let overscan = w.max(h) * 0.25 + 60.0;
        let mut world_cam = Camera2D::from_display_rect(Rect::new(0.0, 0.0, w, h));
        // `from_display_rect` builds its zoom for *render-to-texture*; drawn straight
        // to the screen it comes out vertically flipped (its clip-space Y is the
        // negative of macroquad's default screen projection). Flip `zoom.y` back so
        // the world is upright and matches the screen-space ship/HUD drawn after.
        world_cam.zoom.y = -world_cam.zoom.y;
        world_cam.rotation = cam_roll_deg;
        world_cam.target = vec2(w * 0.5 + cam_sway, h * 0.5 - cam_vert);
        set_camera(&world_cam);

        draw_sky(day, storm, w, horizon, overscan);
        draw_sun(day, storm, kin.heading_rad, half_fov_h_view, w, h, horizon);

        // Distant-water backdrop behind the wave mesh, over-scanned so a rolled view
        // still finds deep sea in the corners.
        let far = lerp3(
            (palette::palette_for(day)[6], palette::palette_for(day)[7], palette::palette_for(day)[8]),
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

        // --- Waves -------------------------------------------------------------
        renderer.render(&kin, t, sea, motion.heave, day, storm, w, h, &visible);

        // Back to screen space for the foreground + HUD, which stay bolted to the
        // viewport rather than riding the swell.
        set_default_camera();

        // --- Ship (deck + rig) -------------------------------------------------
        // The deck leans with the same swell roll *and* wind heel, so she visibly
        // heels under sail while the horizon tilts the other way; pitch/heave are
        // read straight off the swell.
        let rig = RigInput {
            motion: ocean::ShipMotion {
                roll: smooth_roll + smooth_heel,
                yaw: smooth_yaw,
                ..motion
            },
            set: helm.throttle,
            turn: helm.turn,
            wind_rel,
        };
        ship.render(&rig, dt, t, day, storm, w, h);

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
            "W/S sail  ·  A/D helm  ·  Q/E sea  ·  G storm  ·  T time  ·  [ ] wind  ·  Esc quit",
            16.0,
            52.0,
            20.0,
            Color::new(1.0, 1.0, 1.0, 0.7),
        );

        next_frame().await
    }
}
