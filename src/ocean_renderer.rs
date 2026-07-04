//! The sea as a world-anchored 3D wave mesh, ported from
//! `client.OceanRenderer`.
//!
//! Each frame we project a grid of real world points (sampled from
//! [`ocean::height`]) into the first-person view using the same camera the
//! islands use ([`projection`]). Because the grid is anchored to the *world* and
//! merely re-bearing-ed relative to the helm, turning sweeps the near water
//! across the view while the horizon holds still. Quads are flat-shaded from the
//! surface normal: a diffuse term warms crests toward the sun, a Blinn-Phong
//! specular highlight breaks into a sparkling glitter road, a Fresnel term
//! reflects the sky on grazing facets, back-lit crests glow with subsurface
//! scattering, and whitecaps foam on the tallest/steepest faces. A sparse field
//! of world-anchored foam flecks streaks across the near water to read headway.
//!
//! The Scala original draws onto a 2D `<canvas>` and coalesces same-colour quad
//! runs into single fills; here we draw each quad as two `draw_triangle`s and let
//! macroquad batch them.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::isle_features::IsleFeature;
use crate::islands_render::{paint_island, IslandView};
use crate::ocean;
use crate::palette::{self, Palette};
use crate::projection::BASE_EYE;
use crate::sailing::Kinematics;
use crate::scene::SceneView;
use crate::world::Island;

/// Vertical exaggeration applied to wave displacement (and to the ship's heave,
/// so the bob stays in sync). Shared with the island projection so land bobs by
/// the same factor as the sea around it.
pub const WAVE_GAIN: f32 = 4.6;

/// The cold blue-white a lightning strike throws on the water, matched to the lit
/// cloud (`clouds::GLOW`), and how hard the flash lifts a facet toward it at full
/// glare. The flash is shaped per facet so only the sky-facing/grazing quads pop, and
/// confined to a pool of `LIGHTNING_DIR_WIDTH` radians about the strike's bearing so it
/// lights the water *toward* the bolt rather than the whole sea.
const LIGHTNING_COL: [f32; 3] = [200.0, 216.0, 244.0];
const LIGHTNING_GAIN: f32 = 0.7;
const LIGHTNING_DIR_WIDTH: f32 = 0.34;

pub struct OceanRenderer {
    // Grid resolution. Columns span the field of view; rows march out to sea.
    cols: usize,
    rows: usize,
    // Row-distribution exponent (>1): pushes rows toward the far field so distant
    // bands subdivide finely (no big flat horizon triangles) while near bands stay
    // large and chunky. 1.0 = the old even-in-depression-angle spacing.
    row_bias: f32,

    fov_margin: f32,
    f_near: f32,
    f_far: f32,
    // Distance (m) over which the depth *colour* ramp (near→mid→far teal) runs.
    // Decoupled from `f_far` so the mesh can reach right out to the horizon without
    // washing the whole sea into the pale far colour.
    depth_far: f32,
    // How brightly the active light (sun by day, moon by night) burns this frame.
    // Scales the sun-warmth and subsurface glow so the sea dims into night and lifts
    // again at dawn. Set each frame from the sky clock.
    light_strength: f32,
    // The active light's *visibility on the water* this frame: `light_strength` faded
    // out by the gale's fury, since a storm's overcast hides the sun/moon disc (see
    // `celestial::draw`). The mirror-bright specular glitter (and the directional
    // sun-warmth) read off this, so the light source's reflection vanishes from the
    // sea along with the disc rather than glittering under a sky with no sun in it.
    light_source_vis: f32,
    // This frame's lightning glare in [0,1], set from `clouds::StormSky::flash`: the
    // sky's flash mirrored on the water for the instant a bolt fires, and that strike's
    // bearing relative to the view, so the flash falls on the water on its side only.
    lightning: f32,
    lightning_rel: f32,
    shininess: f32,
    base_saturation: f32,
    // How the facet's own brightness is modelled — the "wave shading" that makes
    // some quads lighter than their neighbours. `height_shade` lifts crests over
    // troughs; `slope_shade` lights the swell face turned toward the sun and shades
    // its lee back (a soft, wrapped Lambert so it rolls rather than snaps);
    // `sky_shade` lifts facets tilted up to the open sky.
    height_shade: f32,
    slope_shade: f32,
    sky_shade: f32,
    // How hard the wave *tops* shift toward the bright, saturated crest tone (the
    // subsurface "lit thin water" read large), giving strong colour variation
    // through the body of the swell on top of the multiplicative height shade.
    crest_brighten: f32,

    crest_glass: f32,
    crest_fade_lo: f32,
    crest_fade_hi: f32,

    // Surface drift flecks.
    flow_cell: f32,
    flow_far: f32,
    flow_near: f32,
    flow_alpha: f32,
    flow_shutter: f32,
    flow_step: f32,
    flow_max_steps: i32,

    // Subsurface scattering.
    sss_distort: f32,
    sss_power: f32,
    sss_scale: f32,
    // Always-on translucent inner glow on crests, independent of back-lighting, so
    // the top of every wave reads as thin lit water rather than only the back-lit
    // ones — the signature emerald-through-the-crest look.
    sss_ambient: f32,
    c_glow: (f32, f32, f32),

    // Sky reflection. The Fresnel term now mirrors the live sky *gradient* — the
    // reflected view ray's elevation picks horizon vs zenith sky — through a Schlick
    // curve so flat water is near-transparent and grazing facets near-mirror.
    fresnel_f0: f32,
    reflect_strength: f32,
    // The live sky gradient ends (storm-blended), refreshed each frame so reflections
    // track the painted sky exactly.
    sky_horizon: [f32; 3],
    sky_zenith: [f32; 3],

    // How fully night has fallen this frame (`palette::night_factor`), kept so
    // `scene_light` can brighten the land and ship on the same long twilight
    // window the painted sea palette uses.
    night: f32,

