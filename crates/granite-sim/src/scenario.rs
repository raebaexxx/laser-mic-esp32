//! Scenario catalogue and end-to-end digital-twin runner.

use std::fmt;

use granite_core::analysis::{analyze_capture, AnalysisConfig, CaptureAnalysis};
use granite_core::capture::Capture;
use granite_core::classify::{
    build_reference, classify, Assessment, ClassifierConfig, Condition, ReferenceProfile,
    SlabDescriptor,
};
use granite_core::report::GroundTruth;
use granite_core::rng::Rng;
use granite_core::stream::StreamError;

use crate::defect::{Defect, DefectKind};
use crate::material::{Material, GRANITE};
use crate::network::{receive, transmit, NetworkConfig};
use crate::plate::{PlateGeometry, PlateMode, PlateModel, Point2};
use crate::signal::{dominant_modes, synthesize, SensorConfig, Tap};

/// A fully specified simulation case.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    pub description: String,
    pub geometry: PlateGeometry,
    pub material: Material,
    pub sensor_position: Point2,
    pub impact_position: Point2,
    pub defect: Defect,
    pub taps: Vec<Tap>,
    pub duration_s: f64,
    pub sample_rate_hz: f64,
    pub sensor: SensorConfig,
    pub network: NetworkConfig,
    pub max_frequency_hz: f64,
    pub expected: Condition,
    pub notes: Vec<String>,
}

impl Scenario {
    pub fn slab_descriptor(&self) -> SlabDescriptor {
        SlabDescriptor {
            length_m: self.geometry.length_m,
            width_m: self.geometry.width_m,
            thickness_m: self.geometry.thickness_m,
            material: self.material.name.to_string(),
        }
    }

    pub fn analysis_config(&self) -> AnalysisConfig {
        AnalysisConfig {
            min_frequency_hz: 40.0,
            max_frequency_hz: Some(
                (self.sample_rate_hz * 0.45).min(self.max_frequency_hz.max(1000.0)),
            ),
            event_threshold_sigma: 7.0,
            post_trigger_s: 0.70,
            max_events: 6,
            ..Default::default()
        }
    }
}

/// Result of simulating and analysing one scenario.
#[derive(Clone, Debug)]
pub struct ScenarioRun {
    pub scenario: Scenario,
    pub seed: u64,
    pub capture: Capture,
    pub analysis: CaptureAnalysis,
    pub reference: ReferenceProfile,
    pub assessment: Assessment,
    pub truth: GroundTruth,
    pub invalid_packets: u64,
    pub datagrams: usize,
    pub simulated_modes: Vec<PlateMode>,
}

