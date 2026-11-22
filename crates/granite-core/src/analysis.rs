//! Impact detection, modal parameter estimation and feature extraction.
//!
//! The pipeline is intentionally transparent and deterministic:
//!
//! 1. remove DC, high-pass, notch mains harmonics;
//! 2. detect impact events from a short-time RMS envelope;
//! 3. window each event and compute a zero-padded amplitude spectrum;
//! 4. pick modal peaks and estimate each mode's damping;
//! 5. cluster modes across repeated taps and derive global features.

use std::f64::consts::PI;

use crate::capture::Capture;
use crate::dsp::envelope::{hilbert_envelope, moving_rms, smooth_envelope};
use crate::dsp::fft::next_power_of_two;
use crate::dsp::filters::{filtfilt, filtfilt_cascade, Biquad};
use crate::dsp::spectrum::{amplitude_spectrum, average_spectra, spectral_descriptors, Spectrum};
use crate::dsp::window::WindowKind;
use crate::stats;

/// Tunable analysis parameters.
#[derive(Clone, Debug)]
pub struct AnalysisConfig {
    pub min_frequency_hz: f64,
    pub max_frequency_hz: Option<f64>,
    pub event_threshold_sigma: f64,
    pub event_refractory_s: f64,
    pub pre_trigger_s: f64,
    pub post_trigger_s: f64,
    pub max_events: usize,
    pub peak_threshold_db: f64,
    pub max_modes: usize,
    pub min_peak_separation_hz: f64,
    pub mains_hz: Option<f64>,
    pub mains_harmonics: usize,
    pub highpass_hz: f64,
    pub lowpass_hz: Option<f64>,
    pub window: WindowKind,
    pub zero_pad_factor: usize,
    pub damping_bandwidth_fraction: f64,
    pub damping_min_bandwidth_hz: f64,
    pub damping_min_r2: f64,
    pub decay_floor_fraction: f64,
    pub min_snr_db: f64,
    pub max_clipping_fraction: f64,
    pub max_gap_fraction: f64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            min_frequency_hz: 20.0,
            max_frequency_hz: None,
            event_threshold_sigma: 8.0,
            event_refractory_s: 0.15,
            pre_trigger_s: 0.02,
            post_trigger_s: 0.75,
            max_events: 8,
            peak_threshold_db: -38.0,
            max_modes: 10,
            min_peak_separation_hz: 5.0,
            mains_hz: Some(50.0),
            mains_harmonics: 3,
            highpass_hz: 20.0,
            lowpass_hz: None,
            window: WindowKind::HalfHann,
            zero_pad_factor: 2,
            damping_bandwidth_fraction: 0.08,
            damping_min_bandwidth_hz: 10.0,
            damping_min_r2: 0.35,
            decay_floor_fraction: 0.05,
            min_snr_db: 10.0,
            max_clipping_fraction: 0.001,
            max_gap_fraction: 0.05,
        }
    }
}

/// How a damping ratio was obtained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DampingMethod {
    EnvelopeFit,
    HalfPower,
    Default,
}

impl DampingMethod {
    pub const fn label(self) -> &'static str {
        match self {
            Self::EnvelopeFit => "exponential envelope fit",
            Self::HalfPower => "half-power bandwidth",
            Self::Default => "default / low confidence",
        }
    }
}

/// Modal estimate from one impact event.
#[derive(Clone, Debug)]
pub struct ModeEstimate {
    pub frequency_hz: f64,
    pub damping_ratio: f64,
    pub amplitude: f64,
    pub bandwidth_hz: f64,
    pub fit_r2: f64,
    pub method: DampingMethod,
    pub event_index: usize,
}

/// One detected impact and its analysis.
#[derive(Clone, Debug)]
pub struct EventAnalysis {
    pub index: usize,
    pub start_time_s: f64,
    pub peak_time_s: f64,
    pub peak_amplitude: f64,
    pub snr_db: f64,
    pub modes: Vec<ModeEstimate>,
    pub spectrum: Spectrum,
}

/// A mode tracked across repeated taps.
#[derive(Clone, Debug)]
pub struct ModalFeature {
    pub frequency_hz: f64,
    pub frequency_std_hz: f64,
    pub damping_ratio: f64,
    pub damping_std: f64,
    pub amplitude: f64,
    pub persistence: f64,
    pub q_factor: f64,
}

