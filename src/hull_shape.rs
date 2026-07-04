//! Per-tier hull geometry: each shipyard hull tier is a distinct 3-D shape.
//!
//! A [`HullShape`] carries everything that varies when the shipyard rebuilds
//! the ship around the captain: the lofting stations the first-person deck and
//! the exterior miniature are both lofted from (`ship_render`, `rival_render`),
//! where the eye and the helm furniture stand, how the deck cargo stows, and
//! the hull's own *buoyancy probes*: the waterline points [`crate::ocean::ship_motion`]
//! samples the swell at. The probes are what make each hull ride differently:
//! the sway is a least-squares plane fit over them, so a wave much shorter than
//! the probed waterline averages out (a big ship shrugs off chop) while the
//! same chop pitches and rolls a short hull it spans coherently.
//!
//! Tier 0 sails the [`SLOOP`] (a short single-decker); every higher tier sails
//! the [`BRIG`] (the quarterdecked hull, formerly the only shape) until it gets
//! a model of its own. Pick by tier with [`for_level`].

/// One hull's geometry, shared by the renderers and the buoyancy sampling.
/// All lengths are metres in the loft frame: +x starboard, +y up from the
/// waist deck, +z aft, origin on the deck at the mast.
pub struct HullShape {
    /// Lofting stations bow to stern: (z aft of the mast, half-beam, deck
    /// height, bulwark height). This table *is* the ship's shape; everything
    /// drawn is lofted from it. A doubled z (see `qdeck_break`) is the
    /// quarterdeck riser.
    pub stations: &'static [(f32, f32, f32, f32)],
    /// z of the quarterdeck break (the doubled station), or None for a single
    /// flush deck bow to stern (no riser, stairs or breast rail).
    pub qdeck_break: Option<f32>,
    /// The first-person eye: metres abaft the mast and above the waist deck.
    /// The transom lies behind the eye, so the woodwork runs off-screen
    /// through any sway.
    pub cam_aft: f32,
    pub cam_up: f32,
    /// Where the wheel stands, and its hub's height off the local deck. The
    /// chart stand and trinket rack share the helm's station, placed off
    /// `wheel_z`.
    pub wheel_z: f32,
    pub hub_above_deck: f32,
    /// Fore-aft run the deck cargo may stow and slide within: the rising bow
    /// fences the fore end, the quarterdeck riser (or the clear space kept
    /// before the helm) the aft end.
    pub cargo_z_min: f32,
    pub cargo_z_max: f32,
    /// Athwartship stowage columns for the cargo slots, inside the bulwarks
    /// (and clear of the companion stairs where there is a quarterdeck).
    pub cargo_cols: &'static [f32],
    /// Where the sheets and braces belay on the rails (fore-aft z).
    pub sheet_foot_z: f32,
    pub brace_foot_z: f32,
    /// The bowsprit's run, (height above the waist deck, z aft) at heel and tip.
    pub sprit_base: (f32, f32),
    pub sprit_tip: (f32, f32),
    /// Metres the waist deck stands above her waterline, for the exterior
    /// miniature (the first-person loft never shows it).
    pub freeboard: f32,
    /// How briskly the hull answers a change in the water's plane, as a
    /// multiplier on the sway easing rates (see main's ride smoothing). The
    /// probes filter what the waterline *spans*; this is the mass: a light
    /// hull snaps to the sea (beam chop included, which the probes cannot
    /// filter), a heavy one leans into it. The brig's 1.0 is the tuned
    /// baseline feel.
    pub sway_response: f32,
    /// Buoyancy probes: waterline sample offsets (metres fore of the waterline
    /// midpoint, metres to starboard). `ship_motion` fits a plane to the swell
    /// heights at these points; their spread is the hull's wave filter, so a
    /// longer probed waterline rides out short seas a small hull answers.
    /// Athwart offsets are laid out in symmetric pairs (the fit assumes the
    /// starboard offsets sum to zero).
    pub probes: &'static [(f32, f32)],
}

