//! Deterministic pseudo-random numbers for the digital twin.
//!
//! SplitMix64 is tiny, fast and reproducible across platforms.  No external
//! dependency is needed, which keeps the host and the simulator builds simple.

#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Uniform in `[low, high)`.
    #[inline]
    pub fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next_f64()
    }

    /// Standard normal via Box–Muller.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Bernoulli trial.
    #[inline]
    pub fn chance(&mut self, probability: f64) -> bool {
        self.next_f64() < probability
    }

    /// Random index in `[0, n)`; returns 0 for `n == 0`.
    pub fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// In-place Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, values: &mut [T]) {
        for i in (1..values.len()).rev() {
            let j = self.index(i + 1);
            values.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn normal_has_roughly_zero_mean() {
        let mut rng = Rng::new(7);
        let values: Vec<f64> = (0..20_000).map(|_| rng.normal()).collect();
        let m = crate::stats::mean(&values);
        let s = crate::stats::std_dev(&values);
        assert!(m.abs() < 0.03, "mean {m}");
        assert!((s - 1.0).abs() < 0.03, "std {s}");
    }
}
