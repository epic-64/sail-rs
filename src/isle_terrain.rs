//! The deterministic ground-truth *shape* of an island: its lumpy coastline and
//! its height surface. Split out of the renderer so the three systems that must
//! agree on where the land actually is all read the same model: the renderer
//! ([`crate::islands_render`]) draws it, collision ([`crate::sailing`]) keeps the
//! hull off the visible shore rather than a plain circle, and feature scatter
//! ([`crate::isle_features`]) refuses to stand a tree or house out over the water.
//!
//! Built fresh from the island's id + position (no world seed threaded through),
//! so a given chart always grows the same land.

use crate::geometry::Vec2;
use crate::rng::Rng;
use crate::world::{Island, IsleKind};

use std::f32::consts::TAU;

const GOLDEN: i64 = 0x9e3779b97f4a7c15u64 as i64;

/// One Gaussian hill: a bump centred at `off` (m from the island centre) rising
/// `height` m, with `sigma` the metric width of its skirt.
#[derive(Clone, Copy)]
pub struct Peak {
    pub off: Vec2,
    pub height: f32,
    pub sigma: f32,
}

/// One octave of directional value-noise: a travelling sine `amp·sin(dir·p·freq +
/// phase)` over the chart. Summed across octaves of rising frequency / falling
/// amplitude, these overlapping height-waves give the surface ridges, saddles and
/// dells instead of one smooth dome.
#[derive(Clone, Copy)]
pub struct Octave {
    pub freq: f32,
    pub dir: (f32, f32),
    pub phase: f32,
    pub amp: f32,
}

/// A deterministic per-island terrain. The outline is a lumpy coastline (mean
/// radius modulated by several harmonics: bays and headlands). The height surface
/// is the sum of a few Gaussian hills (the major massifs) plus several octaves of
/// overlapping noise-waves ([`Octave`]) that fold the slopes into ridges and
/// hollows, all faded to sea level at the shore.
pub struct IsleTerrain {
    pub center: Vec2,
    pub radius: f32,
    pub base: f32,
    pub lobes: [(f32, f32, f32); 6], // (frequency, amplitude, phase)
    pub peaks: Vec<Peak>,
    pub octaves: Vec<Octave>,
    /// Metres of relief the noise-waves add (±) on top of the Gaussian hills.
    pub relief: f32,
    /// Surface below this elevation (m) reads as beach/rock rim, above as foliage.
    pub beach: f32,
    /// Radial mesh resolution (ring count from centre to coast).
    pub rings: usize,
}

