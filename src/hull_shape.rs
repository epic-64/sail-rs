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
//! Tier 0 sails the [`SLOOP`] (a short single-decker), tier 1 the [`BRIG`]
//! (the quarterdecked hull, formerly the only shape), tier 2 the [`GALLEON`]
//! (the high-charged two-master), and the top tier the [`INDIAMAN`] (the
//! three-masted flagship). Pick by tier with [`for_level`].

/// One mast of a hull's rig: where it stands, how large its course is cut,
/// how much canvas it flies, and where its running rigging belays. The
/// renderers loft every spar and cloth dimension from their shared rig
/// constants (`ship_render`'s `MAST_TOP_M` and friends) multiplied by
/// `scale`, so a hull's masts differ in size without touching the rig code.
pub struct Mast {
    /// Fore-aft station of the mast foot (the trunk stands on the local deck).
    pub z: f32,
    /// Rig scale on the shared dimensions: mast height, yard span, cloth.
    pub scale: f32,
    /// Extra width factor on the canvas and its yards alone (1.0 flies the
    /// shared cut): a hull can spread broader cloth without a taller mast.
    pub cloth_w: f32,
    /// Metres (in the shared rig's design space, so scaled like the rest) the
    /// course yard hangs below its shared height, for a hull that flies its
    /// sail low on the same mast.
    pub yard_drop: f32,
    /// Extra pole above the shared masthead (design-space metres, like
    /// `yard_drop`), for a mast rigged taller without recutting its canvas.
    pub mast_up: f32,
    /// Metres both yards are hung above their shared heights; the canvas
    /// keeps its cut and rides up with them (`yard_drop` still lowers the
    /// course against this).
    pub yard_up: f32,
    /// Extra daylight opened between the course's head and the topsail's
    /// foot, taken by hoisting the topsail yard alone.
    pub sail_gap: f32,
    /// Square sails flown, stacked course upward: 1 is the course alone, 2
    /// adds a topsail on the pole above it (cut from `ship_render`'s shared
    /// topsail constants, again by `scale`).
    pub sails: usize,
    /// Where this mast's sheets and braces belay on the rails (fore-aft z).
    pub sheet_foot_z: f32,
    pub brace_foot_z: f32,
}

/// One hull's geometry, shared by the renderers and the buoyancy sampling.
/// All lengths are metres in the loft frame: +x starboard, +y up from the
/// waist deck, +z aft, origin on the deck at the main mast.
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
    /// The rig, bow to stern; the aftmost mast is the main (it flies the
    /// pennant, and its rig is cut at scale 1.0). The renderers draw the
    /// masts in table order, which from the helm's aft eye is far to near.
    pub masts: &'static [Mast],
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
        (7.0, 2.00, 0.12, 0.70),
        (9.0, 1.60, 0.20, 0.76), // transom, behind the eye
    ],
    qdeck_break: None,
    cam_aft: 7.5, // stood well aft, so the mast doesn't crowd the view ahead
    cam_up: 1.9,  // a helmsman's eye line, stood on the flush deck
    wheel_z: 4.3,
    hub_above_deck: 0.55,
    cargo_z_min: -4.6,
    cargo_z_max: 3.4, // clear water kept before the wheel
    cargo_cols: &[-1.5, -0.5, 0.5, 1.5],
    // The single course hangs low on its mast: a workboat's cut, and it keeps
    // the cloth's foot near the helmsman's eye line on the flush deck.
    masts: &[Mast {
        z: 0.0,
        scale: 1.0,
        cloth_w: 1.0,
        yard_drop: 1.6,
        mast_up: 0.0,
        yard_up: 0.0,
        sail_gap: 0.0,
        sails: 1,
        sheet_foot_z: 2.0,
        brace_foot_z: 4.5,
    }],
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
/// the cargo. Her mast flies a topsail over the course, the first rung up
/// from the sloop's single sail.
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
    masts: &[Mast {
        z: 0.0,
        scale: 1.0,
        cloth_w: 1.0,
        yard_drop: 0.0,
        mast_up: 0.0,
        yard_up: 0.0,
        sail_gap: 0.0,
        sails: 2,
        sheet_foot_z: 3.5,
        brace_foot_z: 6.5,
    }],
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

