//! Radix-2 Cooley–Tukey FFT and analytic-signal helpers.

use super::complex::Complex;

/// Next power of two greater than or equal to `n` (at least 1).
pub fn next_power_of_two(n: usize) -> usize {
    if n <= 1 {
        1
    } else {
        n.next_power_of_two()
    }
}

/// In-place forward FFT.  `buf.len()` must be a power of two.
pub fn fft_in_place(buf: &mut [Complex]) {
    let n = buf.len();
    assert!(n.is_power_of_two(), "FFT length must be a power of two");
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        let w_len = Complex::from_polar(1.0, angle);
        let half = len / 2;
        let mut start = 0usize;
        while start < n {
            let mut w = Complex::ONE;
            for k in 0..half {
                let u = buf[start + k];
                let v = buf[start + k + half] * w;
                buf[start + k] = u + v;
                buf[start + k + half] = u - v;
                w = w * w_len;
            }
            start += len;
        }
        len <<= 1;
    }
}

/// In-place inverse FFT (including the `1/N` scaling).
pub fn ifft_in_place(buf: &mut [Complex]) {
    let n = buf.len();
    for value in buf.iter_mut() {
        *value = value.conjugate();
    }
    fft_in_place(buf);
    let scale = 1.0 / n as f64;
    for value in buf.iter_mut() {
        *value = value.conjugate().scale(scale);
    }
}

/// FFT of a real signal, zero-padded to `pad_to` (rounded up to a power of two).
pub fn fft_real(signal: &[f64], pad_to: usize) -> Vec<Complex> {
    let n = next_power_of_two(pad_to.max(signal.len()).max(1));
    let mut buf = vec![Complex::ZERO; n];
    for (slot, value) in buf.iter_mut().zip(signal.iter()) {
        slot.re = *value;
    }
    fft_in_place(&mut buf);
    buf
}

/// Analytic-signal envelope via the Hilbert transform.
///
/// The returned vector has the same length as the input.  Edge samples are
/// affected by the circular FFT; callers that fit decays should trim them.
pub fn hilbert_envelope(signal: &[f64]) -> Vec<f64> {
    if signal.is_empty() {
        return Vec::new();
    }
    let n = next_power_of_two(signal.len());
    let mut buf = vec![Complex::ZERO; n];
    for (slot, value) in buf.iter_mut().zip(signal.iter()) {
        slot.re = *value;
    }
    fft_in_place(&mut buf);

    // Multiply positive frequencies by two, keep DC and Nyquist, zero the
    // negative frequencies.
    for value in buf.iter_mut().take(n / 2).skip(1) {
        *value = value.scale(2.0);
    }
    for value in buf.iter_mut().skip(n / 2 + 1) {
        *value = Complex::ZERO;
    }

    ifft_in_place(&mut buf);
    buf.iter()
        .take(signal.len())
        .map(|value| value.magnitude())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn fft_roundtrip() {
        let signal: Vec<f64> = (0..256)
            .map(|i| (i as f64 * 0.1).sin() + 0.3 * (i as f64 * 0.31).cos())
            .collect();
        let mut spectrum = fft_real(&signal, 256);
        assert_eq!(spectrum.len(), 256);
        ifft_in_place(&mut spectrum);
        for (i, expected) in signal.iter().enumerate() {
            assert!(approx(spectrum[i].re, *expected, 1e-9));
            assert!(approx(spectrum[i].im, 0.0, 1e-9));
        }
    }

    #[test]
    fn sine_has_single_bin() {
        let fs = 48_000.0;
        let n = 4096;
        let f = 1000.0;
        let signal: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / fs).sin())
            .collect();
        let spectrum = fft_real(&signal, n);
        let mut best = 0usize;
        let mut best_mag = 0.0;
        for (i, value) in spectrum.iter().take(n / 2).enumerate() {
            let mag = value.magnitude();
            if mag > best_mag {
                best_mag = mag;
                best = i;
            }
        }
        let peak_hz = best as f64 * fs / n as f64;
        assert!(approx(peak_hz, f, fs / n as f64));
    }

    #[test]
    fn hilbert_envelope_of_cosine_is_flat() {
        let n = 4096;
        let signal: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 40.0 * i as f64 / n as f64).cos())
            .collect();
        let env = hilbert_envelope(&signal);
        // Ignore circular edge effects.
        for value in env.iter().take(n - 200).skip(200) {
            assert!(approx(*value, 1.0, 1e-6), "envelope {value}");
        }
    }

    #[test]
    fn hilbert_envelope_of_decay_is_exponential() {
        let fs = 48_000.0;
        let f = 500.0;
        let zeta = 0.01;
        let n = 24_000;
        let signal: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / fs;
                (-zeta * 2.0 * std::f64::consts::PI * f * t).exp()
                    * (2.0 * std::f64::consts::PI * f * t).cos()
            })
            .collect();
        let env = hilbert_envelope(&signal);
        let a = env[2000];
        let b = env[20_000];
        let expected = (-zeta * 2.0 * std::f64::consts::PI * f * (18_000.0 / fs)).exp();
        assert!(approx(b / a, expected, 0.02), "{} vs {}", b / a, expected);
    }
}