    // Eased palette state.
    live: Palette,
    shown: Palette,
    prev_t: Option<f32>,

    // Per-frame light colours unpacked from `shown` (channels in [0,255]).
    p_near: [f32; 3],
    p_mid: [f32; 3],
    p_far: [f32; 3],
    p_foam: [f32; 3],
    p_sun: [f32; 3],
    p_sky: [f32; 3],
    p_glint: [f32; 3],

    // Reusable column buffers.
    phis: Vec<f32>,
    screen_x: Vec<f32>,
    prev_y: Vec<f32>,
    prev_z: Vec<f32>,
    cur_y: Vec<f32>,
    cur_z: Vec<f32>,
    cur_x: Vec<f32>,     // lateral world-offset factor tan(phi)
    cos_phi: Vec<f32>,   // cos of each column bearing
    cur_x_span: Vec<f32>, // tan(phi) span across each quad
}

#[inline]
fn col3(c: [f32; 3], a: f32) -> Color {
    Color::new(c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, a)
}

/// A cheap, well-mixed integer hash of a world grid cell (`OceanRenderer.hash2`).
fn hash2(ix: i32, iy: i32) -> i32 {
    let mut hh = ix.wrapping_mul(374761393).wrapping_add(iy.wrapping_mul(668265263));
    hh = (hh ^ (hh >> 13)).wrapping_mul(1274126177);
    hh ^ (hh >> 16)
}

impl OceanRenderer {
    pub fn new(start_tod: f32) -> Self {
        // We keep the original chunky, low-poly *near* look (flat-shaded facets ≈
        // the old 52×36 canvas mesh) but bias the rows hard toward the far field so
        // the distant bands get subdivided instead of stretching into big flat
        // triangles at the horizon — see `row_bias` in `render`. The mesh runs all
        // the way out to `f_far` (near the true horizon) so the sea fills the
        // distance, while the colour ramp uses the nearer `depth_far`.
        let cols = 60;
        let rows = 104;
        // Nearest sampled distance (m). In calm water this row runs off-screen
        // under the deck, but it must reach close to the bow: when a large crest
        // rears up a few metres ahead, the water on *its near face* (between the
        // eye and the crest) fills the screen below the crest line. Sample only
        // out to ~6m and that face is unmeshed — the flat near-water skirt floods
        // it as a dark, sine-edged silhouette, as if looking *into* the wave.
        // Reaching in to ~2.5m meshes that face so it renders as shaded,
        // foam-flecked bands and the flat skirt retreats to the off-screen sliver.
        let f_near = 2.5;
        let f_far = 2600.0;
        let live = palette::sea_palette(start_tod);
        OceanRenderer {
            cols,
            rows,
            row_bias: 2.7,
            fov_margin: 1.12,
            f_near,
            f_far,
            depth_far: 850.0,
            light_strength: 1.0,
            light_source_vis: 1.0,
            lightning: 0.0,
            lightning_rel: 0.0,
            shininess: 90.0,
            base_saturation: 1.7,
            height_shade: 0.62,
            slope_shade: 0.42,
            sky_shade: 0.16,
            crest_brighten: 0.74,
            crest_glass: 0.2,
            crest_fade_lo: 0.12,
            crest_fade_hi: 0.42,
            flow_cell: 17.0,
            flow_far: 230.0,
            flow_near: 4.0,
            flow_alpha: 0.5,
            flow_shutter: 0.16,
            flow_step: 9.0,
            flow_max_steps: 12,
            sss_distort: 0.35,
            sss_power: 2.6,
            sss_scale: 0.95,
            sss_ambient: 0.16,
            c_glow: (40.0, 232.0, 172.0),
            fresnel_f0: 0.02,
            reflect_strength: 0.46,
            sky_horizon: [0.0; 3],
            sky_zenith: [0.0; 3],
            night: 0.0,
            live,
            shown: live,
            prev_t: None,
            p_near: [0.0; 3],
            p_mid: [0.0; 3],
            p_far: [0.0; 3],
            p_foam: [0.0; 3],
            p_sun: [0.0; 3],
            p_sky: [0.0; 3],
            p_glint: [0.0; 3],
            phis: vec![0.0; cols],
            screen_x: vec![0.0; cols],
            prev_y: vec![0.0; cols],
            prev_z: vec![0.0; cols],
            cur_y: vec![0.0; cols],
            cur_z: vec![0.0; cols],
            cur_x: vec![0.0; cols],
            cos_phi: vec![0.0; cols],
            cur_x_span: vec![0.0; cols],
        }
    }

    fn live_col(&self, o: usize) -> [f32; 3] {
        [self.shown[o], self.shown[o + 1], self.shown[o + 2]]
    }