/// Tier 2: the galleon, the yard's only two-master. A third again the brig's
/// length with a beam to match, a deep freeboard, and a taller quarterdeck
/// set further aft (the classic high stern, with the counter carried out over
/// the transom): from her wheel you look down into a long waist that stows a
/// fifth cargo column, past a smaller foremast riding the bow. A probed
/// waterline well past the brig's plus a slower sway response make her ride
/// the same seas the smaller hulls answer: stately, leaning into the swell
/// rather than snapping to it.
pub static GALLEON: HullShape = HullShape {
    stations: &[
        (-20.0, 0.05, 2.00, 0.55), // stem tip
        (-18.0, 1.15, 1.58, 0.80),
        (-15.5, 2.40, 1.14, 0.78),
        (-12.5, 3.30, 0.72, 0.75),
        (-8.5, 3.95, 0.34, 0.72),
        (-4.0, 4.30, 0.12, 0.70),
        (0.0, 4.40, 0.02, 0.70), // the mast station: full beam
        (3.5, 4.36, 0.00, 0.71),
        (5.5, 4.30, 0.005, 0.73), // the sheer starts its climb to the quarterdeck...
        (7.0, 4.24, 0.01, 2.07),  // ...topping out level with the platform's wall
        (7.0, 4.24, 1.35, 0.73),  // quarterdeck side of the break (the riser)
        (11.0, 3.85, 1.42, 0.80),
        (13.5, 3.45, 1.47, 0.88),
        (15.0, 3.10, 1.50, 0.95), // transom, behind the eye
    ],
    qdeck_break: Some(7.0),
    cam_aft: 12.5,
    cam_up: 3.2, // a helmsman's eye line, stood on the taller quarterdeck
    wheel_z: 8.8,
    hub_above_deck: 0.41,
    cargo_z_min: -8.0,
    cargo_z_max: 7.0, // the quarterdeck riser
    cargo_cols: &[-3.2, -2.0, -0.8, 0.4, 1.6], // the wider beam buys a fifth column
    // Two-masted: a smaller foremast on the rising foredeck (clear forward of
    // the cargo run), the full-rigged main at the origin; both fly a topsail
    // over the course, cut broader than the shared plan (the big hulls spread
    // wider cloth without a taller rig).
    masts: &[
        Mast {
            z: -9.5,
            scale: 0.78,
            cloth_w: 1.12,
            yard_drop: 0.0,
            mast_up: 0.0,
            yard_up: 0.0,
            sail_gap: 0.0,
            sails: 2,
            sheet_foot_z: -4.8,
            brace_foot_z: -2.5,
        },
        Mast {
            z: 0.0,
            scale: 1.0,
            cloth_w: 1.12,
            yard_drop: 0.0,
            mast_up: 0.0,
            yard_up: 0.0,
            sail_gap: 0.0,
            sails: 2,
            sheet_foot_z: 4.5,
            brace_foot_z: 9.0,
        },
    ],
    sprit_base: (1.95, -19.4),
    sprit_tip: (3.5, -24.0),
    freeboard: 1.75,
    sway_response: 0.75,
    // Same row spacing rule as the smaller hulls; the yard's longest probed
    // waterline filters swell that still works the brig.
    probes: &[
        (17.5, 0.0),
        (14.6, -1.55),
        (14.6, 1.55),
        (11.7, -2.7),
        (11.7, 2.7),
        (8.75, -3.4),
        (8.75, 3.4),
        (5.8, -3.85),
        (5.8, 3.85),
        (2.9, -4.05),
        (2.9, 4.05),
        (0.0, -4.2),
        (0.0, 4.2),
        (-2.9, -4.25),
        (-2.9, 4.25),
        (-5.8, -4.2),
        (-5.8, 4.2),
        (-8.75, -4.1),
        (-8.75, 4.1),
        (-11.7, -3.9),
        (-11.7, 3.9),
        (-14.6, -3.5),
        (-14.6, 3.5),
        (-17.5, -2.95),
        (-17.5, 2.95),
    ],
};

