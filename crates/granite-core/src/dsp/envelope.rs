//! Envelope and moving-window helpers.

pub use super::fft::hilbert_envelope;

/// Centred moving average with edge clamping.
pub fn moving_average(signal: &[f64], window: usize) -> Vec<f64> {
    let n = signal.len();
    if n == 0 || window == 0 {
        return signal.to_vec();
    }
    let half = window / 2;
    let mut prefix = vec![0.0; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + signal[i];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(n);
        out.push((prefix[end] - prefix[start]) / (end - start) as f64);
    }
    out
}

/// Centred moving RMS with edge clamping.
pub fn moving_rms(signal: &[f64], window: usize) -> Vec<f64> {
    let n = signal.len();
    if n == 0 || window == 0 {
        return signal.to_vec();
    }
    let half = window / 2;
    let mut prefix = vec![0.0; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + signal[i] * signal[i];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(n);
        out.push(((prefix[end] - prefix[start]) / (end - start) as f64).sqrt());
    }
    out
}

/// Smooth an envelope with a short moving average.
pub fn smooth_envelope(envelope: &[f64], window: usize) -> Vec<f64> {
    moving_average(envelope, window.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_average_of_constant_is_constant() {
        let signal = vec![2.0; 100];
        let out = moving_average(&signal, 7);
        assert!(out.iter().all(|v| (*v - 2.0).abs() < 1e-12));
    }

    #[test]
    fn moving_rms_of_sine_is_about_rms() {
        let n = 4800;
        let signal: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin())
            .collect();
        let env = moving_rms(&signal, 480);
        for value in env.iter().skip(500).take(3000) {
            assert!((*value - 0.707).abs() < 0.02, "value {value}");
        }
    }
}
