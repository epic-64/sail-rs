//! sail-rs — a macroquad port of the ScalaJS sailing game.
//!
//! This stage sets up the first-person scene (sky gradient, sun, horizon) and the
//! world-anchored wave system, and lets you sail a free camera around the swell.
//! Islands, the ship deck/rig, HUD and weather come in later stages.

mod geometry;
mod islands_render;
mod ocean;
mod ocean_renderer;
mod palette;
mod projection;
mod rng;
mod sailing;
mod world;

use macroquad::prelude::*;

use geometry::{clamp, wrap_angle, Vec2};
use ocean_renderer::{OceanRenderer, SUN_BEARING};
use palette::Daytime;
use projection::MAX_VIEW;
use sailing::{Helm, Kinematics};
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
fn draw_sky(day: Daytime, storm: f32, w: f32, horizon: f32) {
    let fair = palette::fair_sky(day);
    let top = lerp3(fair[0], palette::STORM_SKY[0], storm);
    let mid = lerp3(fair[1], palette::STORM_SKY[1], storm);
    let hor = lerp3(fair[2], palette::STORM_SKY[2], storm);

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
        // +1px overlap so no seams show between strips.
        draw_rectangle(0.0, y, w, strip_h + 1.0, rgb(c));
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

fn read_helm() -> Helm {
    let mut turn = 0.0;
    let mut throttle = 0.0;
    if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
        throttle = 1.0;
    }
    if is_key_down(KeyCode::S) || is_key_down(KeyCode::Down) {
        throttle = 0.0; // no reverse; brake by easing off
    }
    if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
        turn += 1.0;
    }
    if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
        turn -= 1.0;
    }
    Helm { turn, throttle }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut day = Daytime::Day;
    let mut renderer = OceanRenderer::new(day);

    // Build the world and start the captain in the home (centre) cluster's waters,
    // surrounded by its archipelago.
    let world = world::generate(1);
    let start = world.cluster_at(Vec2::ZERO).center;
    let mut kin = Kinematics::still(start, 0.0);

    let mut sea: f32 = 0.6; // sea-state scalar (0 glassy … ~1.3 storm)
    let mut storm: f32 = 0.0; // gale fury [0,1]

    loop {
        let dt = get_frame_time().min(0.05);
        let t = get_time() as f32;
        let w = screen_width();
        let h = screen_height();
        let horizon = h * 0.54;

        // --- Input -------------------------------------------------------------
        let helm = read_helm();
        kin = sailing::step(kin, helm, dt);
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

        // --- Scene -------------------------------------------------------------
        let px_per_rad = h * 0.85;
        let half_fov_h_view = projection::MAX_HALF_FOV_H.min((w * 0.5) / px_per_rad);

        clear_background(BLACK);
        draw_sky(day, storm, w, horizon);
        draw_sun(day, storm, kin.heading_rad, half_fov_h_view, w, h, horizon);

        // Distant-water backdrop behind the wave mesh, so the band between the
        // horizon and the farthest mesh row reads as deep sea.
        let far = lerp3(
            (palette::palette_for(day)[6], palette::palette_for(day)[7], palette::palette_for(day)[8]),
            (palette::STORM_PALETTE[6], palette::STORM_PALETTE[7], palette::STORM_PALETTE[8]),
            clamp(storm, 0.0, 1.0) * 0.9,
        );
        draw_rectangle(0.0, horizon, w, h - horizon, rgb(far));

        // --- Islands (drawn interleaved with the waves) ------------------------
        // Visible isles: in front of the camera and within view, sorted farthest-
        // first by near-shore distance so the wave renderer can slot each in at its
        // own depth.
        let key = |i: &Island| kin.pos.distance_to(i.pos) - i.radius;
        let mut visible: Vec<&Island> = world
            .islands_near(kin.pos, MAX_VIEW)
            .into_iter()
            .filter(|i| {
                let d = kin.pos.distance_to(i.pos);
                let rel = wrap_angle(kin.pos.bearing_to(i.pos) - kin.heading_rad);
                d <= MAX_VIEW && d >= i.radius * 1.1 && rel.abs() <= half_fov_h_view * 1.6
            })
            .collect();
        visible.sort_by(|a, b| key(b).partial_cmp(&key(a)).unwrap());

        // --- Waves -------------------------------------------------------------
        let heave = ocean::ship_motion(kin.pos, kin.heading_rad, t, sea).heave;
        renderer.render(&kin, t, sea, heave, day, storm, w, h, &visible);

        // --- HUD ---------------------------------------------------------------
        let knots = kin.speed() / sailing::KNOT;
        let hud = format!(
            "{}  sea {:.2}  storm {:.2}  {:.1} kn   [WASD sail  Q/E sea  G storm  T time]",
            day.label(),
            sea,
            storm,
            knots
        );
        draw_text(&hud, 16.0, 28.0, 24.0, WHITE);

        next_frame().await
    }
}