impl IsleTerrain {
    pub fn for_island(isle: &Island) -> IsleTerrain {
        // Vary the shape by both island id and its (seed-dependent) position, so
        // different worlds grow different coastlines for the same slot.
        let bits = (isle.id as i64).wrapping_mul(GOLDEN)
            ^ (isle.pos.x.to_bits() as i64)
            ^ ((isle.pos.y.to_bits() as i64) << 21);
        let mut rng = Rng::from_seed(bits);
        let r = isle.radius;
        let h = isle.height;
        let tau = TAU as f64;

        // Coastline lobes. The old outline was a couple of whole-island-scale sines on
        // top of a high `base`, so every isle came out a gently-wobbled disc (a blob).
        // Instead we weight several *shorter*-wavelength harmonics (freq 2..13, i.e.
        // that many headlands/bays around the shore) and let their amplitudes dominate
        // a much lower `base`, so the disc breaks up into a lumpy, lobed star: deep
        // gulfs between distinct headlands and peninsulas rather than one round mass.
        // Amplitudes run large on purpose; `coast_radius` clamps the sum into
        // `[0.3, 1.0]·radius`, so bays bite right in and a headland tops out flush with
        // the grounding circle (`radius`) rather than breaching it.
        let lobes = [
            (2.0, rng.between(0.10, 0.18) as f32, rng.between(0.0, tau) as f32),
            (3.0, rng.between(0.08, 0.15) as f32, rng.between(0.0, tau) as f32),
            (4.0, rng.between(0.07, 0.13) as f32, rng.between(0.0, tau) as f32),
            (6.0, rng.between(0.05, 0.10) as f32, rng.between(0.0, tau) as f32),
            (8.0, rng.between(0.03, 0.07) as f32, rng.between(0.0, tau) as f32),
            (13.0, rng.between(0.02, 0.045) as f32, rng.between(0.0, tau) as f32),
        ];

        // A small offset near the centre for a summit, so the peak isn't dead-centred.
        let peak = |rng: &mut Rng, rad_lo: f32, rad_hi: f32, height: f32, sig: f32| -> Peak {
            let a = rng.between(0.0, tau) as f32;
            let rad = rng.between(rad_lo as f64, rad_hi as f64) as f32 * r;
            Peak {
                off: Vec2::new(a.cos() * rad, a.sin() * rad),
                height,
                sigma: sig * r,
            }
        };

        // The major massifs (a handful of overlapping Gaussian hills) and how much
        // the noise-waves then fold the slopes. Volcanic keeps a recognisable cone;
        // rocky is the craggiest; green/jungle roll gently.
        let mut peaks = Vec::new();
        let relief = match isle.terrain {
            IsleKind::Volcanic => {
                // A dominant cone, then a scatter of lower parasitic vents/foothills
                // ringing it out toward the shore.
                peaks.push(peak(&mut rng, 0.0, 0.12, h, 0.30));
                let extra = rng.int_between(1, 3);
                for _ in 0..extra {
                    let hh = rng.between(0.35, 0.6) as f32 * h;
                    peaks.push(peak(&mut rng, 0.24, 0.55, hh, 0.20));
                }
                h * 0.24
            }
            IsleKind::Rocky => {
                // The craggiest: a knot of jagged summits of varied height flung right
                // out to the headlands.
                peaks.push(peak(&mut rng, 0.0, 0.18, h, 0.28));
                let extra = rng.int_between(3, 6);
                for _ in 0..extra {
                    let hh = rng.between(0.45, 0.9) as f32 * h;
                    peaks.push(peak(&mut rng, 0.20, 0.6, hh, 0.20));
                }
                h * 0.45
            }
            IsleKind::Green | IsleKind::Jungle => {
                // Rolling downs: a main rise and a few softer, broad-shouldered hills.
                peaks.push(peak(&mut rng, 0.0, 0.20, h, 0.36));
                let extra = rng.int_between(2, 4);
                for _ in 0..extra {
                    let hh = rng.between(0.45, 0.85) as f32 * h;
                    peaks.push(peak(&mut rng, 0.20, 0.56, hh, 0.28));
                }
                h * 0.34
            }
            IsleKind::Tropical => {
                // A flat sand bar: one broad, low dome and at most a couple of
                // gentle swells, with almost no noise so the strand stays smooth.
                peaks.push(peak(&mut rng, 0.0, 0.22, h, 0.42));
                let extra = rng.int_between(1, 3);
                for _ in 0..extra {
                    let hh = rng.between(0.4, 0.7) as f32 * h;
                    peaks.push(peak(&mut rng, 0.24, 0.55, hh, 0.32));
                }
                h * 0.18
            }
            IsleKind::Desert => {
                // Dune country: several broad mounds of similar height ranged
                // across the isle, with strong noise rippling them into crests.
                peaks.push(peak(&mut rng, 0.0, 0.24, h, 0.30));
                let extra = rng.int_between(3, 6);
                for _ in 0..extra {
                    let hh = rng.between(0.5, 0.85) as f32 * h;
                    peaks.push(peak(&mut rng, 0.18, 0.58, hh, 0.24));
                }
                h * 0.5
            }
        };

        // Overlapping height-waves: four octaves of directional value-noise, each
        // half the wavelength and ~half the amplitude of the last. The longest is a
        // touch over the island span (one or two broad swells across it); the
        // shortest stays above the mesh's sampling limit so it doesn't alias.
        let mut octaves = Vec::new();
        let mut wavelength = r * rng.between(1.1, 1.5) as f32;
        let mut amp = 1.0f32;
        for _ in 0..4 {
            let ang = rng.between(0.0, tau) as f32;
            octaves.push(Octave {
                freq: TAU / wavelength,
                dir: (ang.cos(), ang.sin()),
                phase: rng.between(0.0, tau) as f32,
                amp,
            });
            wavelength *= 0.5;
            amp *= 0.5;
        }

        let tall = matches!(isle.terrain, IsleKind::Rocky | IsleKind::Volcanic);
        // Tropical isles are mostly strand: the sand cutoff rides far up the (already
        // low) profile, so a broad beach ring wraps a small palmy heart.
        let beach = match isle.terrain {
            IsleKind::Tropical => (h * 0.45).max(2.5),
            _ => (h * 0.06).max(1.4),
        };
        IsleTerrain {
            center: isle.pos,
            radius: r,
            base: 0.58,
            lobes,
            peaks,
            octaves,
            relief,
            beach,
            rings: if tall { 14 } else { 10 },
        }
    }

    /// Shore radius (m) in compass-free local angle `a` (atan2(y, x)).
    #[inline]
    pub fn coast_radius(&self, a: f32) -> f32 {
        let mut s = self.base;
        for &(f, amp, ph) in &self.lobes {
            s += amp * (f * a + ph).sin();
        }
        // Clamp to the grounding circle: bays may cut deep, but a headland tops out
        // flush with `radius` so collision (which keeps the hull outside the shore) is
        // never breached by a spur reaching past it.
        self.radius * s.clamp(0.3, 1.0)
    }

    /// The shore radius (m) toward a world point `p` (its local bearing from centre).
    /// This is the coastline the ship grounds against and the edge features must stay
    /// inside.
    #[inline]
    pub fn coast_radius_toward(&self, p: Vec2) -> f32 {
        let local = p - self.center;
        self.coast_radius(local.y.atan2(local.x))
    }

