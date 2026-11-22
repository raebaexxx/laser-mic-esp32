//! Window functions and their normalisation constants.

/// Analysis window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Rectangular,
    Hann,
    /// Half-Hann taper: 1 at the start, 0 at the end.  Ideal for an impact
    /// response that begins at the start of the analysis window.
    HalfHann,
    Hamming,
    BlackmanHarris,
}

impl WindowKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rectangular => "rectangular",
            Self::Hann => "Hann",
            Self::HalfHann => "half-Hann (impact)",
            Self::Hamming => "Hamming",
            Self::BlackmanHarris => "Blackman-Harris",
        }
    }

    /// Window coefficients for a signal of length `n`.
    pub fn values(self, n: usize) -> Vec<f64> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![1.0];
        }
        let denom = (n - 1) as f64;
        (0..n)
            .map(|i| {
                let x = i as f64 / denom;
                match self {
                    Self::Rectangular => 1.0,
                    Self::Hann => 0.5 - 0.5 * (2.0 * std::f64::consts::PI * x).cos(),
                    Self::HalfHann => 0.5 + 0.5 * (std::f64::consts::PI * x).cos(),
                    Self::Hamming => 0.54 - 0.46 * (2.0 * std::f64::consts::PI * x).cos(),
                    Self::BlackmanHarris => {
                        let a0 = 0.35875;
                        let a1 = 0.48829;
                        let a2 = 0.14128;
                        let a3 = 0.01168;
                        let w = 2.0 * std::f64::consts::PI * x;
                        a0 - a1 * w.cos() + a2 * (2.0 * w).cos() - a3 * (3.0 * w).cos()
                    }
                }
            })
            .collect()
    }

    /// Coherent gain (mean of the window).
    pub fn coherent_gain(self, n: usize) -> f64 {
        let w = self.values(n);
        if w.is_empty() {
            1.0
        } else {
            w.iter().sum::<f64>() / w.len() as f64
        }
    }

    /// Equivalent noise bandwidth in bins.
    pub fn enbw_bins(self, n: usize) -> f64 {
        let w = self.values(n);
        let sum = w.iter().sum::<f64>();
        if sum == 0.0 {
            1.0
        } else {
            n as f64 * w.iter().map(|v| v * v).sum::<f64>() / (sum * sum)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_endpoints_are_zero() {
        let w = WindowKind::Hann.values(8);
        assert!(w[0].abs() < 1e-12);
        assert!(w[7].abs() < 1e-12);
        assert!((w[3] - 0.81).abs() < 0.2);
    }

    #[test]
    fn enbw_values_are_sensible() {
        assert!((WindowKind::Rectangular.enbw_bins(1024) - 1.0).abs() < 1e-9);
        assert!((WindowKind::Hann.enbw_bins(1024) - 1.5).abs() < 0.01);
        assert!((WindowKind::Hamming.enbw_bins(1024) - 1.3628).abs() < 0.01);
    }
}