/// The top tier: the East Indiaman, the yard's flagship and its only
/// three-master. Longer again than the galleon with the deepest freeboard and
/// the highest quarterdeck afloat, and a beam that stows a sixth cargo
/// column. Her rig steps up bow to stern: a small foremast on the rising bow,
/// a taller mast amidships, and the full-rigged main aft at the origin, every
/// one flying a topsail over its course, the most canvas in the yard. From
/// the helm the three tiers of cloth recede down the deck. The yard's longest
/// probed waterline and its slowest sway response: she shrugs off seas that
/// still work the galleon, and leans where lesser hulls snap.
pub static INDIAMAN: HullShape = HullShape {
    stations: &[
        (-24.0, 0.05, 2.30, 0.60), // stem tip
        (-21.5, 1.30, 1.82, 0.85),
        (-18.5, 2.70, 1.32, 0.83),
        (-15.0, 3.80, 0.85, 0.80),
        (-10.0, 4.60, 0.40, 0.76),
        (-5.0, 5.00, 0.14, 0.74),
        (0.0, 5.10, 0.02, 0.74), // the main mast station: full beam
        (4.0, 5.05, 0.00, 0.75),
        (6.5, 4.98, 0.005, 0.77), // the sheer starts its climb to the quarterdeck...
        (8.5, 4.90, 0.01, 2.46),  // ...topping out level with the platform's wall
        (8.5, 4.90, 1.70, 0.77),  // quarterdeck side of the break (the riser)
        (12.5, 4.45, 1.78, 0.85),
        (15.5, 3.95, 1.84, 0.94),
        (18.0, 3.50, 1.88, 1.02), // transom, behind the eye
    ],
    qdeck_break: Some(8.5),
    cam_aft: 14.5,
    cam_up: 3.55, // a helmsman's eye line, stood on the highest quarterdeck
    wheel_z: 10.5,
    hub_above_deck: 0.41,
    cargo_z_min: -9.5,
    cargo_z_max: 8.5, // the quarterdeck riser
    cargo_cols: &[-3.7, -2.5, -1.3, -0.1, 1.1, 2.3], // the widest beam buys a sixth column
    // Three-masted, the rig stepping up bow to stern to the full-rigged main,
    // every cloth cut broader than the shared plan (like the galleon's); the
    // fore and middle masts stand clear enough that no course sweeps its
    // neighbour's canvas at any brace.
    masts: &[
        Mast {
            z: -16.0,
            scale: 0.72,
            cloth_w: 1.12,
            yard_drop: 0.0,
            mast_up: 0.0,
            yard_up: 0.0,
            sail_gap: 0.0,
            sails: 2,
            sheet_foot_z: -11.3,
            brace_foot_z: -9.0,
        },
        Mast {
            z: -8.0,
            scale: 0.86,
            cloth_w: 1.12,
            yard_drop: 0.0,
            mast_up: 0.0,
            yard_up: 0.0,
            sail_gap: 0.0,
            sails: 2,
            sheet_foot_z: -3.3,
            brace_foot_z: -1.0,
        },
        // The flagship's main is rigged above the shared plan: a taller pole,
        // both yards hoisted higher, and extra daylight between the cloths;
        // the canvas itself keeps the shared cut.
        Mast {
            z: 0.0,
            scale: 1.0,
            cloth_w: 1.12,
            yard_drop: 0.0,
            mast_up: 1.2,
            yard_up: 0.7,
            sail_gap: 0.2,
            sails: 2,
            sheet_foot_z: 5.0,
            brace_foot_z: 10.0,
        },
    ],
    sprit_base: (2.25, -23.2),
    sprit_tip: (4.0, -29.0),
    freeboard: 2.1,
    sway_response: 0.6,
    // Same row spacing rule as the smaller hulls; the yard's longest probed
    // waterline filters chop that still works the galleon.
    probes: &[
        (21.0, 0.0),
        (18.0, -1.45),
        (18.0, 1.45),
        (15.0, -2.7),
        (15.0, 2.7),
        (12.0, -3.6),
        (12.0, 3.6),
        (9.0, -4.1),
        (9.0, 4.1),
        (6.0, -4.5),
        (6.0, 4.5),
        (3.0, -4.75),
        (3.0, 4.75),
        (0.0, -4.9),
        (0.0, 4.9),
        (-3.0, -4.95),
        (-3.0, 4.95),
        (-6.0, -4.9),
        (-6.0, 4.9),
        (-9.0, -4.85),
        (-9.0, 4.85),
        (-12.0, -4.7),
        (-12.0, 4.7),
        (-15.0, -4.35),
        (-15.0, 4.35),
        (-18.0, -3.85),
        (-18.0, 3.85),
        (-21.0, -3.3),
        (-21.0, 3.3),
    ],
};

