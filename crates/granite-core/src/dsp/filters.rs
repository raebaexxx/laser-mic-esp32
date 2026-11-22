//! Second-order IIR filters and zero-phase forward/backward filtering.

/// Transposed direct-form II biquad.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Create a filter from normalised coefficients (`a0` already divided out).
    pub const fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// A pass-through filter.
    pub const fn identity() -> Self {
        Self::new(1.0, 0.0, 0.0, 0.0, 0.0)
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// RBJ low-pass with cutoff `f0` and quality factor `q`.
    pub fn lowpass(sample_rate_hz: f64, f0: f64, q: f64) -> Self {
        let (w0, cos_w0, alpha) = rbj_terms(sample_rate_hz, f0, q);
        let _ = w0;
        let a0 = 1.0 + alpha;
        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// RBJ high-pass with cutoff `f0` and quality factor `q`.
    pub fn highpass(sample_rate_hz: f64, f0: f64, q: f64) -> Self {
        let (_w0, cos_w0, alpha) = rbj_terms(sample_rate_hz, f0, q);
        let a0 = 1.0 + alpha;
        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// RBJ band-pass with 0 dB peak gain.
    pub fn bandpass(sample_rate_hz: f64, f0: f64, q: f64) -> Self {
        let (_w0, cos_w0, alpha) = rbj_terms(sample_rate_hz, f0, q);
        let a0 = 1.0 + alpha;
        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// RBJ notch with quality factor `q`.
    pub fn notch(sample_rate_hz: f64, f0: f64, q: f64) -> Self {
        let (_w0, cos_w0, alpha) = rbj_terms(sample_rate_hz, f0, q);
        let a0 = 1.0 + alpha;
        let b0 = 1.0;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::new(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }
}

fn rbj_terms(sample_rate_hz: f64, f0: f64, q: f64) -> (f64, f64, f64) {
    let fs = sample_rate_hz.max(1.0);
    let f0 = f0.clamp(1e-6, fs * 0.499_999);
    let q = q.max(1e-6);
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    (w0, cos_w0, alpha)
}

fn apply_in_place(signal: &mut [f64], filter: &Biquad) {
    let mut f = *filter;
    for value in signal {
        *value = f.process(*value);
    }
}

fn reverse(signal: &[f64]) -> Vec<f64> {
    signal.iter().rev().copied().collect()
}

fn reflected_padding(signal: &[f64], pad: usize) -> Vec<f64> {
    let n = signal.len();
    if n == 0 || pad == 0 {
        return signal.to_vec();
    }
    let pad = pad.min(n - 1);
    let mut out = Vec::with_capacity(n + 2 * pad);
    for i in 0..pad {
        out.push(signal[(pad - i).min(n - 1)]);
    }
    out.extend_from_slice(signal);
    for i in 0..pad {
        out.push(signal[n.saturating_sub(2 + i).min(n - 1)]);
    }
    out
}

/// Zero-phase forward/backward filtering with one biquad.
pub fn filtfilt(signal: &[f64], filter: &Biquad) -> Vec<f64> {
    filtfilt_cascade(signal, &[*filter])
}

/// Zero-phase forward/backward filtering with a cascade of biquads.
///
/// The signal is reflected at both ends to reduce startup transients.
pub fn filtfilt_cascade(signal: &[f64], sections: &[Biquad]) -> Vec<f64> {
    if signal.len() < 4 || sections.is_empty() {
        return signal.to_vec();
    }
    let pad = 256.min(signal.len() - 1);
    let mut work = reflected_padding(signal, pad);

    for section in sections {
        apply_in_place(&mut work, section);
    }
    work = reverse(&work);
    for section in sections {
        apply_in_place(&mut work, section);
    }
    work = reverse(&work);
    work.truncate(work.len().saturating_sub(pad));
    work.drain(..pad.min(work.len()));
    work
}

/// Causal forward filtering with a cascade (used where phase is unimportant).
pub fn filter_cascade(signal: &[f64], sections: &[Biquad]) -> Vec<f64> {
    let mut out = signal.to_vec();
    for section in sections {
        apply_in_place(&mut out, section);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(fs: f64, f: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / fs).sin())
            .collect()
    }

    #[test]
    fn lowpass_passes_dc() {
        let signal = vec![1.0; 2000];
        let out = filtfilt(&signal, &Biquad::lowpass(48_000.0, 1000.0, 0.707));
        for value in out.iter().skip(200) {
            assert!((*value - 1.0).abs() < 1e-3, "value {value}");
        }
    }

    #[test]
    fn highpass_blocks_dc() {
        let signal = vec![1.0; 4000];
        let out = filtfilt(&signal, &Biquad::highpass(48_000.0, 100.0, 0.707));
        for value in out.iter().skip(1000) {
            assert!(value.abs() < 1e-3, "value {value}");
        }
    }

    #[test]
    fn notch_removes_mains_frequency() {
        let fs = 48_000.0;
        let signal = sine(fs, 50.0, 96_000);
        let out = filtfilt(&signal, &Biquad::notch(fs, 50.0, 10.0));
        // Ignore the edge transients of the zero-phase filter.
        let middle = &out[24_000..72_000];
        let rms_out = (middle.iter().map(|v| v * v).sum::<f64>() / middle.len() as f64).sqrt();
        assert!(rms_out < 0.01, "rms out {rms_out}");
    }

    #[test]
    fn bandpass_selects_frequency() {
        let fs = 48_000.0;
        let signal: Vec<f64> = (0..24_000)
            .map(|i| {
                let t = i as f64 / fs;
                (2.0 * std::f64::consts::PI * 500.0 * t).sin()
                    + (2.0 * std::f64::consts::PI * 3000.0 * t).sin()
            })
            .collect();
        let out = filtfilt(&signal, &Biquad::bandpass(fs, 500.0, 10.0));
        let rms = (out.iter().map(|v| v * v).sum::<f64>() / out.len() as f64).sqrt();
        // The 500 Hz component has RMS 0.707; the 3 kHz component is rejected.
        assert!((rms - 0.707).abs() < 0.1, "rms {rms}");
    }

    #[test]
    fn filtfilt_preserves_phase() {
        let fs = 48_000.0;
        let signal = sine(fs, 1000.0, 4800);
        let out = filtfilt(&signal, &Biquad::lowpass(fs, 5000.0, 0.707));
        let max_index = |x: &[f64]| {
            x.iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
                .unwrap()
                .0
        };
        let i_in = max_index(&signal);
        let i_out = max_index(&out);
        assert!((i_in as i64 - i_out as i64).abs() <= 1);
    }
}
