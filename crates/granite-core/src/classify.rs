//! Reference profiles, robust anomaly scoring and condition classification.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::analysis::{CaptureAnalysis, FeatureSet};
use crate::time::now_unix_s;

/// Slab identity stored with a reference profile.
#[derive(Clone, Debug, PartialEq)]
pub struct SlabDescriptor {
    pub length_m: f64,
    pub width_m: f64,
    pub thickness_m: f64,
    pub material: String,
}

/// A reference mode and its allowed variation.
#[derive(Clone, Debug)]
pub struct ReferenceMode {
    pub frequency_hz: f64,
    pub frequency_tol_hz: f64,
    pub damping_ratio: f64,
    pub damping_tol: f64,
}

/// A reference scalar feature and its standard deviation.
#[derive(Clone, Debug)]
pub struct ReferenceFeature {
    pub key: String,
    pub mean: f64,
    pub std: f64,
}

/// A healthy-slab baseline, measured from one or more pristine captures.
#[derive(Clone, Debug)]
pub struct ReferenceProfile {
    pub name: String,
    pub sample_rate_hz: f64,
    pub created_unix_s: u64,
    pub slab: Option<SlabDescriptor>,
    pub modes: Vec<ReferenceMode>,
    pub features: Vec<ReferenceFeature>,
}

impl ReferenceProfile {
    pub fn feature(&self, key: &str) -> Option<&ReferenceFeature> {
        self.features.iter().find(|feature| feature.key == key)
    }

    /// Serialise to a compact, line-oriented text format.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("granite-reference 1\n");
        out.push_str(&format!("name={}\n", self.name.replace('\n', " ")));
        out.push_str(&format!("sample_rate_hz={:.6}\n", self.sample_rate_hz));
        out.push_str(&format!("created_unix_s={}\n", self.created_unix_s));
        if let Some(slab) = &self.slab {
            out.push_str(&format!(
                "slab={:.6},{:.6},{:.6},{}\n",
                slab.length_m, slab.width_m, slab.thickness_m, slab.material
            ));
        }
        for mode in &self.modes {
            out.push_str(&format!(
                "mode={:.6},{:.8},{:.6},{:.8}\n",
                mode.frequency_hz, mode.damping_ratio, mode.frequency_tol_hz, mode.damping_tol
            ));
        }
        for feature in &self.features {
            out.push_str(&format!(
                "feature={},{:.8},{:.8}\n",
                feature.key, feature.mean, feature.std
            ));
        }
        out
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("").trim();
        if header != "granite-reference 1" {
            return Err("not a granite-reference v1 profile".to_string());
        }
        let mut profile = ReferenceProfile {
            name: "reference".to_string(),
            sample_rate_hz: 48_000.0,
            created_unix_s: 0,
            slab: None,
            modes: Vec::new(),
            features: Vec::new(),
        };
        for (line_number, raw) in lines.enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: missing '='", line_number + 2))?;
            match key {
                "name" => profile.name = value.to_string(),
                "sample_rate_hz" => {
                    profile.sample_rate_hz = parse_f64(value, line_number + 2)?;
                }
                "created_unix_s" => {
                    profile.created_unix_s = value
                        .parse()
                        .map_err(|_| format!("line {}: bad timestamp", line_number + 2))?;
                }
                "slab" => {
                    let parts: Vec<&str> = value.split(',').collect();
                    if parts.len() != 4 {
                        return Err(format!("line {}: bad slab", line_number + 2));
                    }
                    profile.slab = Some(SlabDescriptor {
                        length_m: parse_f64(parts[0], line_number + 2)?,
                        width_m: parse_f64(parts[1], line_number + 2)?,
                        thickness_m: parse_f64(parts[2], line_number + 2)?,
                        material: parts[3].to_string(),
                    });
                }
                "mode" => {
                    let parts: Vec<&str> = value.split(',').collect();
                    if parts.len() != 4 {
                        return Err(format!("line {}: bad mode", line_number + 2));
                    }
                    profile.modes.push(ReferenceMode {
                        frequency_hz: parse_f64(parts[0], line_number + 2)?,
                        damping_ratio: parse_f64(parts[1], line_number + 2)?,
                        frequency_tol_hz: parse_f64(parts[2], line_number + 2)?,
                        damping_tol: parse_f64(parts[3], line_number + 2)?,
                    });
                }
                "feature" => {
                    let parts: Vec<&str> = value.split(',').collect();
                    if parts.len() != 3 {
                        return Err(format!("line {}: bad feature", line_number + 2));
                    }
                    profile.features.push(ReferenceFeature {
                        key: parts[0].to_string(),
                        mean: parse_f64(parts[1], line_number + 2)?,
                        std: parse_f64(parts[2], line_number + 2)?,
                    });
                }
                _ => {
                    // Forward-compatible: ignore unknown keys.
                }
            }
        }
        if profile.modes.is_empty() {
            return Err("reference profile has no modes".to_string());
        }
        Ok(profile)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fs::write(path, self.to_text())
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        Self::from_text(&text)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
    }
}

