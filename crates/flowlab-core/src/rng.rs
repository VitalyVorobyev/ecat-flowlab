//! The deterministic generator of the simulator, the studies and the PDI benchmark: SplitMix64.
//! It is implemented here, not taken from a crate, so that the same seed gives the same numbers
//! across dependency upgrades, and with them byte-identical captures and results.

/// The SplitMix64 output function: a bijective mix of a 64-bit value.
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The SplitMix64 increment (the golden ratio, odd).
pub const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64: a 64-bit state advanced by [`GOLDEN`] and mixed with [`mix64`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// A generator starting from `seed`.
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN);
        mix64(self.0)
    }

    /// Uniform in [0, 1), from the top 53 bits.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `true` with probability `p` (never for `p` ≤ 0).
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && self.unit() < p
    }

    /// Standard normal (Box–Muller, one of the pair).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit().max(f64::MIN_POSITIVE);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Uniform in `0..n` (by remainder; `n` = 0 gives 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_values() {
        // SplitMix64 from seed 0, as in the reference implementation.
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        let mut r = SplitMix64::new(7);
        assert!((0..1000).map(|_| r.unit()).all(|u| (0.0..1.0).contains(&u)));
        assert!(!r.chance(0.0));
        assert!(r.below(5) < 5);
        assert_eq!(r.below(0), 0);
    }
}
