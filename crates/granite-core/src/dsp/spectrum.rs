//! One-sided amplitude spectra and spectral descriptors.

use super::fft::{fft_in_place, next_power_of_two};
use super::window::WindowKind;
use super::Complex;
use crate::stats;

/// A one-sided, amplitude-corrected spectrum.
#[derive(Clone, Debug)]
pub struct Spectrum {
    pub sample_rate_hz: f64,
    pub freqs_hz: Vec<f64>,
    pub magnitude: Vec<f64>,
    pub magnitude_db: Vec<f64>,
    pub enbw_hz: f64,
    pub window: WindowKind,
}

impl Spectrum {
    pub fn len(&self) -> usize {
        self.freqs_hz.len()
    }

    pub fn is_empty(&self) -> bool {
        self.freqs_hz.is_empty()
    }

    /// Bin width in Hz.
    pub fn bin_hz(&self) -> f64 {
        if self.freqs_hz.len() >= 2 {
            self.freqs_hz[1] - self.freqs_hz[0]
        } else {
            0.0
        }
    }

    /// Highest magnitude and its index.
    pub fn peak(&self) -> Option<(usize, f64)> {
        let mut best = None;
        for (i, value) in self.magnitude.iter().enumerate() {
            if best.is_none_or(|(_, b)| *value > b) {
                best = Some((i, *value));
            }
        }
        best
    }

    /// Parabolic interpolation of the peak at `index` using log magnitude.
    pub fn interpolated_frequency(&self, index: usize) -> f64 {
        if index == 0 || index + 1 >= self.len() {
            return self.freqs_hz.get(index).copied().unwrap_or(0.0);
        }
        let y0 = self.magnitude_db[index - 1];
        let y1 = self.magnitude_db[index];
        let y2 = self.magnitude_db[index + 1];
        let denom = y0 - 2.0 * y1 + y2;
        let delta = if denom.abs() < 1e-12 {
            0.0
        } else {
            (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5)
        };
        self.freqs_hz[index] + delta * self.bin_hz()
    }

    /// Scale all magnitudes so the maximum is 1.0.
    pub fn normalize_max(&mut self) {
        let max = self.magnitude.iter().cloned().fold(0.0, f64::max);
        if max > 0.0 {
            for value in &mut self.magnitude {
                *value /= max;
            }
            for value in &mut self.magnitude_db {
                *value -= 20.0 * max.log10();
            }
        }
    }
}

/// Compute a one-sided amplitude spectrum.
///
/// `pad_to` zero-pads the FFT (rounded up to a power of two).  Amplitudes are
/// corrected for the window coherent gain, so a full-scale sine produces a
/// peak amplitude near 1.0.
pub fn amplitude_spectrum(
    signal: &[f64],
    sample_rate_hz: f64,
    window: WindowKind,
    pad_to: usize,
) -> Spectrum {
    if signal.is_empty() || sample_rate_hz <= 0.0 {
        return Spectrum {
            sample_rate_hz,
            freqs_hz: Vec::new(),
            magnitude: Vec::new(),
            magnitude_db: Vec::new(),
            enbw_hz: 0.0,
            window,
        };
    }
    let n = signal.len();
    let padded = next_power_of_two(pad_to.max(n));
    let w = window.values(n);
    let sum_w = w.iter().sum::<f64>();
    let sum_w2 = w.iter().map(|v| v * v).sum::<f64>();

    let mut buf = vec![Complex::ZERO; padded];
    for i in 0..n {
        buf[i].re = signal[i] * w[i];
    }
    fft_in_place(&mut buf);

    let bins = padded / 2 + 1;
    let mut freqs_hz = Vec::with_capacity(bins);
    let mut magnitude = Vec::with_capacity(bins);
    let mut magnitude_db = Vec::with_capacity(bins);
    for (k, bin) in buf.iter().enumerate().take(bins) {
        let factor = if k == 0 || (padded % 2 == 0 && k == padded / 2) {
            1.0
        } else {
            2.0
        };
        let amp = factor * bin.magnitude() / sum_w;
        freqs_hz.push(k as f64 * sample_rate_hz / padded as f64);
        magnitude.push(amp);
        magnitude_db.push(20.0 * amp.max(super::TINY).log10());
    }

    Spectrum {
        sample_rate_hz,
        freqs_hz,
        magnitude,
        magnitude_db,
        enbw_hz: sample_rate_hz * sum_w2 / (sum_w * sum_w),
        window,
    }
}

/// Descriptors derived from a spectrum between `f_min` and `f_max`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpectralDescriptors {
    pub centroid_hz: f64,
    pub spread_hz: f64,
    pub rolloff95_hz: f64,
    pub flatness: f64,
    pub tilt_db_per_octave: f64,
    pub total_power: f64,
}