fn parse_f64(value: &str, line: usize) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("line {line}: bad number '{value}'"))
}

/// Build a baseline from a healthy capture.
pub fn build_reference(
    name: impl Into<String>,
    analysis: &CaptureAnalysis,
    slab: Option<SlabDescriptor>,
) -> Result<ReferenceProfile, String> {
    if analysis.modes.is_empty() {
        return Err("cannot build a reference from a capture with no stable modes".to_string());
    }
    // Weak modes close to the noise floor are not reliable enough to be
    // reference features; including them would turn normal peak-picking
    // variation into false "missing mode" penalties.
    let mut reference_modes: Vec<_> = analysis
        .modes
        .iter()
        .filter(|mode| mode.amplitude * mode.persistence >= 0.12)
        .collect();
    if reference_modes.len() < 2 {
        reference_modes = analysis.modes.iter().collect();
    }
    reference_modes.truncate(8);
    let mut modes = Vec::new();
    for mode in reference_modes {
        modes.push(ReferenceMode {
            frequency_hz: mode.frequency_hz,
            frequency_tol_hz: (mode.frequency_hz * 0.02).max(5.0),
            damping_ratio: mode.damping_ratio,
            damping_tol: (mode.damping_ratio * 0.25).max(0.001),
        });
    }
    let feature_floor = |key: &str, value: f64| -> f64 {
        match key {
            "fundamental_hz" => (value * 0.02).max(5.0),
            "mean_damping" | "max_damping" => (value * 0.25).max(0.001),
            "spectral_centroid_hz" | "spectral_rolloff95_hz" => (value * 0.05).max(10.0),
            "spectral_flatness" => (value * 0.25).max(0.01),
            "spectral_tilt_db_per_octave" => 2.0,
            "decay_t60_s" => (value * 0.20).max(0.01),
            "high_freq_energy_ratio" => (value * 0.20).max(0.01),
            "mode_splitting_score" => 0.08,
            "crest_factor" => (value * 0.10).max(0.1),
            _ => (value.abs() * 0.10).max(0.01),
        }
    };
    let keys = [
        "fundamental_hz",
        "mean_damping",
        "max_damping",
        "spectral_centroid_hz",
        "spectral_rolloff95_hz",
        "spectral_flatness",
        "spectral_tilt_db_per_octave",
        "decay_t60_s",
        "high_freq_energy_ratio",
        "mode_splitting_score",
        "crest_factor",
    ];
    let mut features = Vec::new();
    for key in keys {
        if let Some(value) = analysis.features.get(key) {
            features.push(ReferenceFeature {
                key: key.to_string(),
                mean: value,
                std: feature_floor(key, value),
            });
        }
    }
    Ok(ReferenceProfile {
        name: name.into(),
        sample_rate_hz: analysis.sample_rate_hz,
        created_unix_s: now_unix_s(),
        slab,
        modes,
        features,
    })
}

/// Overall condition label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Condition {
    Healthy,
    Suspect,
    Defect,
    Unknown,
}

impl Condition {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Suspect => "SUSPECT",
            Self::Defect => "DEFECT",
            Self::Unknown => "UNKNOWN",
        }
    }

    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Suspect => "suspect",
            Self::Defect => "defect",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One line of the assessment table.
#[derive(Clone, Debug)]
pub struct AssessmentMetric {
    pub name: String,
    pub value: f64,
    pub expected: f64,
    pub z_score: f64,
    pub weight: f64,
    pub unit: String,
}

/// Classification result.
#[derive(Clone, Debug)]
pub struct Assessment {
    pub condition: Condition,
    pub anomaly_score: f64,
    pub confidence: f64,
    pub flags: Vec<String>,
    pub explanations: Vec<String>,
    pub metrics: Vec<AssessmentMetric>,
    pub reference_name: String,
}

