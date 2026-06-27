//! Automatic sea-state weather, ported from `shared.Weather`.
//!
//! Six scenarios sit on a single calm→storm ladder. Each carries the **sea-state**
//! scalar it drives (wave height + how hard the deck rolls — see [`crate::ocean`])
//! and a **gloom** in `[0, 1]` for how grey it turns the sky (which the storm/fury
//! blend reads off). The weather only ever drifts to an *adjacent* scenario, so it
//! builds and eases through the middle states rather than leaping calm→gale.
//!
//! Unlike the original's even coin-flip drift, ours is **biased toward calm**
//! (`CALM_BIAS`): from any middle scenario it is likelier to ease back toward fair
//! weather than to build. Over a long voyage fair seas dominate — roughly three
//! quarters of the time is spent Calm/Clear/Breezy and barely a tenth in
//! Squall/Storm (see `tests::calm_states_dominate_the_long_run`).

use crate::geometry::clamp;
use crate::rng::Rng;

/// A weather scenario, ordered calm→storm. Order *is* the ladder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weather {
    Calm,
    Clear,
    Breezy,
    Choppy,
    Squall,
    Storm,
}

impl Weather {
    /// The calm→storm ladder; neighbours on it are the only reachable drifts.
    pub const LADDER: [Weather; 6] = [
        Weather::Calm,
        Weather::Clear,
        Weather::Breezy,
        Weather::Choppy,
        Weather::Squall,
        Weather::Storm,
    ];

    /// Where this scenario sits on the ladder (0 = calmest).
    pub fn ordinal(self) -> usize {
        Self::LADDER.iter().position(|&w| w == self).unwrap()
    }

    /// Short HUD/log label.
    pub fn label(self) -> &'static str {
        match self {
            Weather::Calm => "Calm",
            Weather::Clear => "Clear",
            Weather::Breezy => "Breezy",
            Weather::Choppy => "Choppy",
            Weather::Squall => "Squall",
            Weather::Storm => "Storm",
        }
    }

    /// The sea-state scalar this scenario drives (wave height + deck roll).
    pub fn sea_state(self) -> f32 {
        match self {
            Weather::Calm => 0.15,
            Weather::Clear => 0.35,
            Weather::Breezy => 0.55,
            Weather::Choppy => 0.75,
            Weather::Squall => 1.05,
            Weather::Storm => 1.30,
        }
    }

    /// How grey it turns the sky, in `[0, 1]`; the storm/fury blend reads off this.
    pub fn gloom(self) -> f32 {
        match self {
            Weather::Calm => 0.00,
            Weather::Clear => 0.00,
            Weather::Breezy => 0.08,
            Weather::Choppy => 0.22,
            Weather::Squall => 0.38,
            Weather::Storm => 0.62,
        }
    }

    /// One step calmer (clamped at `Calm`).
    pub fn calmer(self) -> Weather {
        Self::LADDER[self.ordinal().saturating_sub(1)]
    }

    /// One step stormier (clamped at `Storm`).
    pub fn stormier(self) -> Weather {
        Self::LADDER[(self.ordinal() + 1).min(Self::LADDER.len() - 1)]
    }

    /// Drift to an adjacent scenario. At the ends there is only one way to go; in
    /// the middle it eases *calmer* with probability `CALM_BIAS`, else builds
    /// stormier — so fair weather dominates a long voyage.
    pub fn drift(self, rng: &mut Rng) -> Weather {
        let ord = self.ordinal();
        if ord == 0 {
            self.stormier()
        } else if ord == Self::LADDER.len() - 1 || rng.next_f64() < CALM_BIAS {
            // At the stormy end there is only one way to go; in the middle it eases
            // calmer with probability `CALM_BIAS`. The `||` short-circuits at the end
            // so the rng draw still happens only on the middle rungs (draw order intact).
            self.calmer()
        } else {
            self.stormier()
        }
    }
}

/// Probability a *middle* scenario eases calmer on a drift (vs. building stormier).
/// Above 0.5 so the stationary distribution piles up on the fair-weather end:
/// roughly Calm 21% / Clear 34% / Breezy 21% / Choppy 13% / Squall 8% / Storm 3%.
pub const CALM_BIAS: f64 = 0.62;

/// The gloom below which the sky is still fair — the storm/fury blend stays nil
/// until the weather has some grey to it. Mirrors `SailingView.fury`'s offset.
const GLOOM_FLOOR: f32 = 0.18;

/// The storm/fury scalar `[0, 1]` for a given (eased) `gloom`: nil until the sky
/// greys past `GLOOM_FLOOR`, then ramping to full at the storm's gloom. Drives the
/// storm palette/sky/audio blend. `SailingView.fury`.
pub fn fury(gloom: f32) -> f32 {
    clamp(
        (gloom - GLOOM_FLOOR) / (Weather::Storm.gloom() - GLOOM_FLOOR),
        0.0,
        1.0,
    )
}