#[derive(Clone, Debug)]
pub enum SimError {
    Stream(StreamError),
    NoModes(String),
    EmptyCapture,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stream(error) => write!(f, "stream error: {error}"),
            Self::NoModes(message) => write!(f, "{message}"),
            Self::EmptyCapture => write!(f, "simulation produced an empty capture"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<StreamError> for SimError {
    fn from(value: StreamError) -> Self {
        Self::Stream(value)
    }
}

fn nominal_geometry() -> PlateGeometry {
    PlateGeometry::nominal_granite()
}

fn nominal_taps() -> Vec<Tap> {
    vec![
        Tap::new(0.20, 1.00, 3000.0),
        Tap::new(0.95, 0.92, 3200.0),
        Tap::new(1.70, 0.96, 2900.0),
    ]
}

fn base_scenario(id: &str, title: &str, description: &str) -> Scenario {
    Scenario {
        id: id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        geometry: nominal_geometry(),
        material: GRANITE,
        sensor_position: Point2::new(0.18, 0.14),
        impact_position: Point2::new(0.33, 0.22),
        defect: Defect::healthy(),
        taps: nominal_taps(),
        duration_s: 2.4,
        sample_rate_hz: 48_000.0,
        sensor: SensorConfig::default(),
        network: NetworkConfig::clean(),
        max_frequency_hz: 8000.0,
        expected: Condition::Healthy,
        notes: Vec::new(),
    }
}

/// All built-in validation scenarios.
pub fn catalog() -> Vec<Scenario> {
    let mut scenarios = Vec::new();

    let mut healthy = base_scenario(
        "healthy",
        "Healthy granite slab",
        "Nominal 600 x 400 x 20 mm granite slab, three hammer taps, clean UDP link and a 38 dB SNR laser vibrometer.",
    );
    healthy.notes.push(
        "This case establishes the healthy modal baseline used by the other scenarios.".to_string(),
    );
    scenarios.push(healthy);

    let mut noisy = base_scenario(
        "healthy-noisy",
        "Healthy slab, noisy laser return",
        "Healthy slab with a diffuse granite surface: 22 dB SNR, 1/f noise, mains pickup and light packet impairment.",
    );
    noisy.sensor = SensorConfig::noisy(22.0);
    noisy.network = NetworkConfig::impaired(0.004, 1.5);
    noisy.expected = Condition::Healthy;
    noisy.notes.push(
        "Tests whether the trigger and peak picker still find the same modes when the laser return is weak."
            .to_string(),
    );
    scenarios.push(noisy);

    let mut lossy = base_scenario(
        "lossy-link",
        "Healthy slab, congested WiFi",
        "Healthy slab with 3% packet loss, 4 ms jitter and reordering; the reassembler must preserve the modal estimate.",
    );
    lossy.network = NetworkConfig::impaired(0.03, 4.0);
    lossy.sensor = SensorConfig::noisy(32.0);
    lossy.expected = Condition::Healthy;
    lossy.notes.push(
        "Packet gaps are zero-filled and counted; the report should show a data-quality warning without a false defect."
            .to_string(),
    );
    scenarios.push(lossy);

    let mut crack = base_scenario(
        "crack",
        "Surface-breaking crack",
        "A 55%-severity crack lowers bending stiffness by about 3%, triples modal damping and splits the lowest modes.",
    );
    crack.defect = Defect::new(DefectKind::Crack, 0.55, Point2::new(0.36, 0.19));
    crack.expected = Condition::Defect;
    crack.notes.push(
        "Expected signature: downward frequency shift, higher damping and closely spaced mode pairs."
            .to_string(),
    );
    scenarios.push(crack);

    let mut delamination = base_scenario(
        "delamination",
        "Near-surface delamination",
        "A 70%-severity delamination adds a low-frequency layer mode around 120 Hz and breaks modal symmetry.",
    );
    delamination.defect = Defect::new(DefectKind::Delamination, 0.70, Point2::new(0.25, 0.30));
    delamination.expected = Condition::Defect;
    delamination.notes.push(
        "Expected signature: a new mode well below the reference fundamental, plus damping increase."
            .to_string(),
    );
    scenarios.push(delamination);

    let mut void = base_scenario(
        "void",
        "Internal void",
        "A 75%-severity internal void lowers effective stiffness and roughly doubles modal damping without strong mode splitting.",
    );
    void.defect = Defect::new(DefectKind::Void, 0.90, Point2::new(0.30, 0.25));
    void.expected = Condition::Defect;
    scenarios.push(void);

    let mut thickness = base_scenario(
        "thickness-plus",
        "Thicker-than-reference slab",
        "A slab about 3% thicker than the reference is not cracked, but its modes are systematically higher.",
    );
    thickness.defect = Defect::new(DefectKind::ThicknessVariation, 0.8, Point2::new(0.30, 0.20));
    thickness.expected = Condition::Suspect;
    thickness.notes.push(
        "This is a reference-mismatch case: the classifier should flag it rather than call it healthy."
            .to_string(),
    );
    scenarios.push(thickness);

    let mut clipped = base_scenario(
        "overloaded",
        "Overloaded ADC",
        "Healthy slab but the receiver gain is too high, clipping the strongest impacts and adding harmonics.",
    );
    clipped.sensor = SensorConfig::overloaded();
    clipped.expected = Condition::Healthy;
    clipped.notes.push(
        "The modal estimate may survive, but the report must show clipping warnings and reduced confidence."
            .to_string(),
    );
    scenarios.push(clipped);

    scenarios
}

pub fn find(id: &str) -> Option<Scenario> {
    catalog().into_iter().find(|scenario| scenario.id == id)
}

/// FNV-1a hash used to give each scenario a stable, independent RNG stream.
fn stable_hash(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn simulate_capture(
    scenario: &Scenario,
    seed: u64,
) -> Result<(Capture, Vec<PlateMode>, u64, usize), SimError> {
    let model = PlateModel::new(
        scenario.geometry,
        scenario.material,
        scenario.sensor_position,
        scenario.impact_position,
        scenario.max_frequency_hz,
    );
    let mut modes = model.modes.clone();
    scenario
        .defect
        .apply(&mut modes, &scenario.geometry, scenario.material);
    let contact_hz = scenario
        .taps
        .iter()
        .map(|tap| tap.contact_hz)
        .fold(2000.0f64, f64::max);
    let selected = dominant_modes(&modes, scenario.max_frequency_hz, contact_hz, 22, 0.004);
    if selected.is_empty() {
        return Err(SimError::NoModes(
            "plate model produced no modes in the analysis band".to_string(),
        ));
    }
    let mut rng = Rng::new(seed ^ stable_hash(&scenario.id));
    let samples = synthesize(
        &selected,
        &scenario.taps,
        scenario.sample_rate_hz,
        scenario.duration_s,
        &scenario.sensor,
        &mut rng,
    );
    let datagrams = transmit(
        &samples,
        scenario.sample_rate_hz,
        &scenario.network,
        &mut rng,
    );
    if datagrams.is_empty() {
        return Err(SimError::EmptyCapture);
    }
    let received = receive(&datagrams, 0)?;
    Ok((
        received.capture,
        selected,
        received.invalid_packets,
        received.datagrams,
    ))
}

/// Build the pristine baseline used to score a scenario.
pub fn healthy_reference(scenario: &Scenario, seed: u64) -> Result<ReferenceProfile, SimError> {
    let mut baseline = scenario.clone();
    baseline.id = format!("{}-baseline", scenario.id);
    baseline.defect = Defect::healthy();
    baseline.network = NetworkConfig::clean();
    baseline.sensor = SensorConfig {
        snr_db: 45.0,
        pink_fraction: 0.30,
        mains_dbfs: -70.0,
        gain: 1.0,
        ..scenario.sensor.clone()
    };
    baseline.taps = nominal_taps();
    let (capture, _, _, _) = simulate_capture(&baseline, seed)?;
    let analysis = analyze_capture(&capture, &scenario.analysis_config());
    build_reference(
        format!("{} healthy baseline", scenario.id),
        &analysis,
        Some(scenario.slab_descriptor()),
    )
    .map_err(SimError::NoModes)
}

/// Simulate, reassemble, analyse and classify one scenario.
pub fn run_scenario(scenario: &Scenario, seed: u64) -> Result<ScenarioRun, SimError> {
    let (capture, simulated_modes, invalid_packets, datagrams) = simulate_capture(scenario, seed)?;
    let analysis = analyze_capture(&capture, &scenario.analysis_config());
    let reference = healthy_reference(scenario, seed)?;
    let assessment = classify(&analysis, &reference, &ClassifierConfig::default());
    let truth = GroundTruth {
        condition: scenario.expected.label().to_string(),
        description: scenario.defect.kind.description().to_string(),
        modal_frequencies_hz: simulated_modes
            .iter()
            .take(10)
            .map(|mode| mode.frequency_hz)
            .collect(),
        damping_ratios: simulated_modes
            .iter()
            .take(10)
            .map(|mode| mode.damping_ratio)
            .collect(),
        notes: {
            let mut notes = scenario.notes.clone();
            notes.push(format!(
                "Link: {} datagrams, {} invalid, {:.1}% packet loss.",
                datagrams,
                invalid_packets,
                capture
                    .stats
                    .as_ref()
                    .map(|stats| stats.packet_loss_fraction() * 100.0)
                    .unwrap_or(0.0)
            ));
            notes
        },
    };
    Ok(ScenarioRun {
        scenario: scenario.clone(),
        seed,
        capture,
        analysis,
        reference,
        assessment,
        truth,
        invalid_packets,
        datagrams,
        simulated_modes,
    })
}

/// Run a scenario with its built-in default seed.
pub fn run(id: &str) -> Result<ScenarioRun, SimError> {
    let scenario = find(id).ok_or_else(|| SimError::NoModes(format!("unknown scenario '{id}'")))?;
    run_scenario(&scenario, 0x5eed_1234)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<String> = catalog().into_iter().map(|s| s.id).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn healthy_scenario_is_healthy() {
        let run = run_scenario(&find("healthy").unwrap(), 42).unwrap();
        assert!(
            !run.analysis.modes.is_empty(),
            "no modes: {:?}",
            run.analysis.quality
        );
        assert_eq!(
            run.assessment.condition,
            Condition::Healthy,
            "assessment: {:?}",
            run.assessment
        );
    }

    #[test]
    fn crack_scenario_is_not_healthy() {
        let run = run_scenario(&find("crack").unwrap(), 42).unwrap();
        assert_ne!(
            run.assessment.condition,
            Condition::Healthy,
            "assessment: {:?}",
            run.assessment
        );
    }
}
