//! Batch validation suite and report-bundle writer.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use granite_core::classify::Condition;
use granite_core::report::{
    render_html, render_suite_html, write_events_csv, write_features_csv, write_modes_csv,
    ReportData, SuiteEntry,
};
use granite_core::time::{format_unix_utc, now_unix_s};
use granite_core::wav::write_wav_f32;

use crate::lab::write_lab;
use crate::scenario::{catalog, run_scenario, Scenario, ScenarioRun};
use crate::visual::{write_scope, write_showcase};

/// Options for a batch run.
#[derive(Clone, Debug)]
pub struct SuiteOptions {
    pub output_dir: PathBuf,
    pub seed: u64,
    /// Empty means "all catalogue scenarios".
    pub scenario_ids: Vec<String>,
}

impl Default for SuiteOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("out/suite"),
            seed: 0x5eed_1234,
            scenario_ids: Vec::new(),
        }
    }
}

/// Paths written for one scenario.
#[derive(Clone, Debug)]
pub struct ReportBundle {
    pub directory: PathBuf,
    pub report_html: PathBuf,
    pub scope_html: PathBuf,
    pub capture_wav: PathBuf,
    pub modes_csv: PathBuf,
    pub features_csv: PathBuf,
    pub events_csv: PathBuf,
    pub reference: PathBuf,
    pub summary: PathBuf,
}

/// Result of a batch run.
#[derive(Clone, Debug)]
pub struct SuiteResult {
    pub entries: Vec<SuiteEntry>,
    pub index_path: PathBuf,
    pub showcase_path: PathBuf,
    pub lab_path: PathBuf,
    pub matched: usize,
}

pub fn is_match(expected: &str, predicted: &str) -> bool {
    expected.eq_ignore_ascii_case(predicted)
        || (expected.eq_ignore_ascii_case("SUSPECT") && !predicted.eq_ignore_ascii_case("HEALTHY"))
}

fn summary_text(run: &ScenarioRun) -> String {
    let mut out = String::new();
    out.push_str(&format!("scenario={}\n", run.scenario.id));
    out.push_str(&format!("title={}\n", run.scenario.title));
    out.push_str(&format!("seed={}\n", run.seed));
    out.push_str(&format!("expected={}\n", run.scenario.expected.label()));
    out.push_str(&format!("predicted={}\n", run.assessment.condition.label()));
    out.push_str(&format!(
        "anomaly_score={:.2}\nconfidence={:.3}\n",
        run.assessment.anomaly_score, run.assessment.confidence
    ));
    out.push_str(&format!(
        "fundamental_hz={:.3}\nmean_damping={:.6}\nmodes={}\nevents={}\n",
        run.analysis.features.fundamental_hz,
        run.analysis.features.mean_damping,
        run.analysis.modes.len(),
        run.analysis.events.len()
    ));
    out.push_str(&format!(
        "packet_loss_fraction={:.6}\ninvalid_packets={}\ndatagrams={}\n",
        run.capture
            .stats
            .as_ref()
            .map(|stats| stats.packet_loss_fraction())
            .unwrap_or(0.0),
        run.invalid_packets,
        run.datagrams
    ));
    out.push_str(&format!(
        "quality_usable={}\nquality_snr_db={:.2}\nquality_clipping={:.6}\n",
        run.analysis.quality.usable,
        run.analysis.quality.snr_db,
        run.analysis.quality.clipping_fraction
    ));
    for flag in &run.assessment.flags {
        out.push_str(&format!("flag={flag}\n"));
    }
    out
}

/// Write WAV, HTML, CSV and reference files for one run.
pub fn write_bundle(run: &ScenarioRun, directory: &Path) -> io::Result<ReportBundle> {
    fs::create_dir_all(directory)?;
    let report_html = directory.join("report.html");
    let scope_html = directory.join("scope.html");
    let capture_wav = directory.join("capture.wav");
    let modes_csv = directory.join("modes.csv");
    let features_csv = directory.join("features.csv");
    let events_csv = directory.join("events.csv");
    let reference = directory.join("reference.txt");
    let summary = directory.join("summary.txt");

    write_wav_f32(
        &capture_wav,
        run.capture.sample_rate_hz,
        &run.capture.samples,
    )?;
    write_modes_csv(&modes_csv, &run.analysis)?;
    write_features_csv(&features_csv, &run.analysis)?;
    write_events_csv(&events_csv, &run.analysis)?;
    run.reference.save(&reference)?;
    fs::write(&summary, summary_text(run))?;

    let mut notes = run.scenario.notes.clone();
    notes.push(format!(
        "Simulation seed: {}. Simulated modes: {}.",
        run.seed,
        run.simulated_modes.len()
    ));
    let data = ReportData {
        title: format!("Granite NDT — {}", run.scenario.title),
        subtitle: run.scenario.description.clone(),
        generated_at: format_unix_utc(now_unix_s()),
        capture: run.capture.clone(),
        analysis: run.analysis.clone(),
        assessment: Some(run.assessment.clone()),
        reference: Some(run.reference.clone()),
        ground_truth: Some(run.truth.clone()),
        scenario: Some(run.scenario.id.clone()),
        notes,
    };
    fs::write(&report_html, render_html(&data))?;
    write_scope(run, &scope_html)?;

    Ok(ReportBundle {
        directory: directory.to_path_buf(),
        report_html,
        scope_html,
        capture_wav,
        modes_csv,
        features_csv,
        events_csv,
        reference,
        summary,
    })
}