    /// The coloured light pair the scenery is shaded with this frame: an overall
    /// brightness (never to full black, so a moonlit silhouette keeps a fifth of
    /// its daylight and its shape still reads), split into a *key* light (hue from
    /// the sea palette's sun-warmth channel) and an *ambient* sky fill (hue from
    /// the mean of the sky dome). The land and deck thus redden at dusk and cool
    /// under the moon with the sea and sky, instead of only dimming. Both feed off
    /// the storm-blended live colours, so a gale drains the warmth toward pewter
    /// along with everything else. Valid once `render` has eased this frame's
    /// palette; the foreground ship reads the same pair (`main.rs`), so the deck
    /// sits in the very light the islands take.
    ///
    /// Brightness follows the *painted* day, not the raw sun: the sea palette
    /// holds its full dusk fire until the sun is well down and brightens ahead of
    /// the sunrise (`palette::night_factor`'s long window), so the land and deck
    /// track `1 - night` through the twilight or they'd fall to black against a
    /// still-blazing sea. The active light's strength only takes over where it's
    /// the brighter claim (a moonlit midnight lifting the floor).
    pub fn scene_light(&self) -> ((f32, f32, f32), (f32, f32, f32)) {
        let brightness = 0.22 + 0.78 * self.light_strength.max(1.0 - self.night);
        let sky_mean = [
            (self.sky_zenith[0] + self.sky_horizon[0]) * 0.5,
            (self.sky_zenith[1] + self.sky_horizon[1]) * 0.5,
            (self.sky_zenith[2] + self.sky_horizon[2]) * 0.5,
        ];
        crate::islands_render::island_light(brightness, self.p_sun, sky_mean)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        kin: &Kinematics,
        t: f32,
        sea: f32,
        heave: f32,
        // The world bearing the wind blows toward: the rival and the traders trim
        // their rigs and stream their pennants by it (see `SceneView::wind_toward`).
        wind_toward: f32,
        // The sea palette for the current clock (already blended across the day),
        // the fair-weather sky gradient to reflect, and the active light: its world
        // bearing, sine-altitude and brightness (sun by day, moon by night).
        target_sea: &Palette,
        sky_grad: [(f32, f32, f32); 3],
        light_az: f32,
        light_alt: f32,
        light_strength: f32,
        storm: f32,
        // How fully night has fallen (0 by day, 1 once the sun is well down). The
        // storm overcast the water mirrors darkens with it, matching the painted sky.
        night: f32,
        // This frame's lightning glare in [0,1] (see `clouds::StormSky::flash`) and the
        // world bearing it strikes from: a brief cold flash run across the swell on the
        // strike's side, so a bolt lights the water in its direction, not everywhere.
        lightning: f32,
        lightning_az: f32,
        w: f32,
        h: f32,
        // Visible islands paired with their features, sorted *descending* by near-
        // shore distance (farthest first), so we can draw each between the wave
        // bands at its own depth.
        islands: &[(&Island, &[IsleFeature])],
        // The racing rival, if one is on the water, slotted into the same depth
        // march so nearer wave crests and islands occlude it. `rival_light` dims it
        // into the night with the rest of the scene; `rival_hull` is the shape she
        // sails (the tier her race demands, see `crate::hull_shape`).
        rival: Option<Kinematics>,
        rival_hull: &'static crate::hull_shape::HullShape,
        rival_light: f32,
        // Visible floating salvage (position + kind), sorted *descending* by
        // distance (farthest first) like the islands, so each piece slots into the
        // march at its own depth and nearer water paints over it. Dimmed by the same
        // `rival_light` (the scene light).
        flotsam: &[(Vec2, crate::flotsam::FlotsamKind)],
        // The local cluster's wandering traders, sorted *descending* by distance
        // (farthest first) like everything else, so each merchant slots into the
        // depth march and nearer crests/islands occlude it. Drawn like the rival but
        // flying a green pennant.
        traders: &[Kinematics],
        // How brightly the harbour lights burn (the dusk ramp): fed to the island
        // view so a port's houses light their windows after dark.
        lamp: f32,
        // The visible ports' town lights (after dusk). Each is baked into the per-
        // facet wave shading as a low, warm local light, so the road is occluded by
        // nearer crests for free instead of shining through the swell.
        lights: &[crate::port_lights::PortLight],
    ) {
        // Ease the live palette toward the clock's target with a slow cross-fade,
        // then blend toward the cold storm palette by the gale's fury.
        let dt = match self.prev_t {
            None => 0.0,
            Some(p) => (t - p).min(0.1),
        };
        self.prev_t = Some(t);
        self.light_strength = light_strength;
        // The sun/moon disc is swallowed by a storm's overcast, so its mirror glitter
        // on the water has to go with it: fade the light-source reflection by the fury.
        self.light_source_vis = light_strength * (1.0 - clamp(storm, 0.0, 1.0));
        self.night = clamp(night, 0.0, 1.0);
        self.lightning = clamp(lightning, 0.0, 1.0);
        // The strike's bearing across the view, so the flash below lights only the water
        // turned toward it (a pool on the strike's side, not a wash over the whole sea).
        self.lightning_rel = wrap_angle(lightning_az - kin.heading_rad);
        let k = clamp(dt * 0.9, 0.0, 1.0);
        let target = *target_sea;
        // Ease toward the storm sea, night-darkened so a midnight gale is dark and
        // unsaturated rather than the daytime slate (a lightning strike relights it).
        let storm_sea = palette::storm_palette(night);
        let storm_blend = clamp(storm, 0.0, 1.0) * 0.9;
        for (((live, shown), &tgt), &storm) in self
            .live
            .iter_mut()
            .zip(self.shown.iter_mut())
            .zip(target.iter())
            .zip(storm_sea.iter())
        {
            *live += (tgt - *live) * k;
            *shown = *live + (storm - *live) * storm_blend;
        }
        self.p_near = self.live_col(0);
        self.p_mid = self.live_col(3);
        self.p_far = self.live_col(6);
        self.p_foam = self.live_col(9);
        self.p_sun = self.live_col(12);
        self.p_sky = self.live_col(15);
        self.p_glint = self.live_col(18);

        // The live sky gradient the water reflects: the clock's fair-weather sky,
        // blended toward the storm overcast (night-darkened to match `main::draw_sky`,
        // so the mirrored sky and the painted sky are the same colours).
        let storm_c = clamp(storm, 0.0, 1.0);
        let storm_sky = palette::storm_sky(night);
        let fair = sky_grad;
        let blend = |a: (f32, f32, f32), b: (f32, f32, f32)| {
            [
                a.0 + (b.0 - a.0) * storm_c,
                a.1 + (b.1 - a.1) * storm_c,
                a.2 + (b.2 - a.2) * storm_c,
            ]
        };
        self.sky_zenith = blend(fair[0], storm_sky[0]);
        self.sky_horizon = blend(fair[2], storm_sky[2]);

        let horizon = h * 0.54;
        let px_per_rad = h * 0.85;
        // Match horizontal scale to vertical, clamped to maxHalfFovH; wider windows
        // stretch the capped bearing span across the extra width.
        let half_fov_h_view = (crate::projection::MAX_HALF_FOV_H).min((w * 0.5) / px_per_rad);
        let px_per_rad_h = (w * 0.5) / half_fov_h_view;

        let fwd = Vec2::from_heading(kin.heading_rad);
        let right = Vec2::new(kin.heading_rad.cos(), -kin.heading_rad.sin());

        // Resolve the visible ports' town lights into this frame's camera basis once,
        // so the band march can shade each into the wave facets without re-projecting.
        let cam_lights = crate::port_lights::camera_frame(lights, kin, fwd, right);

        // The active light's world-frame direction (chart x/y, z up), shared by the
        // island facets and the rival's hull; split into the camera frame below for
        // the wave glitter. The altitude is its sine (the vertical component); the
        // horizontal component is the matching cosine across the bearing.
        let light_horiz = (1.0 - light_alt * light_alt).max(0.0).sqrt();
        let sun_world = (
            light_az.sin() * light_horiz,
            light_az.cos() * light_horiz,
            light_alt,
        );

        // The per-frame sea camera, handed to every billboard and the flow pass so
        // each reads the projection it needs instead of a dozen positional floats.
        // `light` here is `rival_light`: the scene light the floating objects take.
        let scene = SceneView {
            kin,
            t,
            sea,
            heave,
            light: rival_light,
            horizon,
            px_per_rad,
            px_per_rad_h,
            half_fov_h_view,
            fwd,
            right,
            w,
            h,
            sun: sun_world,
            wind_toward,
        };

        // Active-light direction in the camera's (right, forward, up) frame. As the
        // sun arcs over and sets, this swings the glitter and shading.
        let light_rel = wrap_angle(light_az - kin.heading_rad);
        let lx = light_rel.sin() * light_horiz;
        let ly = light_rel.cos() * light_horiz;
        let lz = light_alt;

        // Day/night island lighting: see `scene_light` (this frame's palette state
        // was just eased above, so the pair is current).
        let (key, ambient) = self.scene_light();
        // The sunset/sunrise warm-shift pull, from the same warmth channel the key
        // light takes, so the land reddens at dusk beyond what the multiply alone can.
        let (warm, warm_amt) = crate::islands_render::warm_light(self.p_sun);

        // Island view: same camera, with the light in *world* space (chart x/y, z up)
        // so the landmass facets shade consistently as the ship turns and the sun moves.
        let view = IslandView {
            w,
            horizon,
            px_per_rad,
            px_per_rad_h,
            half_fov_h_view,
            eye_rise: heave * WAVE_GAIN,
            sun: sun_world,
            key,
            ambient,
            warm,
            warm_amt,
            lamp,
            t,
        };
        // Near-shore distance key per island (aligned with `islands`), used to slot
        // each island into the band march. Farthest-first to match the band order.
        let isle_key = |isle: &Island| kin.pos.distance_to(isle.pos) - isle.radius;
        let mut isle_idx = 0;
        // The rival is slotted in at its straight-line distance, drawn once the
        // march descends past it (so every nearer band/island then paints over it).
        let rival_dist = rival.map(|rk| kin.pos.distance_to(rk.pos));
        let mut rival_done = rival.is_none();
        // Floating salvage marches in alongside the islands: each piece is drawn once
        // the band march descends past its distance (farthest first), so nearer bands
        // then paint over it just as they do the islands' bases.
        let mut flot_idx = 0;
        let mut draw_rival = |f: f32| {
            if rival_done {
                return;
            }
            if let (Some(rk), Some(d)) = (rival, rival_dist) {
                if d >= f {
                    crate::rival_render::draw(
                        &rk,
                        &scene,
                        crate::rival_render::RIVAL_PENNANT,
                        rival_hull,
                    );
                    rival_done = true;
                }
            }
        };
        // Traders march in alongside the islands and salvage: each is drawn once the
        // band march descends past its distance (farthest first), so nearer bands
        // then paint over it. `trd_idx` walks the farthest-first list. They are
        // small trading craft, so they sail the shipyard's smallest hull.
        let trader_hull = crate::hull_shape::for_level(0);
        let mut trd_idx = 0;

        // Even screen-row spacing: linear in the depression angle of the flat sea.
        let th_far = (BASE_EYE / self.f_far).atan();
        let th_near = (BASE_EYE / self.f_near).atan();

        for i in 0..self.cols {
            self.phis[i] = (i as f32 / (self.cols - 1) as f32 * 2.0 - 1.0)
                * half_fov_h_view
                * self.fov_margin;
            self.screen_x[i] = w * 0.5 + self.phis[i] * px_per_rad_h;
            self.cur_x[i] = self.phis[i].tan();
            self.cos_phi[i] = self.phis[i].cos();
            if i > 0 {
                self.cur_x_span[i - 1] = self.cur_x[i] - self.cur_x[i - 1];
            }
        }

        // March from the far row toward the viewer, painting the quad band between
        // the previous row and the current one. Nearer bands draw last (on top).
        // `prev_f` carries the previous (farther) row's distance so each band's
        // forward slope uses its *real* world spacing — far bands span hundreds of
        // metres, near ones a few — instead of one coarse average. That makes the
        // surface normals (and so the lighting) accurate at every range.
        let mut prev_f = 0.0;
        let mut j = 0;
        while j <= self.rows {
            // Bias the depression-angle march toward the far field: small steps near
            // the horizon (j≈0) pack the distant bands; large steps near the viewer
            // keep the foreground facets big and chunky.
            let frac = (j as f32 / self.rows as f32).powf(self.row_bias);
            let th = th_far + (th_near - th_far) * frac;
            let f = BASE_EYE / th.tan();
            for c in 0..self.cols {
                let s = f * self.cur_x[c];
                let wp = kin.pos + fwd * f + right * s;
                let z = ocean::height(wp, t, sea);
                self.cur_z[c] = z;
                // Project height relative to the ship's heave so the sea drops away
                // under the camera on a crest and rises in the trough. Scale by
                // cos(phi): the true ground distance off-axis is f / cos(phi).
                self.cur_y[c] = horizon
                    + ((BASE_EYE - (z - heave) * WAVE_GAIN) * self.cos_phi[c] / f).atan()
                        * px_per_rad;
            }

            // Draw every island farther than this band's near edge *before* the
            // band, so the band (nearer water) then paints over its base — a near
            // crest rolls in front of a far island while its summit stands clear.
            // (At j=0, f = f_far: this also flushes isles beyond the mesh, behind
            // all the waves.)
            while isle_idx < islands.len() && isle_key(islands[isle_idx].0) >= f {
                paint_island(islands[isle_idx].0, islands[isle_idx].1, kin, &view);
                isle_idx += 1;
            }
            // The rival, the traders and any salvage sit among the islands at their
            // own depths, then the band paints over them just as it does the islands'
            // bases.
            draw_rival(f);
            while trd_idx < traders.len() && kin.pos.distance_to(traders[trd_idx].pos) >= f {
                crate::rival_render::draw(
                    &traders[trd_idx],
                    &scene,
                    crate::rival_render::TRADER_PENNANT,
                    trader_hull,
                );
                trd_idx += 1;
            }
            while flot_idx < flotsam.len() && kin.pos.distance_to(flotsam[flot_idx].0) >= f {
                crate::flotsam_render::draw(flotsam[flot_idx].0, flotsam[flot_idx].1, &scene);
                flot_idx += 1;
            }

            if j > 0 {
                self.paint_band(f, prev_f - f, sea, lx, ly, lz, &cam_lights);
            }

            std::mem::swap(&mut self.prev_y, &mut self.cur_y);
            std::mem::swap(&mut self.prev_z, &mut self.cur_z);
            prev_f = f;
            j += 1;
        }

        // Skirt the nearest row (now in prev_y after the last swap) down to the
        // bottom edge so the near water runs unbroken into the corners on a wide
        // window. On a normal window the near row is already off the bottom.
        let near_col = col3(self.p_near, 1.0);
        for sc in 0..self.cols - 1 {
            let y_l = self.prev_y[sc].min(h);
            let y_r = self.prev_y[sc + 1].min(h);
            if y_l < h || y_r < h {
                let x_l = self.screen_x[sc];
                let x_r = self.screen_x[sc + 1];
                draw_triangle(vec2(x_l, y_l), vec2(x_r, y_r), vec2(x_r, h), near_col);
                draw_triangle(vec2(x_l, y_l), vec2(x_r, h), vec2(x_l, h), near_col);
            }
        }

        // Any remaining islands are nearer than the closest band: draw them in
        // front of all the water.
        while isle_idx < islands.len() {
            paint_island(islands[isle_idx].0, islands[isle_idx].1, kin, &view);
            isle_idx += 1;
        }
        // A rival nearer than the closest band stands in front of all the water.
        draw_rival(0.0);
        // Any traders nearer than the closest band stand in front of all the water.
        while trd_idx < traders.len() {
            crate::rival_render::draw(
                &traders[trd_idx],
                &scene,
                crate::rival_render::TRADER_PENNANT,
                trader_hull,
            );
            trd_idx += 1;
        }
        // Any salvage nearer than the closest band floats in front of all the water.
        while flot_idx < flotsam.len() {
            crate::flotsam_render::draw(flotsam[flot_idx].0, flotsam[flot_idx].1, &scene);
            flot_idx += 1;
        }

        // Streak the surface flecks on top of the finished wave mesh.
        self.paint_flow(&scene);
    }