/// Global, mostly scale-independent features used by the classifier.
#[derive(Clone, Debug, Default)]
pub struct FeatureSet {
    pub fundamental_hz: f64,
    pub modal_frequencies_hz: Vec<f64>,
    pub modal_damping: Vec<f64>,
    pub frequency_ratios: Vec<f64>,
    pub mean_damping: f64,
    pub max_damping: f64,
    pub spectral_centroid_hz: f64,
    pub spectral_spread_hz: f64,
    pub spectral_rolloff95_hz: f64,
    pub spectral_flatness: f64,
    pub spectral_tilt_db_per_octave: f64,
    pub decay_t60_s: f64,
    pub high_freq_energy_ratio: f64,
    pub mode_splitting_score: f64,
    pub peak_amplitude: f64,
    pub rms: f64,
    pub crest_factor: f64,
    pub mode_count: usize,
    pub event_count: usize,
    pub trigger_snr_db: f64,
}

impl FeatureSet {
    /// Named scalar features, in a stable order, for reports and profiles.
    pub fn named_values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("fundamental_hz", self.fundamental_hz),
            ("mean_damping", self.mean_damping),
            ("max_damping", self.max_damping),
            ("spectral_centroid_hz", self.spectral_centroid_hz),
            ("spectral_spread_hz", self.spectral_spread_hz),
            ("spectral_rolloff95_hz", self.spectral_rolloff95_hz),
            ("spectral_flatness", self.spectral_flatness),
            (
                "spectral_tilt_db_per_octave",
                self.spectral_tilt_db_per_octave,
            ),
            ("decay_t60_s", self.decay_t60_s),
            ("high_freq_energy_ratio", self.high_freq_energy_ratio),
            ("mode_splitting_score", self.mode_splitting_score),
            ("crest_factor", self.crest_factor),
            ("mode_count", self.mode_count as f64),
            ("event_count", self.event_count as f64),
            ("trigger_snr_db", self.trigger_snr_db),
            ("peak_amplitude", self.peak_amplitude),
            ("rms", self.rms),
        ]
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.named_values()
            .into_iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }
}

/// Data-quality diagnostics.
#[derive(Clone, Debug, Default)]
pub struct QualityMetrics {
    pub gap_fraction: f64,
    pub clipping_fraction: f64,
    pub snr_db: f64,
    pub usable: bool,
    pub warnings: Vec<String>,
}

/// Complete result of analysing one capture.
#[derive(Clone, Debug)]
pub struct CaptureAnalysis {
    pub sample_rate_hz: f64,
    pub duration_s: f64,
    pub events: Vec<EventAnalysis>,
    pub modes: Vec<ModalFeature>,
    pub features: FeatureSet,
    pub quality: QualityMetrics,
    pub averaged_spectrum: Spectrum,
}

impl CaptureAnalysis {
    pub fn empty(sample_rate_hz: f64, duration_s: f64) -> Self {
        Self {
            sample_rate_hz,
            duration_s,
            events: Vec::new(),
            modes: Vec::new(),
            features: FeatureSet::default(),
            quality: QualityMetrics {
                usable: false,
                warnings: vec!["no analyzable signal".to_string()],
                ..Default::default()
            },
            averaged_spectrum: Spectrum {
                sample_rate_hz,
                freqs_hz: Vec::new(),
                magnitude: Vec::new(),
                magnitude_db: Vec::new(),
                enbw_hz: 0.0,
                window: WindowKind::Hann,
            },
        }
    }
}

struct EventSeed {
    peak_index: usize,
    start: usize,
    end: usize,
    snr_db: f64,
}

struct DampingEstimate {
    damping_ratio: f64,
    bandwidth_hz: f64,
    fit_r2: f64,
    method: DampingMethod,
}