/// Scoring thresholds and weights.
#[derive(Clone, Debug)]
pub struct ClassifierConfig {
    pub healthy_max: f64,
    pub suspect_max: f64,
    pub mode_frequency_weight: f64,
    pub mode_damping_weight: f64,
    pub feature_weight: f64,
    pub missing_mode_penalty: f64,
    pub extra_mode_penalty: f64,
    pub score_scale: f64,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            healthy_max: 30.0,
            suspect_max: 55.0,
            mode_frequency_weight: 1.0,
            mode_damping_weight: 0.9,
            feature_weight: 0.55,
            missing_mode_penalty: 2.5,
            extra_mode_penalty: 1.5,
            score_scale: 2.5,
        }
    }
}

/// Compare an analysis against a healthy reference and label the slab.
pub fn classify(
    analysis: &CaptureAnalysis,
    reference: &ReferenceProfile,
    config: &ClassifierConfig,
) -> Assessment {
    if analysis.modes.is_empty() || reference.modes.is_empty() {
        return Assessment {
            condition: Condition::Unknown,
            anomaly_score: 100.0,
            confidence: 0.0,
            flags: vec!["insufficient modal data".to_string()],
            explanations: vec![
                "The capture did not contain enough stable modes to compare with the reference."
                    .to_string(),
            ],
            metrics: Vec::new(),
            reference_name: reference.name.clone(),
        };
    }

    let mut metrics = Vec::new();
    let mut flags = Vec::new();
    let mut explanations = Vec::new();
    let mut weighted_squares = 0.0;
    let mut weight_sum = 0.0;
    let mut matched_analysis = vec![false; analysis.modes.len()];
    let mut frequency_shifts = Vec::new();
    let mut damping_ratios = Vec::new();

    for (index, reference_mode) in reference.modes.iter().enumerate() {
        let mut best: Option<usize> = None;
        let mut best_distance = f64::INFINITY;
        for (candidate_index, mode) in analysis.modes.iter().enumerate() {
            if matched_analysis[candidate_index] {
                continue;
            }
            let distance = (mode.frequency_hz - reference_mode.frequency_hz).abs();
            if distance < best_distance {
                best_distance = distance;
                best = Some(candidate_index);
            }
        }
        let weight = 1.0 / (1.0 + 0.35 * index as f64);
        let tolerance = reference_mode
            .frequency_tol_hz
            .max(reference_mode.frequency_hz * 0.10)
            .max(30.0);
        match best {
            Some(candidate) if best_distance <= tolerance => {
                matched_analysis[candidate] = true;
                let mode = &analysis.modes[candidate];
                let frequency_z = best_distance / reference_mode.frequency_tol_hz.max(1e-6);
                let damping_z = ((mode.damping_ratio - reference_mode.damping_ratio)
                    / reference_mode.damping_tol.max(1e-9))
                .max(0.0);
                frequency_shifts.push(
                    (mode.frequency_hz - reference_mode.frequency_hz) / reference_mode.frequency_hz,
                );
                if reference_mode.damping_ratio > 0.0 {
                    damping_ratios.push(mode.damping_ratio / reference_mode.damping_ratio);
                }
                for (z, w, name, value, expected, unit) in [
                    (
                        frequency_z,
                        config.mode_frequency_weight * weight,
                        "mode frequency",
                        mode.frequency_hz,
                        reference_mode.frequency_hz,
                        "Hz",
                    ),
                    (
                        damping_z,
                        config.mode_damping_weight * weight,
                        "mode damping",
                        mode.damping_ratio,
                        reference_mode.damping_ratio,
                        "",
                    ),
                ] {
                    weighted_squares += w * z.min(10.0) * z.min(10.0);
                    weight_sum += w;
                    metrics.push(AssessmentMetric {
                        name: format!("{name} #{}", index + 1),
                        value,
                        expected,
                        z_score: z,
                        weight: w,
                        unit: unit.to_string(),
                    });
                }
            }
            _ => {
                let z = config.missing_mode_penalty;
                weighted_squares += weight * z * z;
                weight_sum += weight;
                metrics.push(AssessmentMetric {
                    name: format!("missing mode #{}", index + 1),
                    value: 0.0,
                    expected: reference_mode.frequency_hz,
                    z_score: z,
                    weight,
                    unit: "Hz".to_string(),
                });
                flags.push(format!(
                    "reference mode at {:.1} Hz was not found",
                    reference_mode.frequency_hz
                ));
            }
        }
    }

    for (index, mode) in analysis.modes.iter().enumerate() {
        if matched_analysis[index] {
            continue;
        }
        if mode.amplitude * mode.persistence < 0.15 {
            continue;
        }
        let weight = config.feature_weight * 0.5;
        let z = config.extra_mode_penalty;
        weighted_squares += weight * z * z;
        weight_sum += weight;
        metrics.push(AssessmentMetric {
            name: "unexpected mode".to_string(),
            value: mode.frequency_hz,
            expected: 0.0,
            z_score: z,
            weight,
            unit: "Hz".to_string(),
        });
        flags.push(format!(
            "unexpected mode at {:.1} Hz (persistence {:.0}%)",
            mode.frequency_hz,
            mode.persistence * 100.0
        ));
    }

    for reference_feature in &reference.features {
        let Some(value) = analysis.features.get(&reference_feature.key) else {
            continue;
        };
        let raw_z = (value - reference_feature.mean) / reference_feature.std.max(1e-9);
        let z = match reference_feature.key.as_str() {
            "mean_damping" | "max_damping" | "high_freq_energy_ratio" | "mode_splitting_score" => {
                raw_z.max(0.0)
            }
            "decay_t60_s" => (-raw_z).max(0.0),
            _ => raw_z.abs(),
        };
        let weight = config.feature_weight;
        weighted_squares += weight * z.min(10.0) * z.min(10.0);
        weight_sum += weight;
        metrics.push(AssessmentMetric {
            name: reference_feature.key.clone(),
            value,
            expected: reference_feature.mean,
            z_score: z,
            weight,
            unit: String::new(),
        });
    }

    let z_rms = if weight_sum > 0.0 {
        (weighted_squares / weight_sum).sqrt()
    } else {
        0.0
    };
    let anomaly_score = 100.0 * z_rms / (z_rms + config.score_scale.max(0.1));
    let condition = if anomaly_score < config.healthy_max {
        Condition::Healthy
    } else if anomaly_score < config.suspect_max {
        Condition::Suspect
    } else {
        Condition::Defect
    };

    let mean_shift = if frequency_shifts.is_empty() {
        0.0
    } else {
        frequency_shifts.iter().sum::<f64>() / frequency_shifts.len() as f64
    };
    let mean_damping_ratio = if damping_ratios.is_empty() {
        0.0
    } else {
        damping_ratios.iter().sum::<f64>() / damping_ratios.len() as f64
    };
    if mean_shift.abs() > 0.02 {
        flags.push(format!(
            "modal frequencies shifted {:.1}% {}",
            mean_shift.abs() * 100.0,
            if mean_shift < 0.0 {
                "downward"
            } else {
                "upward"
            }
        ));
        explanations.push(if mean_shift < 0.0 {
            "A downward frequency shift is consistent with reduced bending stiffness (crack, void or thickness loss).".to_string()
        } else {
            "An upward frequency shift is consistent with a stiffer or thinner-than-reference slab.".to_string()
        });
    }
    if mean_damping_ratio > 1.5 {
        flags.push(format!(
            "damping increased {:.1}x versus reference",
            mean_damping_ratio
        ));
        explanations.push(
            "Higher damping is consistent with a crack or delamination dissipating energy."
                .to_string(),
        );
    }
    if analysis.features.mode_splitting_score > 0.15 {
        flags.push(format!(
            "closely spaced modes detected (splitting score {:.2})",
            analysis.features.mode_splitting_score
        ));
        explanations.push(
            "Mode splitting can indicate a local crack or delamination that breaks the slab's symmetry."
                .to_string(),
        );
    }
    if let Some(reference_fundamental) = reference.modes.first().map(|m| m.frequency_hz) {
        if let Some(low) = analysis.modes.iter().find(|m| {
            m.frequency_hz < reference_fundamental * 0.8 && m.amplitude * m.persistence > 0.15
        }) {
            flags.push(format!(
                "low-frequency mode at {:.1} Hz (reference fundamental {:.1} Hz)",
                low.frequency_hz, reference_fundamental
            ));
            explanations.push(
                "A new low-frequency mode is a classic signature of a delaminated layer."
                    .to_string(),
            );
        }
    }
    for warning in &analysis.quality.warnings {
        flags.push(format!("data quality: {warning}"));
    }
    if flags.is_empty() {
        flags.push("all modal features within the reference envelope".to_string());
        explanations.push(
            "No significant frequency shift, damping increase or mode splitting was detected."
                .to_string(),
        );
    }

    let nearest_threshold = if anomaly_score < config.healthy_max {
        config.healthy_max
    } else if anomaly_score < config.suspect_max {
        (anomaly_score - config.healthy_max).min(config.suspect_max - anomaly_score)
            + config.healthy_max
    } else {
        config.suspect_max
    };
    let distance = (anomaly_score - nearest_threshold).abs();
    let confidence = (0.5 + distance / 40.0).clamp(0.5, 0.99);
    let confidence = if analysis.quality.usable {
        confidence
    } else {
        (confidence * 0.6).min(0.6)
    };

    Assessment {
        condition,
        anomaly_score,
        confidence,
        flags,
        explanations,
        metrics,
        reference_name: reference.name.clone(),
    }
}