/// Compute centroid, spread, rolloff, flatness and spectral tilt.
pub fn spectral_descriptors(spectrum: &Spectrum, f_min: f64, f_max: f64) -> SpectralDescriptors {
    let mut indices = Vec::new();
    let mut total = 0.0;
    for i in 0..spectrum.len() {
        let f = spectrum.freqs_hz[i];
        if f >= f_min && f <= f_max {
            let p = spectrum.magnitude[i] * spectrum.magnitude[i];
            if p > 0.0 {
                total += p;
                indices.push((i, p));
            }
        }
    }
    if total <= 0.0 || indices.is_empty() {
        return SpectralDescriptors::default();
    }

    let centroid = indices
        .iter()
        .map(|(i, p)| spectrum.freqs_hz[*i] * p)
        .sum::<f64>()
        / total;
    let spread = (indices
        .iter()
        .map(|(i, p)| {
            let d = spectrum.freqs_hz[*i] - centroid;
            d * d * p
        })
        .sum::<f64>()
        / total)
        .sqrt();

    let mut cumulative = 0.0;
    let mut rolloff = f_max;
    for (i, p) in &indices {
        cumulative += p;
        if cumulative >= 0.95 * total {
            rolloff = spectrum.freqs_hz[*i];
            break;
        }
    }

    let count = indices.len() as f64;
    let log_mean = indices.iter().map(|(_, p)| p.max(1e-30).ln()).sum::<f64>() / count;
    let arithmetic_mean = total / count;
    let flatness = (log_mean.exp() / arithmetic_mean).clamp(0.0, 1.0);

    // Linear regression of dB magnitude against log2(f), using bins within
    // 60 dB of the strongest bin to avoid fitting the noise floor.
    let peak = indices.iter().map(|(_, p)| *p).fold(0.0, f64::max);
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (i, p) in &indices {
        if *p >= peak * 1e-6 {
            xs.push(spectrum.freqs_hz[*i].log2());
            ys.push(10.0 * p.max(1e-30).log10());
        }
    }
    let tilt = stats::linreg(&xs, &ys).map_or(0.0, |fit| fit.slope);

    SpectralDescriptors {
        centroid_hz: centroid,
        spread_hz: spread,
        rolloff95_hz: rolloff,
        flatness,
        tilt_db_per_octave: tilt,
        total_power: total,
    }
}

/// Average several spectra bin-by-bin.  Shorter spectra are padded with zeros.
pub fn average_spectra(spectra: &[Spectrum]) -> Spectrum {
    if spectra.is_empty() {
        return Spectrum {
            sample_rate_hz: 0.0,
            freqs_hz: Vec::new(),
            magnitude: Vec::new(),
            magnitude_db: Vec::new(),
            enbw_hz: 0.0,
            window: WindowKind::Hann,
        };
    }
    let len = spectra.iter().map(Spectrum::len).min().unwrap_or(0);
    if len == 0 {
        return spectra[0].clone();
    }
    let mut magnitude = vec![0.0; len];
    for spectrum in spectra {
        for (slot, value) in magnitude.iter_mut().zip(spectrum.magnitude.iter()) {
            *slot += *value;
        }
    }
    let count = spectra.len() as f64;
    for value in &mut magnitude {
        *value /= count;
    }
    let magnitude_db = magnitude
        .iter()
        .map(|v| 20.0 * v.max(super::TINY).log10())
        .collect();
    Spectrum {
        sample_rate_hz: spectra[0].sample_rate_hz,
        freqs_hz: spectra[0].freqs_hz.iter().take(len).copied().collect(),
        magnitude,
        magnitude_db,
        enbw_hz: spectra[0].enbw_hz,
        window: spectra[0].window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peak_has_correct_frequency_and_amplitude() {
        let fs = 48_000.0;
        let n = 48_000;
        let f = 1234.5;
        let signal: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / fs).sin())
            .collect();
        let spectrum = amplitude_spectrum(&signal, fs, WindowKind::Hann, n * 2);
        let (index, _) = spectrum.peak().unwrap();
        let peak = spectrum.interpolated_frequency(index);
        assert!((peak - f).abs() < 0.5, "peak {peak}");
        let amp = spectrum.magnitude[index];
        assert!((amp - 1.0).abs() < 0.02, "amp {amp}");
    }

    #[test]
    fn descriptors_of_sine_are_reasonable() {
        let fs = 48_000.0;
        let n = 48_000;
        let f = 2000.0;
        let signal: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / fs).sin())
            .collect();
        let spectrum = amplitude_spectrum(&signal, fs, WindowKind::Hann, n);
        let d = spectral_descriptors(&spectrum, 20.0, 20_000.0);
        assert!((d.centroid_hz - f).abs() < 10.0);
        assert!(d.rolloff95_hz > f);
        assert!(d.flatness < 0.1);
    }
}
