//! Throwaway dev preview: stands a lineup of island scenery models on flat ground
//! under the game's own projection and lighting, and dumps a PNG, so a new
//! `FeatureKind`'s shape can be judged without sailing a chart to find one.
//! Edit `lineup` to choose the models shown; the default is the newest additions.

// The shared game modules carry plenty of code this preview never calls.
#![allow(dead_code)]

use macroquad::prelude::{
    clear_background, draw_rectangle, draw_triangle, get_screen_data, next_frame, screen_height,
    screen_width, vec2, Color, Conf,
};

#[path = "../src/geometry.rs"]
mod geometry;
#[path = "../src/rng.rs"]
mod rng;
#[path = "../src/world.rs"]
mod world;
#[path = "../src/isle_terrain.rs"]
mod isle_terrain;
#[path = "../src/isle_features.rs"]
mod isle_features;
#[path = "../src/projection.rs"]
mod projection;
#[path = "../src/sailing.rs"]
mod sailing;
#[path = "../src/islands_render.rs"]
mod islands_render;
#[path = "../src/feature_models.rs"]
mod feature_models;

use geometry::Vec2;
use isle_features::{FeatureKind, IsleFeature};
use islands_render::{island_light, warm_light, IslandView};
use sailing::Kinematics;

fn conf() -> Conf {
    Conf { window_title: "feature preview".into(), window_width: 1600, window_height: 900, ..Default::default() }
}

/// The lineup: kind, a representative in-game height (m), and a size multiplier.
fn lineup() -> Vec<(FeatureKind, f32, f32)> {
    use FeatureKind::*;
    vec![
        (Mushrooms, 1.8, 1.1),
        (StandingStones, 6.5, 1.0),
        (Totem, 8.5, 1.0),
        (Statue, 6.5, 1.0),
        (WhaleBones, 5.0, 1.2),
        (BasaltColumn, 5.5, 1.0),
        (Tent, 3.2, 1.0),
        (Boat, 2.5, 1.2),
        // Repeats at other hashed yaws, to judge the asymmetric models all round.
        (Boat, 2.5, 1.2),
        (WhaleBones, 5.0, 1.2),
    ]
}

#[macroquad::main(conf)]
async fn main() {
    let out = std::env::var("PREVIEW_OUT").unwrap_or_else(|_| "feature_preview.png".into());
    let mut frames = 0;
    loop {
        let (w, h) = (screen_width(), screen_height());
        clear_background(Color::new(0.45, 0.68, 0.86, 1.0));

        // The game camera at the origin looking due north, and its noon light.
        let kin = Kinematics::still(Vec2::ZERO, 0.0);
        let horizon = h * 0.54;
        let px_per_rad = h * 0.85;
        let half_fov_h_view = projection::MAX_HALF_FOV_H.min((w * 0.5) / px_per_rad);
        let px_per_rad_h = (w * 0.5) / half_fov_h_view;
        let sun_len = (0.6f32 * 0.6 + 0.4 * 0.4 + 0.7 * 0.7).sqrt();
        let (key, ambient) = island_light(1.0, [255.0, 246.0, 220.0], [168.0, 200.0, 255.0]);
        let (warm, warm_amt) = warm_light([255.0, 246.0, 220.0]);
        let v = IslandView {
            w,
            horizon,
            px_per_rad,
            px_per_rad_h,
            half_fov_h_view,
            eye_rise: 0.0,
            sun: (0.6 / sun_len, -0.4 / sun_len, 0.7 / sun_len),
            key,
            ambient,
            warm,
            warm_amt,
            lamp: 0.0,
            t: 0.0,
        };
        // A grass apron below the horizon so the lineup stands on land.
        draw_rectangle(0.0, horizon, w, h - horizon, Color::new(0.32, 0.55, 0.30, 1.0));

        let kinds = lineup();
        let dist = 55.0f32;
        let spread = 66.0f32;
        let mut tris = Vec::new();
        for (i, &(kind, height, size)) in kinds.iter().enumerate() {
            let x = (i as f32 - (kinds.len() as f32 - 1.0) * 0.5) / (kinds.len() as f32 - 1.0)
                * spread;
            let f = IsleFeature { kind, offset: Vec2::new(x, dist), height, size };
            let wp = Vec2::new(x, dist);
            let key = kin.pos.distance_to(wp);
            // Stand clear of the sub-0.5 m waterline projection regime, like
            // terrain-set features do in the game.
            feature_models::emit(&f, i * 7 + 3, wp, 1.5, key, 1.0, &kin, &v, &mut tris);
        }
        tris.sort_by(|a, b| b.key.total_cmp(&a.key));
        for t in &tris {
            draw_triangle(
                vec2(t.p[0].0, t.p[0].1),
                vec2(t.p[1].0, t.p[1].1),
                vec2(t.p[2].0, t.p[2].1),
                t.color,
            );
        }

        frames += 1;
        if frames >= 4 {
            get_screen_data().export_png(&out);
            break;
        }
        next_frame().await;
    }
}