/// The live, eased weather for the voyage: the current scenario, the auto-drift
/// timer, and the smoothly-eased `sea` and `gloom` it drives — so the waves and sky
/// transition *across* a drift instead of snapping to the new scenario's values.
pub struct WeatherState {
    pub weather: Weather,
    rng: Rng,
    since_change: f32,
    period: f32,
    /// Eased sea-state scalar handed to [`crate::ocean`] (wave height + deck roll).
    pub sea: f32,
    /// Eased sky gloom; [`fury`] reads off it for the storm blend.
    pub gloom: f32,
}

impl WeatherState {
    /// How often the weather drifts to an adjacent scenario (seconds), before the
    /// per-step jitter. Long enough that a scenario settles in before it shifts.
    const BASE_PERIOD: f32 = 130.0;
    /// How fast `sea`/`gloom` chase their scenario targets (per second). Gentle, so
    /// the sea takes a few seconds to build or lay down across a drift.
    const EASE: f32 = 0.5;

    /// Open on `start`, with the drift RNG seeded off `seed` (so a chart's weather
    /// is reproducible). `sea`/`gloom` begin settled on the opening scenario.
    pub fn new(start: Weather, seed: i64) -> WeatherState {
        WeatherState {
            weather: start,
            rng: Rng::from_seed(seed),
            since_change: 0.0,
            period: Self::BASE_PERIOD,
            sea: start.sea_state(),
            gloom: start.gloom(),
        }
    }

    /// Advance the drift timer and ease `sea`/`gloom` toward the current scenario.
    pub fn update(&mut self, dt: f32) {
        self.since_change += dt;
        if self.since_change >= self.period {
            self.weather = self.weather.drift(&mut self.rng);
            self.since_change = 0.0;
            // Jitter the next interval ±25% so drifts never fall on a metronome.
            self.period = Self::BASE_PERIOD * self.rng.between(0.75, 1.25) as f32;
        }
        let k = clamp(Self::EASE * dt, 0.0, 1.0);
        self.sea += (self.weather.sea_state() - self.sea) * k;
        self.gloom += (self.weather.gloom() - self.gloom) * k;
    }

    /// The storm/fury scalar `[0, 1]` the renderers blend on, from the eased gloom.
    pub fn fury(&self) -> f32 {
        fury(self.gloom)
    }

    /// Force the scenario a step calmer (dev aid); resets the drift timer.
    pub fn nudge_calmer(&mut self) {
        self.weather = self.weather.calmer();
        self.since_change = 0.0;
    }

    /// Force the scenario a step stormier (dev aid); resets the drift timer.
    pub fn nudge_stormier(&mut self) {
        self.weather = self.weather.stormier();
        self.since_change = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_drift_inward() {
        let mut rng = Rng::from_seed(1);
        assert_eq!(Weather::Calm.drift(&mut rng), Weather::Clear);
        assert_eq!(Weather::Storm.drift(&mut rng), Weather::Squall);
    }

    #[test]
    fn drift_only_ever_steps_to_a_neighbour() {
        let mut rng = Rng::from_seed(7);
        let mut w = Weather::Breezy;
        for _ in 0..10_000 {
            let next = w.drift(&mut rng);
            assert_eq!((next.ordinal() as i32 - w.ordinal() as i32).abs(), 1);
            w = next;
        }
    }

    #[test]
    fn probe_time_to_first_storm() {
        // Drive the real WeatherState exactly as the game does (fixed 1/60 s steps,
        // the in-game seed derivation) and report the simulated time the first Storm
        // arrives, for a spread of world seeds. Also confirms it's deterministic: the
        // same seed yields the identical time twice.
        fn time_to_storm(world_seed: i64) -> Option<f32> {
            let mut ws = WeatherState::new(Weather::Clear, world_seed ^ 0x57e4_c107);
            let dt = 1.0 / 60.0;
            let mut t = 0.0f32;
            // Cap the walk at ~8 in-game hours so a storm-shy seed can't loop forever.
            while t < 8.0 * 3600.0 {
                ws.update(dt);
                t += dt;
                if ws.weather == Weather::Storm {
                    return Some(t);
                }
            }
            None
        }
        for seed in [1i64, 2, 7, 42, 1234] {
            let a = time_to_storm(seed);
            let b = time_to_storm(seed);
            assert_eq!(a, b, "seed {seed} must be deterministic");
            match a {
                Some(t) => println!("seed {seed}: first Storm at {:.0} s ({:.1} min)", t, t / 60.0),
                None => println!("seed {seed}: no Storm within 8 h"),
            }
        }
    }

    #[test]
    fn calm_states_dominate_the_long_run() {
        // Walk a long chain and count time spent fair (Calm/Clear/Breezy) vs. foul
        // (Squall/Storm). The calm bias should make fair far the more common.
        let mut rng = Rng::from_seed(42);
        let mut w = Weather::Clear;
        let (mut fair, mut foul) = (0usize, 0usize);
        for _ in 0..200_000 {
            match w {
                Weather::Calm | Weather::Clear | Weather::Breezy => fair += 1,
                Weather::Squall | Weather::Storm => foul += 1,
                Weather::Choppy => {}
            }
            w = w.drift(&mut rng);
        }
        assert!(fair > foul * 4, "fair={fair} foul={foul}");
    }
}