impl HullShape {
    /// Hull data (half-beam, deck height, bulwark height) interpolated at
    /// fore-aft z, for placing furniture and rope feet between stations.
    pub fn station_at(&self, z: f32) -> (f32, f32, f32) {
        for pair in self.stations.windows(2) {
            let (z0, b0, d0, w0) = pair[0];
            let (z1, b1, d1, w1) = pair[1];
            if z >= z0 && z <= z1 && z1 > z0 {
                let t = (z - z0) / (z1 - z0);
                return (b0 + (b1 - b0) * t, d0 + (d1 - d0) * t, w0 + (w1 - w0) * t);
            }
        }
        let (_, b, d, wh) = self.stations[self.stations.len() - 1];
        (b, d, wh)
    }

    /// Fore-aft extent of the loft: the stem tip and the transom.
    pub fn z_bow(&self) -> f32 {
        self.stations[0].0
    }
    pub fn z_stern(&self) -> f32 {
        self.stations[self.stations.len() - 1].0
    }

    /// The waterline midpoint, metres ahead of the eye (`pos` in world terms):
    /// the point the buoyancy probes anchor to.
    pub fn centre_ahead(&self) -> f32 {
        self.cam_aft - (self.z_bow() + self.z_stern()) * 0.5
    }

    /// Half the lofted waterline's length.
    pub fn half_length(&self) -> f32 {
        (self.z_stern() - self.z_bow()) * 0.5
    }

    /// The fullest half-beam in the station table.
    pub fn half_beam(&self) -> f32 {
        self.stations.iter().map(|s| s.1).fold(0.0, f32::max)
    }

    /// Metres ahead of the eye where the stem parts the water: where the bow's
    /// lift is sampled for the deck's heave bob and the frontal slam.
    pub fn bow_reach(&self) -> f32 {
        self.centre_ahead() + self.half_length()
    }
}

/// Tier 0: a short single-decked sloop. One flush deck bow to stern (the
/// helmsman stands on the same planks the cargo rides), a low freeboard, and
/// a probed waterline short enough that the chop a brig ignores tosses her.
pub static SLOOP: HullShape = HullShape {
    stations: &[
        (-10.0, 0.05, 1.05, 0.42), // stem tip
        (-9.0, 0.70, 0.84, 0.52),
        (-7.5, 1.45, 0.60, 0.54),
        (-5.5, 2.05, 0.36, 0.56),
        (-3.0, 2.45, 0.15, 0.57),
        (0.0, 2.60, 0.02, 0.58), // the mast station: full beam
        (2.5, 2.52, 0.00, 0.60),
        (4.8, 2.35, 0.05, 0.64),
        (7.0, 2.00, 0.12, 0.70), // transom, behind the eye
    ],
    qdeck_break: None,
    cam_aft: 5.5,
    cam_up: 1.9, // a helmsman's eye line, stood on the flush deck
    wheel_z: 2.3,
    hub_above_deck: 0.55,
    cargo_z_min: -4.6,
    cargo_z_max: 1.4, // clear water kept before the wheel
    cargo_cols: &[-1.5, -0.5, 0.5, 1.5],
    sheet_foot_z: 2.0,
    brace_foot_z: 4.5,
    sprit_base: (1.0, -9.7),
    sprit_tip: (2.0, -12.5),
    freeboard: 0.95,
    sway_response: 1.7,
    // Rows every couple of metres, close enough that the plane fit acts as the
    // waterplane's true low-pass (sparser rows alias short waves back in);
    // athwart offsets follow the local beam.
    probes: &[
        (8.5, 0.0),
        (6.4, -1.3),
        (6.4, 1.3),
        (4.25, -2.0),
        (4.25, 2.0),
        (2.1, -2.3),
        (2.1, 2.3),
        (0.0, -2.5),
        (0.0, 2.5),
        (-2.1, -2.55),
        (-2.1, 2.55),
        (-4.25, -2.5),
        (-4.25, 2.5),
        (-6.4, -2.35),
        (-6.4, 2.35),
        (-8.5, -2.0),
        (-8.5, 2.0),
    ],
};