    /// Fill the strip of quads between the previous (farther) row and the current
    /// (nearer) row, shading each from its slope against the sun. `f_near_row` is
    /// the current row's distance, used for the depth-based base colour. `lights` are
    /// the visible ports' town lights (camera frame), baked into each facet as a warm
    /// local light so the harbour road shades the water like a low sun.
    #[allow(clippy::too_many_arguments)]
    fn paint_band(
        &self,
        f_near_row: f32,
        row_df: f32,
        sea: f32,
        lx: f32,
        ly: f32,
        lz: f32,
        lights: &[crate::port_lights::CamLight],
    ) {
        let depth = clamp((f_near_row - self.f_near) / (self.depth_far - self.f_near), 0.0, 1.0);
        let [near_r, near_g, near_b] = self.p_near;
        let [mid_r, mid_g, mid_b] = self.p_mid;
        let [far_r, far_g, far_b] = self.p_far;
        let [sun_r, sun_g, sun_b] = self.p_sun;
        let [skh_r, skh_g, skh_b] = self.sky_horizon;
        let [skz_r, skz_g, skz_b] = self.sky_zenith;
        let [glint_r, glint_g, glint_b] = self.p_glint;
        let [foam_r, foam_g, foam_b] = self.p_foam;
        let raw_r = {
            let m = near_r + (mid_r - near_r) * depth;
            m + (far_r - m) * depth
        };
        let raw_g = {
            let m = near_g + (mid_g - near_g) * depth;
            m + (far_g - m) * depth
        };
        let raw_b = {
            let m = near_b + (mid_b - near_b) * depth;
            m + (far_b - m) * depth
        };
        // Push the depth-ramped base a touch more saturated.
        let base_lum = raw_r * 0.299 + raw_g * 0.587 + raw_b * 0.114;
        let base_r = base_lum + (raw_r - base_lum) * self.base_saturation;
        let base_g = base_lum + (raw_g - base_lum) * self.base_saturation;
        let base_b = base_lum + (raw_b - base_lum) * self.base_saturation;
        // The sun-warmed emerald the subsurface glow blends toward (constant/frame).
        let glow_r = self.c_glow.0 + (sun_r - self.c_glow.0) * 0.30;
        let glow_g = self.c_glow.1 + (sun_g - self.c_glow.1) * 0.30;
        let glow_b = self.c_glow.2 + (sun_b - self.c_glow.2) * 0.30;
        // The bright tone the wave tops glow toward: the horizon-bright water warmed
        // a touch toward the sun, then pushed hard-saturated so the crests pop. Held
        // independent of the sun's strength so the night sea still lifts cyan crests
        // over black troughs. Troughs simply keep the dark depth-ramp base, so the
        // swell body runs dark-deep → bright-lit from trough to crest.
        let cr_r = far_r + (sun_r - far_r) * 0.26;
        let cr_g = far_g + (sun_g - far_g) * 0.26;
        let cr_b = far_b + (sun_b - far_b) * 0.26;
        let cr_lum = cr_r * 0.299 + cr_g * 0.587 + cr_b * 0.114;
        let crest_r = clamp(cr_lum + (cr_r - cr_lum) * 1.5, 0.0, 255.0);
        let crest_g = clamp(cr_lum + (cr_g - cr_lum) * 1.5, 0.0, 255.0);
        let crest_b = clamp(cr_lum + (cr_b - cr_lum) * 1.5, 0.0, 255.0);

        let (road_r, road_g, road_b) = crate::port_lights::ROAD_COL;

        let nearness = 1.0 - depth;
        let max_amp = (0.4_f32).max(ocean::MAX_AMPLITUDE * sea);

        for c in 0..self.cols - 1 {
            // Quad corners: near row (cur) = bottom edge, far row (prev) = top edge.
            let z_l = self.cur_z[c];
            let z_r = self.cur_z[c + 1];
            let slope_lat = (z_r - z_l) / (1e-3_f32).max((self.cur_x_span[c] * f_near_row).abs());
            let slope_fwd = (self.prev_z[c] - self.cur_z[c]) / (0.5_f32).max(row_df);
            // Unit surface normal in the camera's (right, fwd, up) frame.
            let inv_n = 1.0 / (slope_lat * slope_lat + slope_fwd * slope_fwd + 1.0).sqrt();
            let nx = -slope_lat * inv_n;
            let ny = -slope_fwd * inv_n;
            let nz = inv_n;
            // Diffuse term against the sun (Lambert), kept signed so the shading
            // below can wrap it softly around the back of each swell.
            let lambert = nx * lx + ny * ly + nz * lz;
            let diff = lambert.max(0.0);

            // View vector from this facet back to the eye.
            let s_mid = (self.cur_x[c] + self.cur_x[c + 1]) * 0.5 * f_near_row;
            let mid_z = (z_l + z_r) * 0.5;
            let vxr = -s_mid;
            let vyr = -f_near_row;
            let vzr = BASE_EYE - mid_z;
            let v_inv = 1.0 / (vxr * vxr + vyr * vyr + vzr * vzr).sqrt();
            let vx = vxr * v_inv;
            let vy = vyr * v_inv;
            let vz = vzr * v_inv;

            // Blinn-Phong specular: half-vector between sun and eye.
            let hx0 = lx + vx;
            let hy0 = ly + vy;
            let hz0 = lz + vz;
            let h_inv = 1.0 / (hx0 * hx0 + hy0 * hy0 + hz0 * hz0).sqrt();
            let n_dot_h = clamp((nx * hx0 + ny * hy0 + nz * hz0) * h_inv, 0.0, 1.0);
            let spec = if n_dot_h > 0.9 {
                n_dot_h.powf(self.shininess)
            } else {
                0.0
            };
            // Fresnel reflection: grazing facets mirror the sky, head-on facets stay
            // near-clear, on a Schlick curve (F0≈0.02 for water). The *colour* of the
            // reflection is the live sky gradient sampled by where the reflected view
            // ray points — facets tilted to bounce the eye toward the zenith show the
            // deep sky, those that bounce it along the horizon show the bright band —
            // so the same chunky facet picks up a different sky tone as it rocks.
            let n_dot_v = clamp(nx * vx + ny * vy + nz * vz, 0.0, 1.0);
            let u = 1.0 - n_dot_v;
            let u2 = u * u;
            let fres = self.fresnel_f0 + (1.0 - self.fresnel_f0) * u2 * u2 * u;
            // Reflected view ray R = 2(N·V)N − V; its up-component is the elevation we
            // sample the sky band at (downward bounces, rare on water, read as horizon).
            let r_up = clamp(2.0 * n_dot_v * nz - vz, 0.0, 1.0).sqrt();
            let sky_r = skh_r + (skz_r - skh_r) * r_up;
            let sky_g = skh_g + (skz_g - skh_g) * r_up;
            let sky_b = skh_b + (skz_b - skh_b) * r_up;
            // Whitecaps: foam on the tallest crests and steepest, breaking faces.
            let crest = clamp(mid_z / max_amp, -1.0, 1.0);
            let steep = clamp(slope_fwd * 1.6, 0.0, 1.0);
            let foam = (if crest > 0.55 {
                (crest - 0.55) / 0.45 * 0.7
            } else {
                0.0
            })
            .max(steep * 0.5);

            // Subsurface glow. Two parts, both gated on a reared-up crest (thin water
            // up top transmits light): a back-lit term that flares where the sun sits
            // behind the wave and shines through it, plus an always-on inner glow so
            // every crest reads as translucent lit water, not only the back-lit ones.
            // The crest's height is the stand-in for how thin/lit the water there is.
            let sss = if crest > 0.0 {
                let lhx = lx + nx * self.sss_distort;
                let lhy = ly + ny * self.sss_distort;
                let lhz = lz + nz * self.sss_distort;
                let lh_i = 1.0 / (lhx * lhx + lhy * lhy + lhz * lhz).sqrt();
                let back = clamp(-(vx * lhx + vy * lhy + vz * lhz) * lh_i, 0.0, 1.0);
                let lit = back.powf(self.sss_power) * self.sss_scale + self.sss_ambient;
                (lit * crest).min(0.85)
            } else {
                0.0
            };

            // Facet luminance — the wave shading that lights some quads over others,
            // three orientation cues summed about a mid-grey of 1.0:
            //   • height: crests catch the light, troughs sit in shadow;
            //   • sun-facing slope: a wrapped Lambert (lambert·½+½, eased) so the face
            //     of a swell turned toward the sun glows and its lee back falls into
            //     soft shadow, the brightness rolling up the wave rather than snapping
            //     at the terminator;
            //   • sky fill: facets tilted up to the open sky lift, those tipped away dim.
            let wrapped = clamp(lambert * 0.5 + 0.5, 0.0, 1.0);
            let sun_face = wrapped * wrapped; // ease: deeper lee shadow, brighter face
            let sky_face = 0.5 + 0.5 * nz; // up-facing → toward 1, steep faces → toward 0.5
            let shade = clamp(
                1.0 + crest * self.height_shade
                    + (sun_face - 0.5) * self.slope_shade
                    + (sky_face - 0.5) * self.sky_shade,
                0.28,
                1.85,
            );
            let mut r = base_r * shade;
            let mut g = base_g * shade;
            let mut b = base_b * shade;
            // Shift the wave tops toward the bright, saturated crest tone (eased so
            // only the upper third of the swell lifts), leaving troughs on the dark
            // deep-water base. This is the main colour variation through the body.
            let up = clamp(crest, 0.0, 1.0);
            if up > 0.0 {
                let tc = up * up * self.crest_brighten;
                r += (crest_r - r) * tc;
                g += (crest_g - g) * tc;
                b += (crest_b - b) * tc;
            }
            // Direct sun-warmth: the light source tinting the faces turned toward it.
            // Gated on the disc's visibility, so the overcast that hides the sun also
            // lifts its warm wash off the water (the sky reflection below carries the
            // grey instead).
            let t_lit = 0.30 * diff * self.light_source_vis;
            r += (sun_r - r) * t_lit;
            g += (sun_g - g) * t_lit;
            b += (sun_b - b) * t_lit;
            let t_sky = self.reflect_strength * fres;
            r += (sky_r - r) * t_sky;
            g += (sky_g - g) * t_sky;
            b += (sky_b - b) * t_sky;
            if sss > 0.0 {
                let ss = sss * self.light_strength;
                r += (glow_r - r) * ss;
                g += (glow_g - g) * ss;
                b += (glow_b - b) * ss;
            }
            if spec > 0.0 {
                // The mirror-bright glitter of the sun/moon on the water: faded with
                // the disc's visibility so the storm overcast leaves no glitter road.
                let sp = spec * self.light_source_vis;
                r += (glint_r - r) * sp;
                g += (glint_g - g) * sp;
                b += (glint_b - b) * sp;
            }
            if self.lightning > 0.0 {
                // A lightning strike's glare caught on the swell: a brief cold flash,
                // strongest on the flatter, sky-facing facets and grazing reflections,
                // so it reads as the sky's flash mirrored on some quads rather than a
                // flat wash, plus an extra spark off the specular crests it catches.
                // Weighted toward grazing facets and specular crests (where an elevated
                // bolt actually mirrors), with only a slight flat-water floor, so the lit
                // patch is a reflection streak rather than a broad pool.
                let refl = clamp(0.12 + 0.20 * nz + 0.55 * fres + spec, 0.0, 1.0);
                // Confine it to a narrow pool about the strike's bearing: this facet's
                // own bearing (its column's tan, undone) against the bolt's, on a soft
                // bell, so only the water turned toward the strike lights.
                let phi = ((self.cur_x[c] + self.cur_x[c + 1]) * 0.5).atan();
                let dsep = wrap_angle(phi - self.lightning_rel);
                let dir = (-(dsep / LIGHTNING_DIR_WIDTH).powi(2)).exp();
                let fl = (self.lightning * dir * refl * LIGHTNING_GAIN).min(0.9);
                r += (LIGHTNING_COL[0] - r) * fl;
                g += (LIGHTNING_COL[1] - g) * fl;
                b += (LIGHTNING_COL[2] - b) * fl;
            }
            // Harbour town lights: a warm pool on the swell faces turned toward the
            // port, plus a Blinn-Phong glitter road toward the eye. Independent of the
            // scene light (the town burns its own lamps), so it lifts the night sea.
            if !lights.is_empty() {
                let mut road = 0.0;
                for pl in lights {
                    road += pl.on_facet(s_mid, f_near_row, mid_z, nx, ny, nz, vx, vy, vz);
                }
                if road > 0.0 {
                    // The town road reads far too hot; scale it right down so it sits as
                    // a faint shimmer on the night sea rather than a bright glitter road.
                    let q = clamp(road * 0.12, 0.0, 1.0);
                    r += (road_r - r) * q;
                    g += (road_g - g) * q;
                    b += (road_b - b) * q;
                    // Hot cores whiten so the glitter road sparkles rather than smears.
                    let hot = q * q * q * 0.3;
                    r += (255.0 - r) * hot;
                    g += (255.0 - g) * hot;
                    b += (255.0 - b) * hot;
                }
            }
            if foam > 0.0 {
                // Whitecaps fade into the water as the light fails and take on the
                // active light's tone, so crests glow with the time of day — warm
                // at dusk, cool by moonlight — instead of staying pure white over a
                // dark sea. Only the bright midday sea froths fully white.
                let tf = (foam * (0.15 + 0.85 * self.light_strength)).min(1.0);
                let fr = foam_r + (sun_r - foam_r) * 0.32;
                let fg = foam_g + (sun_g - foam_g) * 0.32;
                let fb = foam_b + (sun_b - foam_b) * 0.32;
                r += (fr - r) * tf;
                g += (fg - g) * tf;
                b += (fb - b) * tf;
            }
            // Glassy crest fade toward transparent, near bands only.
            let crest_top = clamp(
                (crest - self.crest_fade_lo) / (self.crest_fade_hi - self.crest_fade_lo),
                0.0,
                1.0,
            );
            let alpha = 1.0 - crest_top * nearness * self.crest_glass;

            let color = Color::new(r / 255.0, g / 255.0, b / 255.0, alpha);
            // Overdraw each quad half a pixel right so neighbours tuck under and no
            // antialiased hairline seam shows through.
            let x_l = self.screen_x[c];
            let x_r = self.screen_x[c + 1] + 0.5;
            let by_l = self.cur_y[c]; // bottom (near) row
            let by_r = self.cur_y[c + 1];
            let ty_l = self.prev_y[c]; // top (far) row
            let ty_r = self.prev_y[c + 1];
            draw_triangle(vec2(x_l, by_l), vec2(x_r, by_r), vec2(x_r, ty_r), color);
            draw_triangle(vec2(x_l, by_l), vec2(x_r, ty_r), vec2(x_l, ty_l), color);
        }
    }

