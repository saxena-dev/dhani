//! Pure retry and reconnect backoff: the `SplitMix64` generator and the full-jitter `Backoff`.
//!
//! Used by REST retries and feed reconnects. The delays are SDK policy supplied by the caller's
//! configuration. There is no `rand` dependency: jitter needs spread, not cryptographic
//! strength, so a small seedable SplitMix64 generator is enough.

use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// A small, seedable pseudo-random generator (SplitMix64).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by REST retries and feed reconnects")
)]
#[derive(Clone, Debug)]
pub(crate) struct SplitMix64(u64);

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by REST retries and feed reconnects")
)]
impl SplitMix64 {
    /// A generator with a fixed seed; the same seed gives the same sequence.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A generator seeded from the standard library's randomly keyed `RandomState` hasher.
    pub(crate) fn from_entropy() -> Self {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(0x9E37_79B9_7F4A_7C15);
        Self(hasher.finish())
    }

    /// The next value in the sequence.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Full-jitter exponential backoff.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by REST retries and feed reconnects")
)]
#[derive(Clone, Debug)]
pub(crate) struct Backoff {
    initial: Duration,
    max: Duration,
    rng: SplitMix64,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by REST retries and feed reconnects")
)]
impl Backoff {
    /// A backoff starting at `initial`, capped at `max`, drawing jitter from `rng`.
    pub(crate) fn new(initial: Duration, max: Duration, rng: SplitMix64) -> Self {
        Self { initial, max, rng }
    }

    /// The delay after `failures` consecutive failures; see [`full_jitter`].
    pub(crate) fn delay(&mut self, failures: u32) -> Duration {
        full_jitter(self.initial, self.max, failures, &mut self.rng)
    }
}

/// Full jitter: uniform in `[0, min(max, initial · 2^(failures − 1))]`, and zero for
/// `failures == 0`. The exponent saturates at `max` instead of overflowing.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by REST retries and feed reconnects")
)]
pub(crate) fn full_jitter(
    initial: Duration,
    max: Duration,
    failures: u32,
    rng: &mut SplitMix64,
) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    let ceiling = match 1u32.checked_shl(failures - 1) {
        Some(factor) => initial.saturating_mul(factor).min(max),
        None => max,
    };
    // Durations beyond u64 nanoseconds (about 584 years) are clamped.
    let ceiling_nanos = u64::try_from(ceiling.as_nanos()).unwrap_or(u64::MAX);
    // Multiply-shift maps a uniform u64 onto 0..=ceiling_nanos.
    let span = u128::from(ceiling_nanos) + 1;
    let nanos = (u128::from(rng.next_u64()) * span) >> 64;
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(ceiling_nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn same_seed_gives_the_same_sequence() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        let xs: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs[0], xs[1]);
        let mut c = SplitMix64::new(43);
        assert_ne!(c.next_u64(), xs[0]);
    }

    #[test]
    fn known_splitmix64_output() {
        // Reference values for seed 0 from the published SplitMix64 algorithm.
        let mut g = SplitMix64::new(0);
        assert_eq!(g.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(g.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn delays_stay_within_their_ceiling() {
        let mut b = Backoff::new(250 * MS, 4000 * MS, SplitMix64::new(7));
        assert_eq!(b.delay(0), Duration::ZERO);
        for _ in 0..1000 {
            assert!(b.delay(1) <= 250 * MS);
            assert!(b.delay(40) <= 4000 * MS);
            assert!(b.delay(u32::MAX) <= 4000 * MS);
        }
    }

    #[test]
    fn full_jitter_spreads_over_the_whole_range() {
        let mut b = Backoff::new(250 * MS, 4000 * MS, SplitMix64::new(1));
        let samples: Vec<Duration> = (0..10_000).map(|_| b.delay(3)).collect();
        let max = samples.iter().max().unwrap();
        let min = samples.iter().min().unwrap();
        // delay(3) is uniform in [0, min(4 s, 250 ms · 4)] = [0, 1 s].
        assert!(*max <= 1000 * MS, "{max:?}");
        assert!(*max > 900 * MS, "{max:?}");
        assert!(*min < 100 * MS, "{min:?}");
    }

    #[test]
    fn huge_durations_do_not_panic() {
        let mut b = Backoff::new(Duration::MAX, Duration::MAX, SplitMix64::new(3));
        let _ = b.delay(1);
        let _ = b.delay(64);
    }

    #[test]
    fn exponent_saturates_at_max() {
        // failures 32 is the last shift that fits in u32; 33 and beyond use `max` directly.
        let mut b = Backoff::new(MS, 5 * MS, SplitMix64::new(9));
        for failures in [32, 33, u32::MAX] {
            assert!((0..200).all(|_| b.delay(failures) <= 5 * MS));
        }
        // A cap below the initial delay wins.
        let mut b = Backoff::new(100 * MS, 10 * MS, SplitMix64::new(9));
        assert!((0..200).all(|_| b.delay(1) <= 10 * MS));
    }

    #[test]
    fn entropy_seeds_differ_between_generators() {
        let a = SplitMix64::from_entropy().next_u64();
        let b = SplitMix64::from_entropy().next_u64();
        assert_ne!(a, b);
    }
}