/// Tiers 1 and up: the quarterdecked brig (the original hull). The raised
/// quarterdeck aft carries the helm; the waist between mast and break stows
/// the cargo.
pub static BRIG: HullShape = HullShape {
    stations: &[
        (-15.0, 0.05, 1.55, 0.50), // stem tip
        (-13.5, 0.95, 1.22, 0.72),
        (-11.5, 1.95, 0.88, 0.70),
        (-9.0, 2.65, 0.55, 0.68),
        (-6.0, 3.15, 0.26, 0.66),
        (-3.0, 3.40, 0.10, 0.65),
        (0.0, 3.50, 0.02, 0.65), // the mast station: full beam
        (3.0, 3.45, 0.00, 0.66),
        (4.0, 3.40, 0.005, 0.68), // the sheer starts its climb to the quarterdeck...
        (5.0, 3.36, 0.01, 1.67),  // ...topping out level with the platform's wall
        (5.0, 3.36, 1.00, 0.68),  // quarterdeck side of the break (the riser)
        (9.0, 3.05, 1.06, 0.74),
        (11.0, 2.72, 1.10, 0.80), // transom, behind the eye
    ],
    qdeck_break: Some(5.0),
    cam_aft: 10.0,
    cam_up: 2.75, // a helmsman's eye line, stood on the quarterdeck
    wheel_z: 6.6,
    hub_above_deck: 0.41,
    cargo_z_min: -6.5,
    cargo_z_max: 5.0, // the quarterdeck riser
    cargo_cols: &[-2.4, -1.2, 0.0, 1.2], // clear of the stairs at x 2.0
    sheet_foot_z: 3.5,
    brace_foot_z: 6.5,
    sprit_base: (1.5, -14.6),
    sprit_tip: (2.7, -18.2),
    freeboard: 1.3,
    sway_response: 1.0,
    // Same row spacing rule as the sloop's probes; the longer run is what
    // filters the chop she no longer feels.
    probes: &[
        (13.0, 0.0),
        (9.75, -1.9),
        (9.75, 1.9),
        (6.5, -2.75),
        (6.5, 2.75),
        (3.25, -3.2),
        (3.25, 3.2),
        (0.0, -3.45),
        (0.0, 3.45),
        (-3.25, -3.45),
        (-3.25, 3.45),
        (-6.5, -3.4),
        (-6.5, 3.4),
        (-9.75, -3.15),
        (-9.75, 3.15),
        (-13.0, -2.7),
        (-13.0, 2.7),
    ],
};

/// The hull a given shipyard tier sails (see `game_state::upgrades`): tier 0
/// is the sloop; every higher tier keeps the brig until it grows a shape of
/// its own.
pub fn for_level(hull_level: i32) -> &'static HullShape {
    if hull_level <= 0 {
        &SLOOP
    } else {
        &BRIG
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The brig's derived buoyancy dims must keep matching her loft (these
    /// were hand-written constants in `ocean.rs` before the hulls diverged).
    #[test]
    fn brig_probe_frame_matches_the_loft() {
        assert_eq!(BRIG.centre_ahead(), 12.0);
        assert_eq!(BRIG.half_length(), 13.0);
        assert_eq!(BRIG.bow_reach(), 25.0);
    }

    /// Every hull's probes must lie inside its own lofted waterline (a probe
    /// off the hull would answer water the ship never touches), athwart
    /// offsets must cancel (the plane fit assumes it), and the quarterdeck
    /// break must be a real doubled station.
    #[test]
    fn shapes_are_self_consistent() {
        for (name, hull) in [("sloop", &SLOOP), ("brig", &BRIG)] {
            let hl = hull.half_length();
            let sum_x: f32 = hull.probes.iter().map(|p| p.1).sum();
            assert!(sum_x.abs() < 1e-4, "{name}: athwart probes don't cancel");
            for &(a, x) in hull.probes {
                assert!(a.abs() <= hl + 1e-3, "{name}: probe past the stem/stern");
                assert!(x.abs() <= hull.half_beam() + 1e-3, "{name}: probe outboard");
            }
            if let Some(q) = hull.qdeck_break {
                let doubled =
                    hull.stations.windows(2).any(|p| p[0].0 == q && p[1].0 == q);
                assert!(doubled, "{name}: break isn't a doubled station");
            }
            // The eye must stand over the hull, transom behind it.
            assert!(hull.cam_aft < hull.z_stern());
            assert!(hull.cargo_z_min < hull.cargo_z_max);
            for &c in hull.cargo_cols {
                assert!(c.abs() < hull.half_beam(), "{name}: cargo column outboard");
            }
        }
    }
}
