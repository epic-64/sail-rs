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
use crate::ocean;
use crate::palette::{self, Daytime, Palette, PALETTE_LEN};
use crate::projection::BASE_EYE;
use crate::sailing::Kinematics;

/// Vertical exaggeration applied to wave displacement (and to the ship's heave,
/// so the bob stays in sync). Shared with the island projection so land bobs by
/// the same factor as the sea around it.
pub const WAVE_GAIN: f32 = 4.6;

/// The bearing the sun sits at on the world chart (`SailingView.sunBearing`).
pub const SUN_BEARING: f32 = 2.4;

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
    sun_elev: f32,
    shininess: f32,
    base_saturation: f32,
    height_shade: f32,

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
    c_glow: (f32, f32, f32),

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
    pub fn new(start_day: Daytime) -> Self {
        // We keep the original chunky, low-poly *near* look (flat-shaded facets ≈
        // the old 52×36 canvas mesh) but bias the rows hard toward the far field so
        // the distant bands get subdivided instead of stretching into big flat
        // triangles at the horizon — see `row_bias` in `render`. The mesh runs all
        // the way out to `f_far` (near the true horizon) so the sea fills the
        // distance, while the colour ramp uses the nearer `depth_far`.
        let cols = 60;
        let rows = 104;
        let f_near = 6.0;
        let f_far = 2600.0;
        let live = palette::palette_for(start_day);
        OceanRenderer {
            cols,
            rows,
            row_bias: 2.7,
            fov_margin: 1.12,
            f_near,
            f_far,
            depth_far: 850.0,
            sun_elev: 0.55,
            shininess: 90.0,
            base_saturation: 1.22,
            height_shade: 0.38,
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
            sss_power: 3.0,
            sss_scale: 0.7,
            c_glow: (46.0, 222.0, 168.0),
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

    /// The current far / mid / near water colours (daytime-eased, storm-blended),
    /// so a static distant backdrop can read the same depth ramp. Valid after
    /// [`OceanRenderer::render`].
    pub fn water_ramp(&self) -> (Color, Color, Color) {
        (
            col3(self.p_far, 1.0),
            col3(self.p_mid, 1.0),
            col3(self.p_near, 1.0),
        )
    }

    fn live_col(&self, o: usize) -> [f32; 3] {
        [self.shown[o], self.shown[o + 1], self.shown[o + 2]]
    }

    pub fn render(
        &mut self,
        kin: &Kinematics,
        t: f32,
        sea: f32,
        heave: f32,
        day: Daytime,
        storm: f32,
        w: f32,
        h: f32,
    ) {
        // Ease the live palette toward the current daytime's target with a slow
        // cross-fade, then blend toward the cold storm palette by the gale's fury.
        let dt = match self.prev_t {
            None => 0.0,
            Some(p) => (t - p).min(0.1),
        };
        self.prev_t = Some(t);
        let k = clamp(dt * 0.9, 0.0, 1.0);
        let target = palette::palette_for(day);
        let storm_blend = clamp(storm, 0.0, 1.0) * 0.9;
        for p in 0..PALETTE_LEN {
            self.live[p] += (target[p] - self.live[p]) * k;
            self.shown[p] = self.live[p] + (palette::STORM_PALETTE[p] - self.live[p]) * storm_blend;
        }
        self.p_near = self.live_col(0);
        self.p_mid = self.live_col(3);
        self.p_far = self.live_col(6);
        self.p_foam = self.live_col(9);
        self.p_sun = self.live_col(12);
        self.p_sky = self.live_col(15);
        self.p_glint = self.live_col(18);

        let horizon = h * 0.54;
        let px_per_rad = h * 0.85;
        // Match horizontal scale to vertical, clamped to maxHalfFovH; wider windows
        // stretch the capped bearing span across the extra width.
        let half_fov_h_view = (crate::projection::MAX_HALF_FOV_H).min((w * 0.5) / px_per_rad);
        let px_per_rad_h = (w * 0.5) / half_fov_h_view;

        let fwd = Vec2::from_heading(kin.heading_rad);
        let right = Vec2::new(kin.heading_rad.cos(), -kin.heading_rad.sin());

        // Sun direction in the camera's (right, forward, up) frame.
        let sun_rel = wrap_angle(SUN_BEARING - kin.heading_rad);
        let lx = sun_rel.sin() * self.sun_elev.cos();
        let ly = sun_rel.cos() * self.sun_elev.cos();
        let lz = self.sun_elev.sin();

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

            if j > 0 {
                self.paint_band(f, prev_f - f, sea, lx, ly, lz);
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

        // Streak the surface flecks on top of the finished wave mesh.
        self.paint_flow(
            kin, t, sea, heave, horizon, px_per_rad, px_per_rad_h, half_fov_h_view, fwd, right, w, h,
        );
    }

    /// Fill the strip of quads between the previous (farther) row and the current
    /// (nearer) row, shading each from its slope against the sun. `f_near_row` is
    /// the current row's distance, used for the depth-based base colour.
    fn paint_band(&self, f_near_row: f32, row_df: f32, sea: f32, lx: f32, ly: f32, lz: f32) {
        let depth = clamp((f_near_row - self.f_near) / (self.depth_far - self.f_near), 0.0, 1.0);
        let [near_r, near_g, near_b] = self.p_near;
        let [mid_r, mid_g, mid_b] = self.p_mid;
        let [far_r, far_g, far_b] = self.p_far;
        let [sun_r, sun_g, sun_b] = self.p_sun;
        let [sky_r, sky_g, sky_b] = self.p_sky;
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
            // Diffuse term against the sun (Lambert).
            let diff = clamp(nx * lx + ny * ly + nz * lz, 0.0, 1.0);

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
            // Fresnel: grazing facets pick up the sky.
            let n_dot_v = clamp(nx * vx + ny * vy + nz * vz, 0.0, 1.0);
            let u = 1.0 - n_dot_v;
            let u2 = u * u;
            let fres = u2 * u2 * u; // (1 - nDotV)^5
            // Whitecaps: foam on the tallest crests and steepest, breaking faces.
            let crest = clamp(mid_z / max_amp, -1.0, 1.0);
            let steep = clamp(slope_fwd * 1.6, 0.0, 1.0);
            let foam = (if crest > 0.55 {
                (crest - 0.55) / 0.45 * 0.7
            } else {
                0.0
            })
            .max(steep * 0.5);

            // Subsurface glow: only reared-up crests with the sun behind them.
            let sss = if crest > 0.0 {
                let lhx = lx + nx * self.sss_distort;
                let lhy = ly + ny * self.sss_distort;
                let lhz = lz + nz * self.sss_distort;
                let lh_i = 1.0 / (lhx * lhx + lhy * lhy + lhz * lhz).sqrt();
                let back = clamp(-(vx * lhx + vy * lhy + vz * lhz) * lh_i, 0.0, 1.0);
                back.powf(self.sss_power) * crest * self.sss_scale
            } else {
                0.0
            };

            // Height shade: troughs sit in shadow, crests catch more light.
            let shade = clamp(1.0 + crest * self.height_shade, 0.55, 1.6);
            let mut r = base_r * shade;
            let mut g = base_g * shade;
            let mut b = base_b * shade;
            let t_lit = 0.30 * diff;
            r += (sun_r - r) * t_lit;
            g += (sun_g - g) * t_lit;
            b += (sun_b - b) * t_lit;
            let t_sky = 0.40 * fres;
            r += (sky_r - r) * t_sky;
            g += (sky_g - g) * t_sky;
            b += (sky_b - b) * t_sky;
            if sss > 0.0 {
                r += (glow_r - r) * sss;
                g += (glow_g - g) * sss;
                b += (glow_b - b) * sss;
            }
            if spec > 0.0 {
                r += (glint_r - r) * spec;
                g += (glint_g - g) * spec;
                b += (glint_b - b) * spec;
            }
            if foam > 0.0 {
                let tf = foam.min(1.0);
                r += (foam_r - r) * tf;
                g += (foam_g - g) * tf;
                b += (foam_b - b) * tf;
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
    #[allow(clippy::too_many_arguments)]
    fn paint_flow(
        &self,
        kin: &Kinematics,
        t: f32,
        sea: f32,
        heave: f32,
        horizon: f32,
        px_per_rad: f32,
        px_per_rad_h: f32,
        half_fov_h_view: f32,
        fwd: Vec2,
        right: Vec2,
        w: f32,
        h: f32,
    ) {
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
                if self.flow_occluded(kin, d, dist, t, sea, heave, horizon, px_per_rad, sy) {
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

    #[allow(clippy::too_many_arguments)]
    fn flow_occluded(
        &self,
        kin: &Kinematics,
        d: Vec2,
        dist: f32,
        t: f32,
        sea: f32,
        heave: f32,
        horizon: f32,
        px_per_rad: f32,
        sy: f32,
    ) -> bool {
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
