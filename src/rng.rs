//! A tiny deterministic PRNG (SplitMix64), ported from `shared.Rng`.
//!
//! The Scala original is immutable (each draw returns the value *and* the next
//! generator). Here it's a mutable struct whose methods advance `state` — the
//! draw *sequence* is identical, so the same seed rebuilds the same world.

const GOLDEN: u64 = 0x9e3779b97f4a7c15;
const C1: u64 = 0xbf58476d1ce4e5b9;
const C2: u64 = 0x94d049bb133111eb;
const SEED_MIX: u64 = 0x2545f4914f6cdd1d;

#[derive(Clone, Copy, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the generator, mixing the seed so even small seeds spread out.
    pub fn from_seed(seed: i64) -> Rng {
        Rng {
            state: (seed as u64).wrapping_mul(SEED_MIX),
        }
    }

    /// The raw generator state, for persisting a sequence mid-stream (the wind
    /// schedule rides the voyage save); resume it with [`Rng::from_state`].
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Rebuild a generator exactly where a [`Rng::state`] capture left it.
    pub fn from_state(state: u64) -> Rng {
        Rng { state }
    }

    /// Advance once, returning a fresh 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(C1);
        z = (z ^ (z >> 27)).wrapping_mul(C2);
        z ^ (z >> 31)
    }

    /// A f64 in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A f64 in [lo, hi).
    pub fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next_f64() * (hi - lo)
    }

    /// An int in [lo, hi).
    pub fn int_between(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next_f64() * (hi - lo) as f64) as i32
    }

    /// Pick one element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        let i = self.int_between(0, xs.len() as i32) as usize;
        &xs[i]
    }
}