/// Analyse a capture end-to-end.
pub fn analyze_capture(capture: &Capture, config: &AnalysisConfig) -> CaptureAnalysis {
    let fs = capture.sample_rate_hz;
    if capture.samples.is_empty() || fs <= 0.0 {
        return CaptureAnalysis::empty(fs.max(0.0), 0.0);
    }
    let duration_s = capture.duration_s();
    let raw = capture.samples_f64();
    let clipping_fraction =
        raw.iter().filter(|v| v.abs() >= 0.999).count() as f64 / raw.len().max(1) as f64;
    let gap_fraction = capture.gap_fraction();
    let max_frequency_hz = config.max_frequency_hz.unwrap_or(fs * 0.45).min(fs * 0.45);

    let filtered = preprocess(&raw, fs, config);
    let seeds = detect_events(&filtered, fs, config);
    let warnings = Vec::new();
    let mut quality = QualityMetrics {
        gap_fraction,
        clipping_fraction,
        ..Default::default()
    };

    if seeds.is_empty() {
        quality.usable = false;
        quality.warnings.push(
            "no impact events detected; check trigger threshold, sensor coupling or gain"
                .to_string(),
        );
        let mut analysis = CaptureAnalysis::empty(fs, duration_s);
        analysis.quality = quality;
        analysis.averaged_spectrum = amplitude_spectrum(
            &filtered,
            fs,
            config.window,
            next_power_of_two(filtered.len()),
        );
        return analysis;
    }

    let max_event_len = seeds
        .iter()
        .map(|seed| seed.end.saturating_sub(seed.start))
        .max()
        .unwrap_or(1);
    let pad_to =
        next_power_of_two(max_event_len.saturating_mul(config.zero_pad_factor.max(1))).min(262_144);

    let mut events = Vec::with_capacity(seeds.len());
    for (index, seed) in seeds.iter().enumerate() {
        let mut segment = filtered[seed.start..seed.end].to_vec();
        detrend(&mut segment);
        let spectrum = amplitude_spectrum(&segment, fs, config.window, pad_to);
        let peak_indices = pick_peaks(&spectrum, config, max_frequency_hz);
        let max_mag = spectrum
            .magnitude
            .iter()
            .cloned()
            .fold(0.0f64, f64::max)
            .max(1e-15);
        let mut modes = Vec::with_capacity(peak_indices.len());
        for peak_index in peak_indices {
            let frequency_hz = spectrum.interpolated_frequency(peak_index);
            let damping = estimate_damping(&segment, fs, frequency_hz, config);
            modes.push(ModeEstimate {
                frequency_hz,
                damping_ratio: damping.damping_ratio,
                amplitude: spectrum.magnitude[peak_index] / max_mag,
                bandwidth_hz: damping.bandwidth_hz,
                fit_r2: damping.fit_r2,
                method: damping.method,
                event_index: index,
            });
        }
        events.push(EventAnalysis {
            index,
            start_time_s: seed.start as f64 / fs,
            peak_time_s: seed.peak_index as f64 / fs,
            peak_amplitude: seed
                .peak_index
                .checked_sub(seed.start)
                .and_then(|i| filtered.get(seed.start + i).copied())
                .map(f64::abs)
                .unwrap_or(0.0),
            snr_db: seed.snr_db,
            modes,
            spectrum,
        });
    }

    let mut normalized: Vec<Spectrum> = events
        .iter()
        .map(|event| {
            let mut spectrum = event.spectrum.clone();
            spectrum.normalize_max();
            spectrum
        })
        .collect();
    normalized.retain(|spectrum| !spectrum.is_empty());
    let averaged_spectrum = if normalized.is_empty() {
        Spectrum {
            sample_rate_hz: fs,
            freqs_hz: Vec::new(),
            magnitude: Vec::new(),
            magnitude_db: Vec::new(),
            enbw_hz: 0.0,
            window: config.window,
        }
    } else {
        average_spectra(&normalized)
    };

    let modes = aggregate_modes(&events, config.max_modes);
    let features = build_features(
        &filtered,
        &events,
        &modes,
        &averaged_spectrum,
        fs,
        config,
        max_frequency_hz,
    );
    quality.snr_db = stats::mean(&events.iter().map(|e| e.snr_db).collect::<Vec<_>>());
    if clipping_fraction > config.max_clipping_fraction {
        quality.warnings.push(format!(
            "clipping detected ({:.3}% of samples at full scale)",
            clipping_fraction * 100.0
        ));
    }
    if gap_fraction > config.max_gap_fraction {
        quality.warnings.push(format!(
            "packet loss created gaps in {:.1}% of the capture",
            gap_fraction * 100.0
        ));
    }
    if quality.snr_db < config.min_snr_db {
        quality.warnings.push(format!(
            "low trigger SNR ({:.1} dB); modal damping estimates may be unreliable",
            quality.snr_db
        ));
    }
    if modes.len() < 2 {
        quality
            .warnings
            .push("fewer than two stable modes were found".to_string());
    }
    if averaged_spectrum
        .magnitude
        .iter()
        .cloned()
        .fold(0.0, f64::max)
        <= 1e-9
    {
        quality.warnings.push("spectrum is empty".to_string());
    }
    quality.usable = !events.is_empty()
        && !modes.is_empty()
        && clipping_fraction <= 0.01
        && gap_fraction <= 0.25;
    if !warnings.is_empty() {
        quality.warnings.extend(warnings);
    }

    CaptureAnalysis {
        sample_rate_hz: fs,
        duration_s,
        events,
        modes,
        features,
        quality,
        averaged_spectrum,
    }
}