fn selected_scenarios(ids: &[String]) -> io::Result<Vec<Scenario>> {
    let all = catalog();
    if ids.is_empty() {
        return Ok(all);
    }
    let mut selected = Vec::new();
    for id in ids {
        let scenario = all
            .iter()
            .find(|scenario| scenario.id == *id)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown scenario '{id}'"),
                )
            })?;
        selected.push(scenario);
    }
    Ok(selected)
}

/// Run every selected scenario, write a report bundle for each and build the
/// suite index.  `on_run` is called after each bundle is written.
pub fn run_suite(
    options: &SuiteOptions,
    mut on_run: impl FnMut(&ScenarioRun, &ReportBundle),
) -> io::Result<SuiteResult> {
    let scenarios = selected_scenarios(&options.scenario_ids)?;
    if scenarios.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no scenarios selected",
        ));
    }
    fs::create_dir_all(&options.output_dir)?;
    let mut entries = Vec::new();
    let mut runs = Vec::new();
    let mut matched = 0usize;
    for scenario in &scenarios {
        let run = run_scenario(scenario, options.seed).map_err(|error| {
            io::Error::other(format!("scenario '{}' failed: {error}", scenario.id))
        })?;
        let directory = options.output_dir.join(&scenario.id);
        let bundle = write_bundle(&run, &directory)?;
        let predicted = run.assessment.condition.label().to_string();
        let expected = scenario.expected.label().to_string();
        if is_match(&expected, &predicted) {
            matched += 1;
        }
        let entry = SuiteEntry {
            title: scenario.title.clone(),
            scenario: scenario.id.clone(),
            expected,
            predicted,
            score: run.assessment.anomaly_score,
            fundamental_hz: run.analysis.features.fundamental_hz,
            damping_ratio: run.analysis.features.mean_damping,
            packet_loss: run
                .capture
                .stats
                .as_ref()
                .map(|stats| stats.packet_loss_fraction())
                .unwrap_or(0.0),
            report_path: format!("{}/report.html", scenario.id),
            scope_path: format!("{}/scope.html", scenario.id),
        };
        entries.push(entry);
        on_run(&run, &bundle);
        runs.push(run);
    }
    let index_path = options.output_dir.join("index.html");
    fs::write(
        &index_path,
        render_suite_html("Granite NDT — digital-twin validation suite", &entries),
    )?;
    let showcase_path = options.output_dir.join("showcase.html");
    write_showcase(&entries, &showcase_path)?;
    let lab_path = options.output_dir.join("lab.html");
    write_lab(&runs, &lab_path)?;
    Ok(SuiteResult {
        entries,
        index_path,
        showcase_path,
        lab_path,
        matched,
    })
}

/// A compact table for the terminal.
pub fn format_table(entries: &[SuiteEntry]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<20} {:<10} {:<10} {:>7} {:>9} {:>9}\n",
        "scenario", "expected", "predicted", "score", "f1 (Hz)", "damping"
    ));
    out.push_str(&format!("{}\n", "-".repeat(72)));
    for entry in entries {
        out.push_str(&format!(
            "{:<20} {:<10} {:<10} {:>7.1} {:>9.1} {:>9.4}\n",
            entry.scenario,
            entry.expected,
            entry.predicted,
            entry.score,
            entry.fundamental_hz,
            entry.damping_ratio
        ));
    }
    out
}

/// Convenience for the CLI: map a condition to a short status marker.
pub fn status_marker(expected: Condition, predicted: Condition) -> &'static str {
    if is_match(expected.label(), predicted.label()) {
        "PASS"
    } else {
        "FAIL"
    }
}