    /// Is a world point on dry, visible land (inside the coastline) rather than out
    /// over the water in a bay or beyond a headland? `margin` (m) pulls the test in
    /// from the very waterline, so a feature sits clear of the shore, not awash in it.
    #[inline]
    pub fn on_land(&self, p: Vec2, margin: f32) -> bool {
        let local = p - self.center;
        local.length() <= (self.coast_radius(local.y.atan2(local.x)) - margin).max(0.0)
    }

    /// Summed octave noise at a local point, normalised to roughly [-1, 1].
    #[inline]
    fn noise(&self, local: Vec2) -> f32 {
        let mut s = 0.0;
        let mut norm = 0.0;
        for o in &self.octaves {
            let t = (local.x * o.dir.0 + local.y * o.dir.1) * o.freq + o.phase;
            s += o.amp * t.sin();
            norm += o.amp;
        }
        s / norm.max(1e-6)
    }

    /// Surface elevation (m above sea) at a world point, 0 outside the coast.
    #[inline]
    pub fn elevation_at(&self, p: Vec2) -> f32 {
        let local = p - self.center;
        let dist = local.length();
        let a = local.y.atan2(local.x);
        let rc = self.coast_radius(a);
        if dist >= rc {
            return 0.0;
        }
        // Major massifs.
        let mut field = 0.0;
        for pk in &self.peaks {
            let dx = local.x - pk.off.x;
            let dy = local.y - pk.off.y;
            let d2 = dx * dx + dy * dy;
            field += pk.height * (-d2 / (2.0 * pk.sigma * pk.sigma)).exp();
        }
        // Overlapping height-waves fold the slopes into ridges and hollows across
        // the interior, fading out only over the outer band so the coastline stays
        // at sea level rather than the waves punching land out into the water.
        let mut w = ((rc - dist) / (rc * 0.32)).clamp(0.0, 1.0);
        w = w * w * (3.0 - 2.0 * w);
        field += self.noise(local) * self.relief * w;
        let field = field.max(0.0);
        // Smooth fade to sea level over the outer fifth so the shore lies flat.
        let mut edge = ((rc - dist) / (rc * 0.22)).clamp(0.0, 1.0);
        edge = edge * edge * (3.0 - 2.0 * edge);
        field * edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isle(id: i32, terrain: IsleKind) -> Island {
        Island {
            id,
            name: "Test".to_string(),
            pos: Vec2::new(1234.0, -567.0),
            radius: 300.0,
            height: 60.0,
            terrain,
            is_port: false,
            is_shipyard: false,
        }
    }

    /// The coastline never breaches the grounding circle (so collision stays sound)
    /// and never collapses past the inner floor.
    #[test]
    fn coast_stays_within_the_grounding_circle() {
        let t = IsleTerrain::for_island(&isle(3, IsleKind::Rocky));
        for k in 0..720 {
            let rc = t.coast_radius(k as f32 / 720.0 * TAU);
            assert!(rc <= t.radius + 1e-3, "coast {rc} exceeds radius {}", t.radius);
            assert!(rc >= 0.3 * t.radius - 1e-3, "coast {rc} collapsed too far in");
        }
    }

    /// The outline is a lumpy, lobed star (deep bays, jutting headlands), not the old
    /// gently-wobbled disc: across many islands the shore radius swings widely.
    #[test]
    fn outline_is_lumpy_not_a_disc() {
        let mut ratios = Vec::new();
        for id in 0..40 {
            let t = IsleTerrain::for_island(&isle(id, IsleKind::Green));
            let (mut lo, mut hi) = (f32::MAX, 0.0f32);
            for k in 0..360 {
                let rc = t.coast_radius(k as f32 / 360.0 * TAU);
                lo = lo.min(rc);
                hi = hi.max(rc);
            }
            ratios.push(hi / lo);
        }
        let mean = ratios.iter().sum::<f32>() / ratios.len() as f32;
        assert!(mean > 1.8, "coastline too round (mean headland:bay ratio {mean})");
    }

    /// `on_land` tracks the real coast: inside is land, out past a headland is water.
    #[test]
    fn on_land_tracks_the_coastline() {
        let t = IsleTerrain::for_island(&isle(7, IsleKind::Jungle));
        assert!(t.on_land(t.center, 6.0), "the centre is dry land");
        let far = t.center + Vec2::new(t.radius * 1.5, 0.0);
        assert!(!t.on_land(far, 6.0), "well past the biggest headland is open water");
        // Straight along +x the local bearing is 0, so `coast_radius(0)` is the shore.
        let rc = t.coast_radius(0.0);
        assert!(!t.on_land(t.center + Vec2::new(rc + 20.0, 0.0), 6.0), "just past the shore");
        assert!(t.on_land(t.center + Vec2::new(rc * 0.5, 0.0), 6.0), "well inshore");
    }
}