fn preprocess(raw: &[f64], fs: f64, config: &AnalysisConfig) -> Vec<f64> {
    let mut signal = raw.to_vec();
    let mean = stats::mean(&signal);
    for value in &mut signal {
        *value -= mean;
    }
    if config.highpass_hz > 0.0 && config.highpass_hz < fs * 0.45 {
        signal = filtfilt(
            &signal,
            &Biquad::highpass(fs, config.highpass_hz, std::f64::consts::FRAC_1_SQRT_2),
        );
    }
    if let Some(mains) = config.mains_hz {
        let mut sections = Vec::new();
        for harmonic in 1..=config.mains_harmonics.max(1) {
            let f0 = mains * harmonic as f64;
            if f0 > 0.0 && f0 < fs * 0.45 {
                sections.push(Biquad::notch(fs, f0, 10.0));
            }
        }
        if !sections.is_empty() {
            signal = filtfilt_cascade(&signal, &sections);
        }
    }
    if let Some(lowpass) = config.lowpass_hz {
        if lowpass > 0.0 && lowpass < fs * 0.49 {
            signal = filtfilt(
                &signal,
                &Biquad::lowpass(fs, lowpass, std::f64::consts::FRAC_1_SQRT_2),
            );
        }
    }
    signal
}

fn detect_events(signal: &[f64], fs: f64, config: &AnalysisConfig) -> Vec<EventSeed> {
    let n = signal.len();
    if n < 16 {
        return Vec::new();
    }
    let env_window = ((fs * 0.002).round() as usize).max(3);
    let envelope = moving_rms(signal, env_window);
    let median = stats::median(&envelope);
    let sigma = 1.4826 * stats::mad(&envelope);
    let threshold = (median + config.event_threshold_sigma * sigma)
        .max(median * 3.0)
        .max(1e-12);

    // Detect threshold crossings rather than arbitrary envelope maxima: a
    // decaying multi-mode response can have beat lobes, but it only crosses
    // the trigger threshold once per impact.
    let refractory = (config.event_refractory_s * fs).round().max(1.0) as usize;
    let attack_window = ((fs * 0.05).round() as usize).max(1);
    let mut selected: Vec<(usize, f64)> = Vec::new();
    let mut i = 0usize;
    while i < n && selected.len() < config.max_events {
        if envelope[i] <= threshold {
            i += 1;
            continue;
        }
        let attack_end = (i + attack_window).min(n);
        let (peak, amplitude) = envelope[i..attack_end]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(offset, value)| (i + offset, *value))
            .unwrap_or((i, envelope[i]));
        if selected
            .last()
            .is_none_or(|(previous, _)| peak.abs_diff(*previous) >= refractory)
        {
            selected.push((peak, amplitude));
        }
        // Stay in the same impact until its envelope has decayed well below
        // the trigger threshold.
        let mut j = peak + 1;
        while j < n && envelope[j] > threshold * 0.5 {
            j += 1;
        }
        i = j.max(peak + 1);
    }
    if selected.is_empty() {
        let (peak, amplitude) = envelope
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, v)| (i, *v))
            .unwrap_or((0, 0.0));
        if amplitude > 1e-12 {
            selected.push((peak, amplitude));
        } else {
            return Vec::new();
        }
    }
    selected.sort_by_key(|(index, _)| *index);

    let noise_len = ((fs * 0.05) as usize).min(n).max(16);
    let noise_rms = stats::rms(&signal[..noise_len])
        .max(
            stats::median(
                &signal[..noise_len]
                    .iter()
                    .map(|v| v.abs())
                    .collect::<Vec<_>>(),
            ) * 1.4826,
        )
        .max(1e-12);

    let pre = (config.pre_trigger_s * fs).round() as usize;
    let post = (config.post_trigger_s * fs).round() as usize;
    let min_len = ((fs * 0.05).round() as usize).max(16);
    let mut seeds = Vec::with_capacity(selected.len());
    for (peak_index, peak_amplitude) in selected {
        let mut start = peak_index.saturating_sub(pre);
        let end = (peak_index + post).min(n);
        if end.saturating_sub(start) < min_len {
            start = end.saturating_sub(min_len);
        }
        if end <= start + 8 {
            continue;
        }
        let snr_db = 20.0 * (peak_amplitude / noise_rms).max(1e-12).log10();
        seeds.push(EventSeed {
            peak_index,
            start,
            end,
            snr_db,
        });
    }
    seeds
}

fn detrend(signal: &mut [f64]) {
    if signal.len() < 2 {
        return;
    }
    let xs: Vec<f64> = (0..signal.len()).map(|i| i as f64).collect();
    if let Some(fit) = stats::linreg(&xs, signal) {
        for (i, value) in signal.iter_mut().enumerate() {
            *value -= fit.slope * i as f64 + fit.intercept;
        }
    }
}

