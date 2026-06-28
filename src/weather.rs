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
//! weather than to build. Over a long voyage fair seas dominate (roughly three
//! quarters of the time is spent Calm/Clear/Breezy and barely a tenth in
//! Squall/Storm; see `tests::calm_states_dominate_the_long_run`).
//!
//! Two things lean that bias back toward foul weather so a player is never wholly
//! storm-starved: every **storm-free day** nudges the odds up a notch (a storm
//! resets the count), and the **high sea** well clear of any archipelago is
//! stormier still. Both are framed as a target multiple on how *often* storms
//! occur, converted to the per-rung bias they need (see `effective_calm_bias`).

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
    /// the middle it eases *calmer* with probability `calm_bias`, else builds
    /// stormier. With `calm_bias` above 0.5 fair weather dominates a long voyage;
    /// [`WeatherState`] lowers it as storm-free days pile up and out on the high sea.
    pub fn drift(self, rng: &mut Rng, calm_bias: f64) -> Weather {
        let ord = self.ordinal();
        if ord == 0 {
            self.stormier()
        } else if ord == Self::LADDER.len() - 1 || rng.next_f64() < calm_bias {
            // At the stormy end there is only one way to go; in the middle it eases
            // calmer with probability `calm_bias`. The `||` short-circuits at the end
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

/// The drift steers the per-rung *odds* of building stormier vs. easing calmer
/// (`(1-calm_bias)/calm_bias`). Two leans scale those odds up; calibrated so each
/// alone makes storms land about three times as often (the long-run Storm share, not
/// the per-step odds: a reflecting 6-rung ladder turns a ~1.5x odds nudge into ~3x
/// share; verified in `tests::high_sea_roughly_triples_storm_share`). They multiply,
/// so a storm-starved high-sea crossing is stormier than either alone.
///
/// `STORM_PRESSURE_ODDS_MULT` is the lean reached after `STORM_PRESSURE_DAYS_CAP`
/// storm-free days (ramped in linearly, a day at a time; a storm resets the count,
/// see [`WeatherState::note_new_day`]). `HIGH_SEA_ODDS_MULT` is the open-water lean,
/// applied whenever the caller flags the ship as out past the isles.
const STORM_PRESSURE_ODDS_MULT: f64 = 1.54;
const STORM_PRESSURE_DAYS_CAP: f32 = 8.0;
const HIGH_SEA_ODDS_MULT: f64 = 1.54;

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
    /// Consecutive storm-free days, capped at [`STORM_PRESSURE_DAYS_CAP`]. Each one
    /// leans the drift a little stormier; reaching a Storm zeroes it back out.
    storm_free_days: f32,
}

impl WeatherState {
    /// How often the weather drifts to an adjacent scenario (seconds), before the
    /// per-step jitter. Long enough that a scenario settles in before it shifts.
    const BASE_PERIOD: f32 = 130.0;
    /// How fast `sea`/`gloom` chase their scenario targets (per second). Gentle, so
    /// the sea takes a few seconds to build or lay down across a drift.
    const EASE: f32 = 0.5;

    /// Open on `start`, with the drift RNG seeded off `seed`. The drift is otherwise
    /// reproducible from the seed, but the storm-free-day lean and the high-sea boost
    /// depend on the voyage (how long since a storm, where the ship is), so the same
    /// chart can weather differently on different runs. `sea`/`gloom` begin settled.
    pub fn new(start: Weather, seed: i64) -> WeatherState {
        WeatherState {
            weather: start,
            rng: Rng::from_seed(seed),
            since_change: 0.0,
            period: Self::BASE_PERIOD,
            sea: start.sea_state(),
            gloom: start.gloom(),
            storm_free_days: 0.0,
        }
    }

    /// The drift's calm bias for this tick: [`CALM_BIAS`] leaned stormier by the
    /// storm-free days banked so far and, when `high_sea`, by the open-water boost.
    /// Both scale the per-rung storm odds (see [`STORM_PRESSURE_ODDS_MULT`]); the day
    /// lean ramps in linearly to its cap, and the two multiply.
    fn effective_calm_bias(&self, high_sea: bool) -> f64 {
        // Base per-rung odds of building stormier rather than easing calmer.
        let base_r = (1.0 - CALM_BIAS) / CALM_BIAS;
        let day_frac = (self.storm_free_days / STORM_PRESSURE_DAYS_CAP).min(1.0) as f64;
        let pressure_mult = 1.0 + (STORM_PRESSURE_ODDS_MULT - 1.0) * day_frac;
        let high_mult = if high_sea { HIGH_SEA_ODDS_MULT } else { 1.0 };
        let r = base_r * pressure_mult * high_mult;
        1.0 / (1.0 + r)
    }

    /// Advance the drift timer and ease `sea`/`gloom` toward the current scenario.
    /// `high_sea` is set when the ship is well clear of any archipelago, where storms
    /// build likelier (the caller owns where that open water begins).
    pub fn update(&mut self, dt: f32, high_sea: bool) {
        self.since_change += dt;
        if self.since_change >= self.period {
            let calm_bias = self.effective_calm_bias(high_sea);
            self.weather = self.weather.drift(&mut self.rng, calm_bias);
            // A storm spends the pressure that built toward it: the count starts over.
            if self.weather == Weather::Storm {
                self.storm_free_days = 0.0;
            }
            self.since_change = 0.0;
            // Jitter the next interval ±25% so drifts never fall on a metronome.
            self.period = Self::BASE_PERIOD * self.rng.between(0.75, 1.25) as f32;
        }
        let k = clamp(Self::EASE * dt, 0.0, 1.0);
        self.sea += (self.weather.sea_state() - self.sea) * k;
        self.gloom += (self.weather.gloom() - self.gloom) * k;
    }

    /// A fresh day has broken with no storm yet: bank another storm-free day so the
    /// drift leans a touch stormier, up to [`STORM_PRESSURE_DAYS_CAP`]. A storm clears
    /// the tally (see [`update`](Self::update)).
    pub fn note_new_day(&mut self) {
        self.storm_free_days = (self.storm_free_days + 1.0).min(STORM_PRESSURE_DAYS_CAP);
    }

    /// The storm/fury scalar `[0, 1]` the renderers blend on, from the eased gloom.
    pub fn fury(&self) -> f32 {
        fury(self.gloom)
    }

    /// A read-only snapshot of the sim's inner state for the logbook's weather debug
    /// page (see [`crate::captains_log`]): the live scenario, the eased-vs-target sea
    /// and gloom it drives, the drift timer, and the modifiers leaning the next drift.
    /// `high_sea` is the caller's open-water flag for this tick, the same one passed to
    /// [`update`](Self::update).
    pub fn debug(&self, high_sea: bool) -> WeatherDebug {
        // Mirror the lean maths of `effective_calm_bias` so the page can show each
        // multiplier on its own rather than only the folded-in result.
        let day_frac = (self.storm_free_days / STORM_PRESSURE_DAYS_CAP).min(1.0) as f64;
        let pressure_mult = 1.0 + (STORM_PRESSURE_ODDS_MULT - 1.0) * day_frac;
        let high_mult = if high_sea { HIGH_SEA_ODDS_MULT } else { 1.0 };
        WeatherDebug {
            weather: self.weather,
            sea: self.sea,
            sea_target: self.weather.sea_state(),
            gloom: self.gloom,
            gloom_target: self.weather.gloom(),
            fury: self.fury(),
            since_change: self.since_change,
            period: self.period,
            storm_free_days: self.storm_free_days,
            storm_free_cap: STORM_PRESSURE_DAYS_CAP,
            high_sea,
            base_calm_bias: CALM_BIAS,
            effective_calm_bias: self.effective_calm_bias(high_sea),
            pressure_mult,
            high_mult,
        }
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

    /// Calm the seas (the Storm Glass tavern ware): jump the scenario to the flattest
    /// `Calm` and reset the drift timer. The eased `sea`/`gloom` lay down to it over
    /// the next few seconds rather than snapping, so the gale visibly subsides.
    pub fn calm(&mut self) {
        self.weather = Weather::Calm;
        self.since_change = 0.0;
    }
}

/// A read-only snapshot of [`WeatherState`] for the logbook's weather debug page:
/// the live, eased state and the modifiers leaning the next drift. Built by
/// [`WeatherState::debug`].
pub struct WeatherDebug {
    pub weather: Weather,
    /// Eased sea-state handed to the ocean, and the scenario target it's chasing.
    pub sea: f32,
    pub sea_target: f32,
    /// Eased sky gloom, and the scenario target it's chasing.
    pub gloom: f32,
    pub gloom_target: f32,
    /// Storm/fury blend off the eased gloom.
    pub fury: f32,
    /// Seconds the current scenario has held, and the jittered interval before the
    /// next drift is rolled.
    pub since_change: f32,
    pub period: f32,
    /// Storm-free days banked toward the storm-pressure lean, and the cap they
    /// saturate at.
    pub storm_free_days: f32,
    pub storm_free_cap: f32,
    /// Whether the caller flagged the ship out on the high sea this tick.
    pub high_sea: bool,
    /// The plain [`CALM_BIAS`] and the effective bias after both leans fold in.
    pub base_calm_bias: f64,
    pub effective_calm_bias: f64,
    /// The two odds multipliers folded into the effective bias: the storm-pressure
    /// lean (from the storm-free days) and the high-sea lean.
    pub pressure_mult: f64,
    pub high_mult: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_drift_inward() {
        let mut rng = Rng::from_seed(1);
        assert_eq!(Weather::Calm.drift(&mut rng, CALM_BIAS), Weather::Clear);
        assert_eq!(Weather::Storm.drift(&mut rng, CALM_BIAS), Weather::Squall);
    }

    #[test]
    fn drift_only_ever_steps_to_a_neighbour() {
        let mut rng = Rng::from_seed(7);
        let mut w = Weather::Breezy;
        for _ in 0..10_000 {
            let next = w.drift(&mut rng, CALM_BIAS);
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
                ws.update(dt, false);
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
    fn storm_free_days_and_high_sea_lean_stormier() {
        let ws = WeatherState::new(Weather::Clear, 1);
        // Fresh, in harbour waters: the drift runs on the plain calm bias.
        assert!((ws.effective_calm_bias(false) - CALM_BIAS).abs() < 1e-9);
        // Both the open sea and a fresh storm-free day pull the bias toward storm.
        assert!(ws.effective_calm_bias(true) < ws.effective_calm_bias(false));
        let mut banked = WeatherState::new(Weather::Clear, 1);
        banked.note_new_day();
        assert!(banked.effective_calm_bias(false) < ws.effective_calm_bias(false));
        // The day count saturates at the cap and never overshoots it.
        for _ in 0..50 {
            banked.note_new_day();
        }
        assert!((banked.storm_free_days - STORM_PRESSURE_DAYS_CAP).abs() < 1e-6);
    }

    #[test]
    fn high_sea_roughly_triples_storm_share() {
        // Run a long drift chain at each bias and count how much of it sits in Storm.
        // The open-sea bias targets ~3x the storm share; allow a wide band around it
        // (a 6-rung chain is noisy) but pin that it lands clearly above the harbour run.
        fn storm_share(calm_bias: f64) -> f64 {
            let mut rng = Rng::from_seed(99);
            let mut w = Weather::Clear;
            let (mut storm, mut total) = (0u64, 0u64);
            for _ in 0..2_000_000 {
                if w == Weather::Storm {
                    storm += 1;
                }
                total += 1;
                w = w.drift(&mut rng, calm_bias);
            }
            storm as f64 / total as f64
        }
        let harbour = WeatherState::new(Weather::Clear, 1).effective_calm_bias(false);
        let open = WeatherState::new(Weather::Clear, 1).effective_calm_bias(true);
        let calm = storm_share(harbour);
        let rough = storm_share(open);
        let ratio = rough / calm;
        assert!((2.0..4.5).contains(&ratio), "calm={calm} high_sea={rough} ratio={ratio}");
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
            w = w.drift(&mut rng, CALM_BIAS);
        }
        assert!(fair > foul * 4, "fair={fair} foul={foul}");
    }
}