/// Convenience wrapper that builds a reference and classifies the same data
/// (used only for quick demos; a real baseline should be measured separately).
pub fn self_reference(name: &str, analysis: &CaptureAnalysis) -> ReferenceProfile {
    build_reference(name, analysis, None).unwrap_or_else(|_| ReferenceProfile {
        name: name.to_string(),
        sample_rate_hz: analysis.sample_rate_hz,
        created_unix_s: now_unix_s(),
        slab: None,
        modes: Vec::new(),
        features: Vec::new(),
    })
}

/// Feature set accessor used by tests and reports.
pub fn feature_value(features: &FeatureSet, key: &str) -> Option<f64> {
    features.get(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{CaptureAnalysis, FeatureSet, ModalFeature, QualityMetrics};
    use crate::dsp::Spectrum;
    use crate::dsp::WindowKind;

    fn analysis_with(frequencies: &[(f64, f64)], damping: f64) -> CaptureAnalysis {
        let modes: Vec<ModalFeature> = frequencies
            .iter()
            .map(|(f, a)| ModalFeature {
                frequency_hz: *f,
                frequency_std_hz: 0.0,
                damping_ratio: damping,
                damping_std: 0.0,
                amplitude: *a,
                persistence: 1.0,
                q_factor: 1.0 / (2.0 * damping),
            })
            .collect();
        let fundamental = modes.first().map(|m| m.frequency_hz).unwrap_or(0.0);
        let features = FeatureSet {
            fundamental_hz: fundamental,
            modal_frequencies_hz: modes.iter().map(|m| m.frequency_hz).collect(),
            modal_damping: modes.iter().map(|m| m.damping_ratio).collect(),
            frequency_ratios: Vec::new(),
            mean_damping: damping,
            max_damping: damping,
            spectral_centroid_hz: fundamental * 2.0,
            spectral_spread_hz: fundamental,
            spectral_rolloff95_hz: fundamental * 3.0,
            spectral_flatness: 0.05,
            spectral_tilt_db_per_octave: -8.0,
            decay_t60_s: 6.907 / (2.0 * std::f64::consts::PI * fundamental * damping),
            high_freq_energy_ratio: 0.1,
            mode_splitting_score: 0.0,
            peak_amplitude: 1.0,
            rms: 0.2,
            crest_factor: 5.0,
            mode_count: modes.len(),
            event_count: 1,
            trigger_snr_db: 40.0,
        };
        CaptureAnalysis {
            sample_rate_hz: 48_000.0,
            duration_s: 1.0,
            events: Vec::new(),
            modes,
            features,
            quality: QualityMetrics {
                usable: true,
                ..Default::default()
            },
            averaged_spectrum: Spectrum {
                sample_rate_hz: 48_000.0,
                freqs_hz: Vec::new(),
                magnitude: Vec::new(),
                magnitude_db: Vec::new(),
                enbw_hz: 0.0,
                window: WindowKind::Hann,
            },
        }
    }

    #[test]
    fn healthy_matches_reference() {
        let healthy = analysis_with(&[(320.0, 1.0), (760.0, 0.7)], 0.004);
        let reference = build_reference("healthy", &healthy, None).unwrap();
        let assessment = classify(&healthy, &reference, &ClassifierConfig::default());
        assert_eq!(assessment.condition, Condition::Healthy);
    }

    #[test]
    fn shifted_and_damped_is_defect() {
        let healthy = analysis_with(&[(320.0, 1.0), (760.0, 0.7)], 0.004);
        let reference = build_reference("healthy", &healthy, None).unwrap();
        let damaged = analysis_with(&[(290.0, 1.0), (700.0, 0.7)], 0.02);
        let assessment = classify(&damaged, &reference, &ClassifierConfig::default());
        assert_eq!(assessment.condition, Condition::Defect, "{assessment:?}");
    }

    #[test]
    fn profile_roundtrip() {
        let healthy = analysis_with(&[(320.0, 1.0), (760.0, 0.7)], 0.004);
        let reference = build_reference("healthy slab", &healthy, None).unwrap();
        let text = reference.to_text();
        let parsed = ReferenceProfile::from_text(&text).unwrap();
        assert_eq!(parsed.name, "healthy slab");
        assert_eq!(parsed.modes.len(), 2);
        assert!((parsed.modes[0].frequency_hz - 320.0).abs() < 1e-6);
    }
}