fn near_mains(frequency_hz: f64, config: &AnalysisConfig) -> bool {
    let Some(mains) = config.mains_hz else {
        return false;
    };
    let tolerance = (frequency_hz * 0.005).max(2.5);
    (1..=config.mains_harmonics.max(1)).any(|harmonic| {
        let f = mains * harmonic as f64;
        (frequency_hz - f).abs() <= tolerance
    })
}

fn pick_peaks(spectrum: &Spectrum, config: &AnalysisConfig, max_frequency_hz: f64) -> Vec<usize> {
    if spectrum.is_empty() {
        return Vec::new();
    }
    let max_db = spectrum
        .magnitude_db
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    if !max_db.is_finite() {
        return Vec::new();
    }
    let floor_db = max_db + config.peak_threshold_db.min(0.0);
    let median_db = stats::median(&spectrum.magnitude_db);
    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for i in 1..spectrum.len().saturating_sub(1) {
        let f = spectrum.freqs_hz[i];
        if f < config.min_frequency_hz || f > max_frequency_hz {
            continue;
        }
        if near_mains(f, config) {
            continue;
        }
        let db = spectrum.magnitude_db[i];
        if db < floor_db || db < median_db + 6.0 {
            continue;
        }
        if db < spectrum.magnitude_db[i - 1] || db < spectrum.magnitude_db[i + 1] {
            continue;
        }
        // Reject shoulders on the flank of a stronger peak.  A genuine mode
        // has a local minimum at least a few dB below it on *both* sides.
        let bin_width = (spectrum.freqs_hz[1] - spectrum.freqs_hz[0]).max(1e-9);
        let window = (((f * 0.03).max(20.0) / bin_width).round() as usize).clamp(3, 512);
        let left = i.saturating_sub(window);
        let right = (i + window + 1).min(spectrum.len());
        let left_floor = spectrum.magnitude_db[left..i]
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        let right_floor = spectrum.magnitude_db[i + 1..right]
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        let prominence = db - left_floor.max(right_floor);
        if prominence < 4.0 {
            continue;
        }
        candidates.push((i, db));
    }
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected = Vec::new();
    for (index, _) in candidates {
        let f = spectrum.freqs_hz[index];
        let separation = config.min_peak_separation_hz.max(f * 0.008);
        if selected
            .iter()
            .all(|other: &usize| (spectrum.freqs_hz[*other] - f).abs() >= separation)
        {
            selected.push(index);
            if selected.len() >= config.max_modes {
                break;
            }
        }
    }
    selected.sort_by(|a, b| {
        spectrum.freqs_hz[*a]
            .partial_cmp(&spectrum.freqs_hz[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    selected
}

fn estimate_damping(
    signal: &[f64],
    fs: f64,
    frequency_hz: f64,
    config: &AnalysisConfig,
) -> DampingEstimate {
    let bandwidth = (frequency_hz * config.damping_bandwidth_fraction)
        .max(config.damping_min_bandwidth_hz)
        .max(2.0 * fs / signal.len().max(1) as f64);
    let q = (frequency_hz / bandwidth).clamp(0.5, 80.0);
    let filtered = filtfilt(signal, &Biquad::bandpass(fs, frequency_hz, q));
    let env_window = ((fs * 0.001).round() as usize).max(3);
    let envelope = smooth_envelope(&hilbert_envelope(&filtered), env_window);
    if envelope.is_empty() {
        return fallback_damping(signal, fs, frequency_hz, config);
    }

    let search_start = ((fs * 0.002) as usize).min(envelope.len().saturating_sub(1));
    let (max_index, max_value) = envelope
        .iter()
        .enumerate()
        .skip(search_start)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, v)| (i, *v))
        .unwrap_or((search_start, 0.0));
    if max_value <= 1e-12 {
        return fallback_damping(signal, fs, frequency_hz, config);
    }

    let skip = ((fs * 0.002) as usize).max(1);
    let fit_start = (max_index + skip).min(envelope.len().saturating_sub(2));
    let floor = max_value * config.decay_floor_fraction.clamp(0.001, 0.5);
    let fit_end = envelope[fit_start..]
        .iter()
        .position(|value| *value <= floor)
        .map(|offset| fit_start + offset)
        .unwrap_or(envelope.len());
    if fit_end.saturating_sub(fit_start) >= 16 {
        let xs: Vec<f64> = (fit_start..fit_end)
            .map(|i| (i - fit_start) as f64 / fs)
            .collect();
        let ys: Vec<f64> = (fit_start..fit_end)
            .map(|i| envelope[i].max(1e-30).ln())
            .collect();
        if let Some(fit) = stats::linreg(&xs, &ys) {
            let zeta = -fit.slope / (2.0 * PI * frequency_hz);
            if fit.r_squared >= config.damping_min_r2 && zeta > 0.0 && zeta < 0.5 {
                return DampingEstimate {
                    damping_ratio: zeta,
                    bandwidth_hz: 2.0 * zeta * frequency_hz,
                    fit_r2: fit.r_squared,
                    method: DampingMethod::EnvelopeFit,
                };
            }
        }
    }
    fallback_damping(signal, fs, frequency_hz, config)
}

fn fallback_damping(
    signal: &[f64],
    fs: f64,
    frequency_hz: f64,
    config: &AnalysisConfig,
) -> DampingEstimate {
    let bandwidth =
        (frequency_hz * config.damping_bandwidth_fraction).max(config.damping_min_bandwidth_hz);
    let q = (frequency_hz / bandwidth).clamp(0.5, 80.0);
    let filtered = filtfilt(signal, &Biquad::bandpass(fs, frequency_hz, q));
    let pad = next_power_of_two(filtered.len() * 2).min(131_072);
    let spectrum = amplitude_spectrum(&filtered, fs, config.window, pad);
    let target = frequency_hz;
    let mut peak_index = 0usize;
    let mut peak_db = f64::NEG_INFINITY;
    for (i, f) in spectrum.freqs_hz.iter().enumerate() {
        if (f - target).abs() < (target * 0.1).max(20.0) && spectrum.magnitude_db[i] > peak_db {
            peak_db = spectrum.magnitude_db[i];
            peak_index = i;
        }
    }
    if peak_db.is_finite() {
        let half = peak_db - 3.0103;
        let mut low = peak_index;
        while low > 0 && spectrum.magnitude_db[low] > half {
            low -= 1;
        }
        let mut high = peak_index;
        while high + 1 < spectrum.len() && spectrum.magnitude_db[high] > half {
            high += 1;
        }
        let f_low = spectrum.freqs_hz[low];
        let f_high = spectrum.freqs_hz[high];
        let measured = (f_high - f_low).max(0.0);
        let zeta = (measured / (2.0 * target)).clamp(1e-5, 0.5);
        return DampingEstimate {
            damping_ratio: zeta,
            bandwidth_hz: measured,
            fit_r2: 0.0,
            method: DampingMethod::HalfPower,
        };
    }
    DampingEstimate {
        damping_ratio: 0.005,
        bandwidth_hz: 0.01 * frequency_hz,
        fit_r2: 0.0,
        method: DampingMethod::Default,
    }
}

fn aggregate_modes(events: &[EventAnalysis], max_modes: usize) -> Vec<ModalFeature> {
    struct Cluster {
        frequencies: Vec<f64>,
        dampings: Vec<(f64, f64)>,
        amplitudes: Vec<f64>,
        events: Vec<usize>,
    }
    let mut estimates: Vec<&ModeEstimate> =
        events.iter().flat_map(|event| event.modes.iter()).collect();
    estimates.sort_by(|a, b| {
        a.frequency_hz
            .partial_cmp(&b.frequency_hz)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut clusters: Vec<Cluster> = Vec::new();
    for estimate in estimates {
        let mut best: Option<usize> = None;
        let mut best_distance = f64::INFINITY;
        for (index, cluster) in clusters.iter().enumerate() {
            let mean = stats::mean(&cluster.frequencies);
            let tolerance = (mean * 0.01).max(4.0);
            let distance = (estimate.frequency_hz - mean).abs();
            if distance <= tolerance && distance < best_distance {
                best = Some(index);
                best_distance = distance;
            }
        }
        match best {
            Some(index) => {
                let cluster = &mut clusters[index];
                cluster.frequencies.push(estimate.frequency_hz);
                let weight = estimate.amplitude.max(0.01) * estimate.fit_r2.max(0.1);
                cluster.dampings.push((estimate.damping_ratio, weight));
                cluster.amplitudes.push(estimate.amplitude);
                cluster.events.push(estimate.event_index);
            }
            None => clusters.push(Cluster {
                frequencies: vec![estimate.frequency_hz],
                dampings: vec![(
                    estimate.damping_ratio,
                    estimate.amplitude.max(0.01) * estimate.fit_r2.max(0.1),
                )],
                amplitudes: vec![estimate.amplitude],
                events: vec![estimate.event_index],
            }),
        }
    }

    let required_support = if events.len() <= 1 {
        1
    } else {
        events.len().div_ceil(2)
    };
    let mut modes: Vec<ModalFeature> = clusters
        .into_iter()
        .filter_map(|cluster| {
            let support = cluster
                .events
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            if support.len() < required_support {
                return None;
            }
            let frequency_hz = stats::mean(&cluster.frequencies);
            let frequency_std_hz = stats::std_dev(&cluster.frequencies);
            let weight_sum: f64 = cluster.dampings.iter().map(|(_, w)| *w).sum();
            let damping_ratio = if weight_sum > 0.0 {
                cluster.dampings.iter().map(|(d, w)| d * w).sum::<f64>() / weight_sum
            } else {
                stats::mean(&cluster.dampings.iter().map(|(d, _)| *d).collect::<Vec<_>>())
            };
            let damping_std = if cluster.dampings.len() > 1 {
                stats::std_dev(&cluster.dampings.iter().map(|(d, _)| *d).collect::<Vec<_>>())
            } else {
                0.0
            };
            let amplitude = stats::mean(&cluster.amplitudes);
            let persistence = support.len() as f64 / events.len().max(1) as f64;
            Some(ModalFeature {
                frequency_hz,
                frequency_std_hz,
                damping_ratio,
                damping_std,
                amplitude,
                persistence,
                q_factor: if damping_ratio > 0.0 {
                    1.0 / (2.0 * damping_ratio)
                } else {
                    0.0
                },
            })
        })
        .collect();

    modes.sort_by(|a, b| {
        (b.amplitude * b.persistence)
            .partial_cmp(&(a.amplitude * a.persistence))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    modes.truncate(max_modes);
    modes.sort_by(|a, b| {
        a.frequency_hz
            .partial_cmp(&b.frequency_hz)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    modes
}

#[allow(clippy::too_many_arguments)]
fn build_features(
    filtered: &[f64],
    events: &[EventAnalysis],
    modes: &[ModalFeature],
    spectrum: &Spectrum,
    fs: f64,
    config: &AnalysisConfig,
    max_frequency_hz: f64,
) -> FeatureSet {
    let mut features = FeatureSet {
        mode_count: modes.len(),
        event_count: events.len(),
        ..Default::default()
    };
    if modes.is_empty() {
        return features;
    }
    // Ignore very weak low-frequency clutter when choosing the reference
    // fundamental; a packet-gap click can otherwise masquerade as a new mode.
    let fundamental = modes
        .iter()
        .find(|mode| mode.amplitude * mode.persistence >= 0.15)
        .unwrap_or(&modes[0])
        .frequency_hz;
    features.fundamental_hz = fundamental;
    features.modal_frequencies_hz = modes.iter().map(|m| m.frequency_hz).collect();
    features.modal_damping = modes.iter().map(|m| m.damping_ratio).collect();
    features.frequency_ratios = modes
        .iter()
        .skip(1)
        .map(|m| {
            if fundamental > 0.0 {
                m.frequency_hz / fundamental
            } else {
                0.0
            }
        })
        .collect();
    features.mean_damping = modes
        .iter()
        .map(|m| m.damping_ratio * m.persistence)
        .sum::<f64>()
        / modes.iter().map(|m| m.persistence).sum::<f64>().max(1e-9);
    features.max_damping = modes.iter().map(|m| m.damping_ratio).fold(0.0, f64::max);
    let descriptors = spectral_descriptors(spectrum, config.min_frequency_hz, max_frequency_hz);
    features.spectral_centroid_hz = descriptors.centroid_hz;
    features.spectral_spread_hz = descriptors.spread_hz;
    features.spectral_rolloff95_hz = descriptors.rolloff95_hz;
    features.spectral_flatness = descriptors.flatness;
    features.spectral_tilt_db_per_octave = descriptors.tilt_db_per_octave;
    features.high_freq_energy_ratio = high_frequency_ratio(
        spectrum,
        (2.0 * fundamental).max(1000.0),
        config.min_frequency_hz,
        max_frequency_hz,
    );
    features.decay_t60_s = if fundamental > 0.0 && features.mean_damping > 0.0 {
        (6.907_755 / (2.0 * PI * fundamental * features.mean_damping)).clamp(0.0, 60.0)
    } else {
        0.0
    };
    features.mode_splitting_score = mode_splitting_score(modes);
    features.peak_amplitude = stats::peak_abs(filtered);
    features.rms = stats::rms(filtered);
    features.crest_factor = if features.rms > 0.0 {
        features.peak_amplitude / features.rms
    } else {
        0.0
    };
    features.trigger_snr_db =
        stats::mean(&events.iter().map(|event| event.snr_db).collect::<Vec<_>>());
    let _ = fs;
    features
}

fn high_frequency_ratio(spectrum: &Spectrum, cutoff_hz: f64, f_min: f64, f_max: f64) -> f64 {
    let mut total = 0.0;
    let mut high = 0.0;
    for i in 0..spectrum.len() {
        let f = spectrum.freqs_hz[i];
        if f < f_min || f > f_max {
            continue;
        }
        let p = spectrum.magnitude[i] * spectrum.magnitude[i];
        total += p;
        if f >= cutoff_hz {
            high += p;
        }
    }
    if total > 0.0 {
        (high / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn mode_splitting_score(modes: &[ModalFeature]) -> f64 {
    let mut score: f64 = 0.0;
    for i in 0..modes.len() {
        for j in i + 1..modes.len() {
            let a = &modes[i];
            let b = &modes[j];
            if a.frequency_hz <= 0.0 || b.frequency_hz <= 0.0 {
                continue;
            }
            let spacing = (b.frequency_hz - a.frequency_hz).abs() / a.frequency_hz;
            let strong = |mode: &ModalFeature| {
                mode.persistence >= 0.75 && mode.amplitude * mode.persistence >= 0.12
            };
            if spacing <= 0.03 && strong(a) && strong(b) {
                let candidate = (1.0 - spacing / 0.03) * a.persistence.min(b.persistence);
                score = score.max(candidate);
            }
        }
    }
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Capture;

    fn synthetic_modes(fs: f64, duration_s: f64, modes: &[(f64, f64, f64)]) -> Capture {
        let n = (fs * duration_s) as usize;
        let tap = (0.2 * fs) as usize;
        let mut samples = vec![0.0f32; n];
        for (freq, zeta, amplitude) in modes {
            for (i, sample) in samples.iter_mut().enumerate().skip(tap) {
                let t = (i - tap) as f64 / fs;
                let value =
                    amplitude * (-zeta * 2.0 * PI * freq * t).exp() * (2.0 * PI * freq * t).cos();
                *sample += value as f32;
            }
        }
        let peak = samples.iter().fold(0.0f32, |a, v| a.max(v.abs())).max(1e-9);
        for value in &mut samples {
            *value /= peak;
        }
        Capture::new(fs, samples, "synthetic-test")
    }

    #[test]
    fn recovers_modal_frequencies_and_damping() {
        let fs = 48_000.0;
        let capture = synthetic_modes(
            fs,
            1.2,
            &[
                (320.0, 0.004, 1.0),
                (760.0, 0.003, 0.7),
                (1250.0, 0.006, 0.4),
            ],
        );
        let analysis = analyze_capture(&capture, &AnalysisConfig::default());
        assert_eq!(analysis.events.len(), 1, "events: {:?}", analysis.events);
        assert!(analysis.quality.usable, "quality: {:?}", analysis.quality);
        let freqs: Vec<f64> = analysis.modes.iter().map(|m| m.frequency_hz).collect();
        assert!(freqs.len() >= 3, "modes: {freqs:?}");
        for (expected, actual) in [320.0, 760.0, 1250.0].iter().zip(freqs.iter()) {
            assert!(
                (actual - expected).abs() / expected < 0.03,
                "expected {expected}, got {actual}"
            );
        }
        let zeta = analysis.modes[0].damping_ratio;
        assert!(zeta > 0.001 && zeta < 0.02, "zeta {zeta}");
    }

    #[test]
    fn no_signal_is_unusable() {
        let capture = Capture::new(48_000.0, vec![0.0; 48_000], "silence");
        let analysis = analyze_capture(&capture, &AnalysisConfig::default());
        assert!(analysis.events.is_empty());
        assert!(!analysis.quality.usable);
    }

    #[test]
    fn detects_multiple_taps() {
        let fs = 48_000.0;
        let n = (fs * 3.0) as usize;
        let mut samples = vec![0.0f32; n];
        for tap_time in [0.2, 1.0, 1.8] {
            let tap = (tap_time * fs) as usize;
            for (i, sample) in samples.iter_mut().enumerate().skip(tap) {
                let t = (i - tap) as f64 / fs;
                *sample +=
                    ((-0.005 * 2.0 * PI * 400.0 * t).exp() * (2.0 * PI * 400.0 * t).cos()) as f32;
            }
        }
        let capture = Capture::new(fs, samples, "taps");
        let analysis = analyze_capture(&capture, &AnalysisConfig::default());
        assert!(
            analysis.events.len() >= 3,
            "events {}",
            analysis.events.len()
        );
    }
}