    /// Scatter the world-anchored foam flecks over the near water, each smeared
    /// from where it sat one shutter ago to where it sits now — its real
    /// screen-space optical flow.
    fn paint_flow(&self, view: &SceneView) {
        let SceneView {
            kin,
            t,
            sea,
            heave,
            horizon,
            px_per_rad,
            px_per_rad_h,
            half_fov_h_view,
            fwd,
            right,
            w,
            h,
            ..
        } = *view;
        let max_phi = half_fov_h_view * self.fov_margin;
        // The camera one shutter ago: way is made along the heading, so the gap
        // between a fleck's old and new screen spot is the optical flow we draw.
        let prev_pos = kin.pos - fwd * (kin.speed() * self.flow_shutter);
        let ix0 = ((kin.pos.x - self.flow_far) / self.flow_cell).floor() as i32;
        let ix1 = ((kin.pos.x + self.flow_far) / self.flow_cell).ceil() as i32;
        let iy0 = ((kin.pos.y - self.flow_far) / self.flow_cell).floor() as i32;
        let iy1 = ((kin.pos.y + self.flow_far) / self.flow_cell).ceil() as i32;
        let foam = self.p_foam;

        for ix in ix0..=ix1 {
            for iy in iy0..=iy1 {
                let hsh = hash2(ix, iy);
                // Keep ~60% of cells so the field reads as scattered foam, not a grid.
                if (hsh & 0xff) >= 150 {
                    continue;
                }
                let jx = ((hsh >> 8) & 0xff) as f32 / 255.0;
                let jy = ((hsh >> 16) & 0xff) as f32 / 255.0;
                let wp = Vec2::new(
                    ix as f32 * self.flow_cell + jx * self.flow_cell,
                    iy as f32 * self.flow_cell + jy * self.flow_cell,
                );
                let d = wp - kin.pos;
                let f = d.dot(fwd);
                if f <= self.flow_near {
                    continue;
                }
                let s = d.dot(right);
                let phi = s.atan2(f);
                let dist = (f * f + s * s).sqrt();
                if phi.abs() > max_phi || dist > self.flow_far {
                    continue;
                }
                let z = ocean::height(wp, t, sea);
                let sx = w * 0.5 + phi * px_per_rad_h;
                let sy = horizon
                    + ((BASE_EYE - (z - heave) * WAVE_GAIN) * phi.cos() / f).atan() * px_per_rad;
                let a = clamp(1.0 - dist / self.flow_far, 0.0, 1.0);
                let alpha = a * a * self.flow_alpha;
                if alpha <= 0.01 || sy >= h {
                    continue;
                }
                // Occlude flecks behind a swell: march the surface along this bearing
                // out toward the fleck; if a nearer point rises above it on screen,
                // an opaque crest stands in front. Early-out on the first hit.
                if self.flow_occluded(view, d, dist, sy) {
                    continue;
                }
                let sz = clamp(220.0 / f, 0.6, 3.0) * (0.7 + jx * 0.6);
                let color = col3(foam, alpha);
                // Where the same speck sat one shutter ago, seen from the old camera.
                let d0 = wp - prev_pos;
                let f0 = d0.dot(fwd);
                if f0 > self.flow_near {
                    let phi0 = d0.dot(right).atan2(f0);
                    let sx0 = w * 0.5 + phi0 * px_per_rad_h;
                    let sy0 = horizon
                        + ((BASE_EYE - (z - heave) * WAVE_GAIN) * phi0.cos() / f0).atan()
                            * px_per_rad;
                    draw_line(sx0, sy0, sx, sy, sz, color);
                } else {
                    draw_circle(sx, sy, sz * 0.5, color);
                }
            }
        }
    }

    fn flow_occluded(&self, view: &SceneView, d: Vec2, dist: f32, sy: f32) -> bool {
        let SceneView {
            kin,
            t,
            sea,
            heave,
            horizon,
            px_per_rad,
            ..
        } = *view;
        let ray_dir = d * (1.0 / dist);
        let mut dd = self.flow_near + self.flow_step;
        let mut steps = 0;
        while dd < dist && steps < self.flow_max_steps {
            let zc = ocean::height(kin.pos + ray_dir * dd, t, sea);
            let yc = horizon + ((BASE_EYE - (zc - heave) * WAVE_GAIN) / dd).atan() * px_per_rad;
            if yc < sy - 1.0 {
                return true;
            }
            dd += self.flow_step;
            steps += 1;
        }
        false
    }
}
