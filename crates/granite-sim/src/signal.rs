//! Synthetic impact response and laser-vibrometer signal chain.
//!
//! The plate model gives a set of modal frequencies, damping ratios and
//! coupling amplitudes.  An impact is synthesised as the sum of exponentially
//! decaying sinusoids, each shaped by the finite contact time of the hammer.
//! The resulting velocity signal then passes through a simple sensor model:
//! white and pink noise, mains pickup, DC offset, gain, clipping and ADC
//! quantisation.

use std::f64::consts::PI;

use granite_core::rng::Rng;

use crate::plate::PlateMode;

/// One hammer tap.
#[derive(Clone, Copy, Debug)]
pub struct Tap {
    pub time_s: f64,
    pub amplitude: f64,
    /// Contact-time low-pass corner; a hard tap excites more high frequencies.
    pub contact_hz: f64,
}

impl Tap {
    pub const fn new(time_s: f64, amplitude: f64, contact_hz: f64) -> Self {
        Self {
            time_s,
            amplitude,
            contact_hz,
        }
    }
}

/// Laser-vibrometer / ADC imperfections.
#[derive(Clone, Debug)]
pub struct SensorConfig {
    /// White-noise RMS relative to the clean signal RMS, in dB.
    pub snr_db: f64,
    /// Pink (1/f-ish) noise RMS as a fraction of the white-noise RMS.
    pub pink_fraction: f64,
    pub mains_hz: f64,
    /// Mains hum RMS relative to full scale, in dBFS.
    pub mains_dbfs: f64,
    pub quantization_bits: u8,
    pub gain: f64,
    pub dc_offset: f64,
    /// Target peak of the clean signal before gain.
    pub signal_peak: f64,
}

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            snr_db: 38.0,
            pink_fraction: 0.45,
            mains_hz: 50.0,
            mains_dbfs: -62.0,
            quantization_bits: 16,
            gain: 1.0,
            dc_offset: 0.0015,
            signal_peak: 0.7,
        }
    }
}

impl SensorConfig {
    pub fn noisy(snr_db: f64) -> Self {
        Self {
            snr_db,
            ..Default::default()
        }
    }

    pub fn overloaded() -> Self {
        Self {
            gain: 2.4,
            snr_db: 34.0,
            ..Default::default()
        }
    }
}