/// The hull a given shipyard tier sails (see `game_state::upgrades`): tier 0
/// is the sloop, tier 1 the brig, tier 2 the galleon, and the top tier the
/// indiaman.
pub fn for_level(hull_level: i32) -> &'static HullShape {
    if hull_level <= 0 {
        &SLOOP
    } else if hull_level == 1 {
        &BRIG
    } else if hull_level == 2 {
        &GALLEON
    } else {
        &INDIAMAN
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

    /// The canvas each tier's hull flies: the sloop a single course, the brig
    /// a topsail over hers, the galleon and the indiaman a topsail over the
    /// course on every mast, and the mast count climbing with the tier.
    #[test]
    fn tier_rigs() {
        assert!(SLOOP.masts.iter().all(|m| m.sails == 1));
        assert!(BRIG.masts.iter().all(|m| m.sails == 2));
        assert!(GALLEON.masts.iter().all(|m| m.sails == 2));
        assert!(INDIAMAN.masts.iter().all(|m| m.sails == 2));
        assert_eq!(GALLEON.masts.len(), 2);
        assert_eq!(INDIAMAN.masts.len(), 3);
    }

    /// Every hull's probes must lie inside its own lofted waterline (a probe
    /// off the hull would answer water the ship never touches), athwart
    /// offsets must cancel (the plane fit assumes it), and the quarterdeck
    /// break must be a real doubled station.
    #[test]
    fn shapes_are_self_consistent() {
        for (name, hull) in
            [("sloop", &SLOOP), ("brig", &BRIG), ("galleon", &GALLEON), ("indiaman", &INDIAMAN)]
        {
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
            // The rig: masts bow to stern on the loft, the aftmost the
            // full-scale main, every belay point on the hull.
            assert!(!hull.masts.is_empty(), "{name}: no masts");
            let main = hull.masts.last().unwrap();
            assert_eq!(main.scale, 1.0, "{name}: the main isn't full scale");
            for pair in hull.masts.windows(2) {
                assert!(pair[0].z < pair[1].z, "{name}: masts out of bow-to-stern order");
            }
            for m in hull.masts {
                assert!(m.sails >= 1, "{name}: a bare mast");
                assert!(m.cloth_w > 0.0, "{name}: no cloth width");
                assert!(m.yard_drop >= 0.0, "{name}: yard_drop hangs downward");
                assert!(m.mast_up >= 0.0, "{name}: mast_up rigs taller");
                assert!(m.yard_up >= 0.0, "{name}: yard_up hoists upward");
                assert!(m.sail_gap >= 0.0, "{name}: sail_gap opens upward");
                for z in [m.z, m.sheet_foot_z, m.brace_foot_z] {
                    assert!(
                        z > hull.z_bow() && z < hull.z_stern(),
                        "{name}: mast or belay off the loft"
                    );
                }
            }
        }
    }
}