/// Select the modes that matter for the synthesised signal.
///
/// A real plate has a dense spectrum, but most high-order modes couple weakly
/// to a given impact/sensor pair.  Keeping the strongest modes within the
/// sensor band makes the twin both realistic and tractable for the analysis
/// pipeline.
pub fn dominant_modes(
    modes: &[PlateMode],
    max_frequency_hz: f64,
    contact_hz: f64,
    max_modes: usize,
    min_separation_fraction: f64,
) -> Vec<PlateMode> {
    let mut ranked: Vec<PlateMode> = modes
        .iter()
        .filter(|mode| mode.frequency_hz > 0.0 && mode.frequency_hz <= max_frequency_hz)
        .map(|mode| {
            let mut mode = mode.clone();
            let contact = (-(mode.frequency_hz / contact_hz.max(100.0)).powi(2)).exp();
            mode.amplitude *= contact;
            mode
        })
        .filter(|mode| mode.amplitude.abs() > 1e-9)
        .collect();
    ranked.sort_by(|a, b| {
        b.amplitude
            .abs()
            .partial_cmp(&a.amplitude.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut selected: Vec<PlateMode> = Vec::new();
    for mode in ranked {
        let separation = (mode.frequency_hz * min_separation_fraction).max(3.0);
        if selected
            .iter()
            .any(|other| (other.frequency_hz - mode.frequency_hz).abs() < separation)
        {
            continue;
        }
        selected.push(mode);
        if selected.len() >= max_modes {
            break;
        }
    }
    selected.sort_by(|a, b| {
        a.frequency_hz
            .partial_cmp(&b.frequency_hz)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    selected
}

/// Synthesise the clean velocity signal from a set of modes and taps.
pub fn modal_response(
    modes: &[PlateMode],
    taps: &[Tap],
    sample_rate_hz: f64,
    duration_s: f64,
) -> Vec<f64> {
    let n = (sample_rate_hz * duration_s).max(1.0) as usize;
    let mut samples = vec![0.0f64; n];
    let dt = 1.0 / sample_rate_hz;
    for tap in taps {
        let start = (tap.time_s * sample_rate_hz).round() as usize;
        if start >= n {
            continue;
        }
        for mode in modes {
            let frequency = mode.frequency_hz;
            if frequency <= 0.0 || frequency >= sample_rate_hz * 0.5 {
                continue;
            }
            let contact = (-(frequency / tap.contact_hz.max(100.0)).powi(2)).exp();
            let amplitude = mode.amplitude * tap.amplitude * contact;
            if amplitude.abs() < 1e-15 {
                continue;
            }
            let omega = 2.0 * PI * frequency;
            let zeta = mode.damping_ratio.clamp(1e-6, 0.5);
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * dt).exp();
            let step = omega_d * dt;
            let rot_sin = step.sin();
            let rot_cos = step.cos();
            let mut osc_sin = 0.0f64;
            let mut osc_cos = 1.0f64;
            let mut envelope = amplitude;
            for slot in samples.iter_mut().skip(start) {
                *slot += envelope * osc_sin;
                let next_sin = osc_sin * rot_cos + osc_cos * rot_sin;
                let next_cos = osc_cos * rot_cos - osc_sin * rot_sin;
                osc_sin = next_sin;
                osc_cos = next_cos;
                envelope *= decay;
            }
        }
    }
    samples
}

/// Apply the sensor/ADC chain and return normalised `f32` samples.
pub fn sensor_chain(
    clean: &[f64],
    sample_rate_hz: f64,
    config: &SensorConfig,
    rng: &mut Rng,
) -> Vec<f32> {
    let n = clean.len();
    if n == 0 {
        return Vec::new();
    }
    let peak = clean.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    let scale = if peak > 0.0 {
        config.signal_peak / peak
    } else {
        0.0
    };
    let signal: Vec<f64> = clean.iter().map(|v| v * scale).collect();
    let signal_rms = (signal.iter().map(|v| v * v).sum::<f64>() / n as f64)
        .sqrt()
        .max(1e-12);
    let white_rms = signal_rms / 10.0f64.powf(config.snr_db.max(0.0) / 20.0);

    // White noise.
    let mut white = vec![0.0f64; n];
    for slot in &mut white {
        *slot = rng.normal() * white_rms;
    }

    // Pink-ish noise: one-pole low-passed white noise, renormalised to the
    // requested RMS fraction.
    let mut pink = vec![0.0f64; n];
    let alpha = 0.02;
    let mut state = 0.0;
    for i in 0..n {
        state += alpha * (white[i] - state);
        pink[i] = state;
    }
    let pink_rms = (pink.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt();
    let pink_target = white_rms * config.pink_fraction.max(0.0);
    let pink_scale = if pink_rms > 0.0 {
        pink_target / pink_rms
    } else {
        0.0
    };

    let mains_amplitude = 10.0f64.powf(config.mains_dbfs.clamp(-160.0, 0.0) / 20.0) * 2.0f64.sqrt();
    let levels = if config.quantization_bits >= 1 && config.quantization_bits <= 32 {
        (1u64 << (config.quantization_bits - 1)) as f64
    } else {
        0.0
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sample_rate_hz;
        let mains = if config.mains_hz > 0.0 {
            mains_amplitude * (2.0 * PI * config.mains_hz * t).sin()
        } else {
            0.0
        };
        let mut value =
            (signal[i] + white[i] + pink[i] * pink_scale + mains) * config.gain + config.dc_offset;
        value = value.clamp(-1.0, 1.0);
        if levels > 0.0 {
            value = (value * levels).round() / levels;
        }
        out.push(value as f32);
    }
    out
}

/// Convenience wrapper: modes + taps -> clean signal -> sensor chain.
pub fn synthesize(
    modes: &[PlateMode],
    taps: &[Tap],
    sample_rate_hz: f64,
    duration_s: f64,
    sensor: &SensorConfig,
    rng: &mut Rng,
) -> Vec<f32> {
    let clean = modal_response(modes, taps, sample_rate_hz, duration_s);
    sensor_chain(&clean, sample_rate_hz, sensor, rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::GRANITE;
    use crate::plate::{PlateGeometry, PlateModel, Point2};

    fn modes() -> Vec<PlateMode> {
        let model = PlateModel::new(
            PlateGeometry::nominal_granite(),
            GRANITE,
            Point2::new(0.18, 0.14),
            Point2::new(0.33, 0.22),
            6000.0,
        );
        dominant_modes(&model.modes, 6000.0, 3000.0, 16, 0.004)
    }

    #[test]
    fn response_starts_at_the_tap_and_decays() {
        let taps = [Tap::new(0.1, 1.0, 3000.0)];
        let signal = modal_response(&modes(), &taps, 48_000.0, 1.0);
        assert!(signal[..4800].iter().all(|v| v.abs() < 1e-9));
        let early: f64 = signal[4800..9600].iter().map(|v| v.abs()).sum();
        let late: f64 = signal[38_400..43_200].iter().map(|v| v.abs()).sum();
        assert!(early > late * 2.0, "early {early}, late {late}");
    }

    #[test]
    fn sensor_output_is_finite_and_bounded() {
        let taps = [Tap::new(0.1, 1.0, 3000.0)];
        let clean = modal_response(&modes(), &taps, 48_000.0, 0.5);
        let mut rng = Rng::new(1);
        let out = sensor_chain(&clean, 48_000.0, &SensorConfig::default(), &mut rng);
        assert_eq!(out.len(), 24_000);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
