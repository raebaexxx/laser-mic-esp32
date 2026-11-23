//! Self-contained HTML report generation.
//!
//! Reports embed inline SVG charts and a base64 BMP spectrogram, so they can
//! be opened offline, attached to a course submission, or printed without any
//! JavaScript or external assets.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use crate::analysis::{CaptureAnalysis, FeatureSet};
use crate::capture::Capture;
use crate::classify::{Assessment, Condition, ReferenceProfile};
use crate::dsp::fft::fft_real;
use crate::dsp::window::WindowKind;

/// Known truth for a simulated scenario, shown next to the prediction.
#[derive(Clone, Debug)]
pub struct GroundTruth {
    pub condition: String,
    pub description: String,
    pub modal_frequencies_hz: Vec<f64>,
    pub damping_ratios: Vec<f64>,
    pub notes: Vec<String>,
}

/// Everything needed to render one report.
#[derive(Clone, Debug)]
pub struct ReportData {
    pub title: String,
    pub subtitle: String,
    pub generated_at: String,
    pub capture: Capture,
    pub analysis: CaptureAnalysis,
    pub assessment: Option<Assessment>,
    pub reference: Option<ReferenceProfile>,
    pub ground_truth: Option<GroundTruth>,
    pub scenario: Option<String>,
    pub notes: Vec<String>,
}

/// One row in the multi-scenario suite index.
#[derive(Clone, Debug)]
pub struct SuiteEntry {
    pub title: String,
    pub scenario: String,
    pub expected: String,
    pub predicted: String,
    pub score: f64,
    pub fundamental_hz: f64,
    pub damping_ratio: f64,
    pub packet_loss: f64,
    pub report_path: String,
    pub scope_path: String,
}

const CSS: &str = r##"
:root {
  --ink: #17212b;
  --muted: #5d6b7a;
  --line: #dbe2ea;
  --bg: #f4f6f9;
  --card: #ffffff;
  --accent: #1f6feb;
  --healthy: #17864b;
  --suspect: #b7791f;
  --defect: #c0392b;
  --unknown: #6b7280;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--ink);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  line-height: 1.5;
}
header {
  background: linear-gradient(135deg, #10243f 0%, #1f4f7a 100%);
  color: #fff;
  padding: 34px 24px 30px;
}
header .inner, main, footer .inner { max-width: 1180px; margin: 0 auto; }
header h1 { margin: 0 0 6px; font-size: 28px; letter-spacing: -0.02em; }
header p { margin: 0; color: #c9d8e8; max-width: 900px; }
header .meta { margin-top: 14px; font-size: 13px; color: #9fb6cd; }
main { padding: 24px; }
section { margin: 0 0 22px; }
.card {
  background: var(--card);
  border: 1px solid var(--line);
  border-radius: 14px;
  padding: 20px;
  box-shadow: 0 1px 2px rgba(16, 36, 63, 0.04);
}
h2 { font-size: 18px; margin: 0 0 14px; letter-spacing: -0.01em; }
h3 { font-size: 15px; margin: 18px 0 8px; }
p { margin: 0 0 10px; }
.muted { color: var(--muted); }
.small { font-size: 13px; }
.hero { display: grid; grid-template-columns: 1.3fr 1fr; gap: 20px; }
@media (max-width: 820px) { .hero { grid-template-columns: 1fr; } }
.badge {
  display: inline-block;
  padding: 5px 11px;
  border-radius: 999px;
  color: #fff;
  font-weight: 700;
  font-size: 13px;
  letter-spacing: 0.04em;
}
.badge.healthy { background: var(--healthy); }
.badge.suspect { background: var(--suspect); }
.badge.defect { background: var(--defect); }
.badge.unknown { background: var(--unknown); }
.score-line { display: flex; align-items: baseline; gap: 10px; margin-top: 10px; }
.score-line .value { font-size: 34px; font-weight: 750; letter-spacing: -0.03em; }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 14px; }
.metric { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 14px 16px; }
.metric .label { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; }
.metric .value { font-size: 22px; font-weight: 700; margin-top: 3px; }
.metric .unit { font-size: 13px; color: var(--muted); font-weight: 500; }
table { width: 100%; border-collapse: collapse; font-size: 14px; }
th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--line); }
th { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; }
tr:last-child td { border-bottom: 0; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
.flag-list { margin: 0; padding-left: 20px; }
.flag-list li { margin: 4px 0; }
.warning { color: #8a5a00; background: #fff8e6; border: 1px solid #f0dca8; padding: 10px 12px; border-radius: 10px; margin: 8px 0; }
.ok { color: #146c43; background: #eaf7ef; border: 1px solid #bfe6cc; padding: 10px 12px; border-radius: 10px; }
.chart { width: 100%; height: auto; display: block; }
.grid { stroke: #e6ebf1; stroke-width: 1; }
.axis { stroke: #9aa8b6; stroke-width: 1; }
.axis-label { fill: #5d6b7a; font-size: 12px; }
.chart-title { fill: #17212b; font-size: 14px; font-weight: 600; }
.series { fill: none; stroke: #1f6feb; stroke-width: 1.6; }
.marker { stroke: #c0392b; stroke-width: 1; stroke-dasharray: 4 4; }
.marker-label { fill: #c0392b; font-size: 11px; }
.mode-dot { fill: #c0392b; stroke: #fff; stroke-width: 1.5; }
.spectrogram { width: 100%; border-radius: 10px; border: 1px solid var(--line); }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 18px; }
@media (max-width: 820px) { .two-col { grid-template-columns: 1fr; } }
footer { padding: 18px 24px 34px; color: var(--muted); font-size: 12px; }
code { background: #eef2f7; padding: 1px 5px; border-radius: 5px; font-size: 0.92em; }
@media print {
  body { background: #fff; }
  header { background: #fff; color: #000; padding: 10px 0; border-bottom: 2px solid #000; }
  header p, header .meta { color: #444; }
  .card, .metric { box-shadow: none; break-inside: avoid; }
  main { padding: 0; }
}
"##;

/// Render a complete report.
pub fn render_html(data: &ReportData) -> String {
    let condition = data
        .assessment
        .as_ref()
        .map(|a| a.condition)
        .unwrap_or(Condition::Unknown);
    let score = data
        .assessment
        .as_ref()
        .map(|a| a.anomaly_score)
        .unwrap_or(0.0);
    let fundamental = data.analysis.features.fundamental_hz;
    let damping = data.analysis.features.mean_damping;

    let mut out = String::with_capacity(64 * 1024);
    out.push_str("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    let _ = writeln!(out, "<title>{}</title>", escape(&data.title));
    out.push_str("<style>");
    out.push_str(CSS);
    out.push_str("</style></head><body>\n<header><div class=\"inner\">\n");
    let _ = writeln!(out, "<h1>{}</h1>", escape(&data.title));
    let _ = writeln!(out, "<p>{}</p>", escape(&data.subtitle));
    let _ = writeln!(
        out,
        "<div class=\"meta\">Generated {} &middot; source: {} &middot; {:.1} kHz &middot; {:.2} s</div>",
        escape(&data.generated_at),
        escape(&data.capture.source),
        data.capture.sample_rate_hz / 1000.0,
        data.capture.duration_s()
    );
    out.push_str("</div></header>\n<main>\n");

    // Hero: condition, score and gauge.
    out.push_str("<section class=\"hero\">\n");
    out.push_str("<div class=\"card\">\n");
    out.push_str("<div class=\"small muted\">Condition assessment</div>\n");
    let _ = writeln!(
        out,
        "<div style=\"margin-top:6px\"><span class=\"badge {}\">{}</span></div>",
        condition.css_class(),
        condition.label()
    );
    out.push_str("<div class=\"score-line\">");
    let _ = write!(out, "<span class=\"value\">{:.0}</span>", score);
    out.push_str("<span class=\"muted\">/ 100 anomaly score</span></div>\n");
    if let Some(assessment) = &data.assessment {
        let _ = writeln!(
            out,
            "<p class=\"small muted\" style=\"margin-top:8px\">Confidence {:.0}% &middot; reference: {}</p>",
            assessment.confidence * 100.0,
            escape(&assessment.reference_name)
        );
    } else {
        out.push_str("<p class=\"small muted\" style=\"margin-top:8px\">No reference profile supplied; showing descriptive analysis only.</p>\n");
    }
    if let Some(truth) = &data.ground_truth {
        let _ = writeln!(
            out,
            "<p class=\"small\" style=\"margin-top:10px\"><strong>Ground truth:</strong> {}</p>",
            escape(&truth.description)
        );
    }
    out.push_str("</div>\n<div class=\"card\">\n");
    let _ = writeln!(out, "{}", score_gauge(score, condition));
    out.push_str("</div>\n</section>\n");

    // Summary cards.
    out.push_str("<section class=\"cards\">\n");
    card(
        &mut out,
        "Fundamental mode",
        &format!("{fundamental:.1}"),
        "Hz",
    );
    card(
        &mut out,
        "Mean damping ratio",
        &format!("{:.4}", damping),
        "",
    );
    card(
        &mut out,
        "Modes / impacts",
        &format!(
            "{} / {}",
            data.analysis.modes.len(),
            data.analysis.events.len()
        ),
        "",
    );
    card(
        &mut out,
        "Trigger SNR",
        &format!("{:.1}", data.analysis.quality.snr_db),
        "dB",
    );
    if let Some(stats) = &data.capture.stats {
        card(
            &mut out,
            "Packet loss",
            &format!("{:.1}", stats.packet_loss_fraction() * 100.0),
            "%",
        );
    } else {
        card(
            &mut out,
            "Data quality",
            if data.analysis.quality.usable {
                "usable"
            } else {
                "check"
            },
            "",
        );
    }
    out.push_str("</section>\n");

    // Assessment flags and explanations.
    out.push_str("<section class=\"card\">\n<h2>Assessment</h2>\n");
    if let Some(assessment) = &data.assessment {
        out.push_str("<ul class=\"flag-list\">\n");
        for flag in &assessment.flags {
            let _ = writeln!(out, "<li>{}</li>", escape(flag));
        }
        out.push_str("</ul>\n");
        if !assessment.explanations.is_empty() {
            out.push_str("<h3>Interpretation</h3>\n");
            for explanation in &assessment.explanations {
                let _ = writeln!(out, "<p>{}</p>", escape(explanation));
            }
        }
        if !assessment.metrics.is_empty() {
            out.push_str("<h3>Scoring detail</h3>\n<table><thead><tr><th>Metric</th><th class=\"num\">Value</th><th class=\"num\">Reference</th><th class=\"num\">z-score</th><th class=\"num\">Weight</th></tr></thead><tbody>\n");
            for metric in &assessment.metrics {
                let _ = writeln!(
                    out,
                    "<tr><td>{}</td><td class=\"num\">{:.4} {}</td><td class=\"num\">{:.4}</td><td class=\"num\">{:.2}</td><td class=\"num\">{:.2}</td></tr>",
                    escape(&metric.name),
                    metric.value,
                    escape(&metric.unit),
                    metric.expected,
                    metric.z_score,
                    metric.weight
                );
            }
            out.push_str("</tbody></table>\n");
        }
    } else {
        out.push_str("<p class=\"muted\">Provide a reference profile to enable anomaly scoring and condition classification.</p>\n");
    }
    out.push_str("</section>\n");

    // Waveform and spectrum.
    out.push_str("<section class=\"card\">\n<h2>Time-domain signal</h2>\n");
    let _ = writeln!(out, "{}", waveform_svg(&data.capture, &data.analysis));
    out.push_str("</section>\n");

    out.push_str("<section class=\"card\">\n<h2>Modal spectrum</h2>\n");
    let _ = writeln!(out, "{}", spectrum_svg(&data.analysis));
    out.push_str("<h3>Spectrogram</h3>\n");
    if let Some(uri) = spectrogram_data_uri(&data.capture) {
        let _ = writeln!(
            out,
            "<img class=\"spectrogram\" alt=\"spectrogram\" src=\"{}\">",
            uri
        );
    } else {
        out.push_str("<p class=\"muted\">Capture is too short for a spectrogram.</p>\n");
    }
    out.push_str("</section>\n");

    // Modal table.
    out.push_str("<section class=\"card\">\n<h2>Modal parameters</h2>\n");
    if data.analysis.modes.is_empty() {
        out.push_str("<p class=\"muted\">No stable modes were extracted.</p>\n");
    } else {
        out.push_str("<table><thead><tr><th>#</th><th class=\"num\">Frequency</th><th class=\"num\">Std</th><th class=\"num\">Damping</th><th class=\"num\">Q</th><th class=\"num\">Amplitude</th><th class=\"num\">Persistence</th></tr></thead><tbody>\n");
        for (index, mode) in data.analysis.modes.iter().enumerate() {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td class=\"num\">{:.1} Hz</td><td class=\"num\">{:.2} Hz</td><td class=\"num\">{:.4}</td><td class=\"num\">{:.0}</td><td class=\"num\">{:.3}</td><td class=\"num\">{:.0}%</td></tr>",
                index + 1,
                mode.frequency_hz,
                mode.frequency_std_hz,
                mode.damping_ratio,
                mode.q_factor,
                mode.amplitude,
                mode.persistence * 100.0
            );
        }
        out.push_str("</tbody></table>\n");
    }
    out.push_str("</section>\n");

    // Feature comparison.
    out.push_str("<section class=\"card\">\n<h2>Feature comparison</h2>\n");
    feature_table(&mut out, &data.analysis.features, data.reference.as_ref());
    out.push_str("</section>\n");

    // Transport and quality.
    out.push_str("<section class=\"two-col\">\n<div class=\"card\">\n<h2>Transport</h2>\n");
    if let Some(stats) = &data.capture.stats {
        out.push_str("<table><tbody>\n");
        row(
            &mut out,
            "Packets received",
            &stats.packets_received.to_string(),
        );
        row(&mut out, "Packets lost", &stats.packets_lost.to_string());
        row(
            &mut out,
            "Duplicates / reordered",
            &format!(
                "{} / {}",
                stats.packets_duplicate, stats.packets_out_of_order
            ),
        );
        row(
            &mut out,
            "Gap samples / gaps",
            &format!("{} / {}", stats.gap_samples, stats.gap_count),
        );
        row(
            &mut out,
            "Max jitter",
            &format!("{:.1} ms", stats.max_jitter_ms),
        );
        row(&mut out, "Bytes", &stats.bytes_received.to_string());
        out.push_str("</tbody></table>\n");
    } else {
        out.push_str("<p class=\"muted\">Local file input; no transport statistics.</p>\n");
    }
    out.push_str("</div>\n<div class=\"card\">\n<h2>Data quality</h2>\n");
    out.push_str("<table><tbody>\n");
    row(
        &mut out,
        "Usable",
        if data.analysis.quality.usable {
            "yes"
        } else {
            "no"
        },
    );
    row(
        &mut out,
        "Gap fraction",
        &format!("{:.2}%", data.analysis.quality.gap_fraction * 100.0),
    );
    row(
        &mut out,
        "Clipping",
        &format!("{:.3}%", data.analysis.quality.clipping_fraction * 100.0),
    );
    row(
        &mut out,
        "Trigger SNR",
        &format!("{:.1} dB", data.analysis.quality.snr_db),
    );
    out.push_str("</tbody></table>\n");
    if data.analysis.quality.warnings.is_empty() {
        out.push_str("<div class=\"ok\">No data-quality warnings.</div>\n");
    } else {
        for warning in &data.analysis.quality.warnings {
            let _ = writeln!(out, "<div class=\"warning\">{}</div>", escape(warning));
        }
    }
    out.push_str("</div>\n</section>\n");

    // Ground truth comparison.
    if let Some(truth) = &data.ground_truth {
        out.push_str("<section class=\"card\">\n<h2>Digital-twin ground truth</h2>\n");
        let _ = writeln!(
            out,
            "<p><strong>Expected condition:</strong> {}</p>",
            escape(&truth.condition)
        );
        let _ = writeln!(out, "<p>{}</p>", escape(&truth.description));
        if !truth.modal_frequencies_hz.is_empty() {
            let values: Vec<String> = truth
                .modal_frequencies_hz
                .iter()
                .map(|f| format!("{f:.1} Hz"))
                .collect();
            let _ = writeln!(
                out,
                "<p><strong>Simulated modes:</strong> {}</p>",
                escape(&values.join(", "))
            );
        }
        if !truth.damping_ratios.is_empty() {
            let values: Vec<String> = truth
                .damping_ratios
                .iter()
                .map(|d| format!("{d:.4}"))
                .collect();
            let _ = writeln!(
                out,
                "<p><strong>Simulated damping:</strong> {}</p>",
                escape(&values.join(", "))
            );
        }
        for note in &truth.notes {
            let _ = writeln!(out, "<p class=\"small muted\">{}</p>", escape(note));
        }
        out.push_str("</section>\n");
    }

    // Notes / method.
    out.push_str("<section class=\"card\">\n<h2>Method and notes</h2>\n");
    out.push_str("<p class=\"small muted\">Raw laser-vibrometer samples are streamed from the ESP32 as framed UDP packets. The host removes DC and mains hum, detects impacts from a short-time RMS envelope, computes a Hann-windowed amplitude spectrum for each impact, picks modal peaks, estimates damping from the band-passed analytic-signal envelope, and tracks modes across repeated taps. The anomaly score is a weighted z-score distance from a measured healthy reference.</p>\n");
    for note in &data.notes {
        let _ = writeln!(out, "<p class=\"small\">{}</p>", escape(note));
    }
    out.push_str("</section>\n</main>\n<footer><div class=\"inner\">Generated by <code>granite-host</code> / <code>granite-twin</code>. Research and educational instrument; not a certified structural-safety assessment.</div></footer>\n</body></html>\n");
    out
}

fn card(out: &mut String, label: &str, value: &str, unit: &str) {
    let _ = writeln!(
        out,
        "<div class=\"metric\"><div class=\"label\">{}</div><div class=\"value\">{} <span class=\"unit\">{}</span></div></div>",
        escape(label),
        escape(value),
        escape(unit)
    );
}

fn row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(
        out,
        "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
        escape(label),
        escape(value)
    );
}

fn feature_table(out: &mut String, features: &FeatureSet, reference: Option<&ReferenceProfile>) {
    out.push_str("<table><thead><tr><th>Feature</th><th class=\"num\">Value</th><th class=\"num\">Reference</th><th class=\"num\">z-score</th></tr></thead><tbody>\n");
    for (key, value) in features.named_values() {
        let reference_feature = reference.and_then(|r| r.feature(key));
        let (expected, z) = match reference_feature {
            Some(reference) => (
                format!("{:.4}", reference.mean),
                format!("{:.2}", (value - reference.mean) / reference.std.max(1e-9)),
            ),
            None => ("&mdash;".to_string(), "&mdash;".to_string()),
        };
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td class=\"num\">{:.4}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            escape(key),
            value,
            expected,
            z
        );
    }
    out.push_str("</tbody></table>\n");
}

/// Render the multi-scenario suite index.
pub fn render_suite_html(title: &str, entries: &[SuiteEntry]) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    let _ = writeln!(
        out,
        "<title>{}</title><style>{}</style></head><body>",
        escape(title),
        CSS
    );
    let _ = writeln!(
        out,
        "<header><div class=\"inner\"><h1>{}</h1><p>Digital-twin validation suite. Each row links to a full analysis report and an animated scope. <a href=\"showcase.html\"><strong>Open the interactive showcase →</strong></a> · <a href=\"lab.html\"><strong>Open the 3D rig →</strong></a></p></div></header><main>",
        escape(title)
    );
    let correct = entries
        .iter()
        .filter(|entry| {
            entry.expected.eq_ignore_ascii_case(&entry.predicted)
                || (entry.expected == "SUSPECT" && entry.predicted != "HEALTHY")
        })
        .count();
    let _ = writeln!(
        out,
        "<section class=\"cards\"><div class=\"metric\"><div class=\"label\">Scenarios</div><div class=\"value\">{}</div></div><div class=\"metric\"><div class=\"label\">Expected label matched</div><div class=\"value\">{} / {}</div></div></section>",
        entries.len(),
        correct,
        entries.len()
    );
    out.push_str("<section class=\"card\"><h2>Results</h2><table><thead><tr><th>Scenario</th><th>Expected</th><th>Predicted</th><th class=\"num\">Score</th><th class=\"num\">f1 (Hz)</th><th class=\"num\">Damping</th><th class=\"num\">Packet loss</th><th>Views</th></tr></thead><tbody>\n");
    for entry in entries {
        let _ = writeln!(
            out,
            "<tr><td><strong>{}</strong><br><span class=\"small muted\">{}</span></td><td>{}</td><td><span class=\"badge {}\">{}</span></td><td class=\"num\">{:.0}</td><td class=\"num\">{:.1}</td><td class=\"num\">{:.4}</td><td class=\"num\">{:.1}%</td><td><a href=\"{}\">report</a><br><a href=\"{}\">scope</a></td></tr>",
            escape(&entry.title),
            escape(&entry.scenario),
            escape(&entry.expected),
            condition_class(&entry.predicted),
            escape(&entry.predicted),
            entry.score,
            entry.fundamental_hz,
            entry.damping_ratio,
            entry.packet_loss * 100.0,
            escape(&entry.report_path),
            escape(&entry.scope_path)
        );
    }
    out.push_str("</tbody></table></section></main><footer><div class=\"inner\">Generated by <code>granite-twin suite</code>.</div></footer></body></html>\n");
    out
}

fn condition_class(label: &str) -> &'static str {
    match label {
        "HEALTHY" => "healthy",
        "SUSPECT" => "suspect",
        "DEFECT" => "defect",
        _ => "unknown",
    }
}

fn waveform_svg(capture: &Capture, analysis: &CaptureAnalysis) -> String {
    let width = 1000.0;
    let height = 280.0;
    let left = 62.0;
    let right = 18.0;
    let top = 24.0;
    let bottom = 38.0;
    let plot_w = width - left - right;
    let plot_h = height - top - bottom;
    let n = capture.samples.len();
    let duration = capture.duration_s().max(1e-6);
    let peak = capture.peak().max(1e-9) * 1.08;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg class=\"chart\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-label=\"waveform\">"
    );
    let _ = writeln!(
        out,
        "<text class=\"chart-title\" x=\"{left:.0}\" y=\"16\">Reconstructed signal (peak ±{peak:.3})</text>"
    );
    for tick in 0..=4 {
        let frac = tick as f64 / 4.0;
        let y = top + plot_h * (1.0 - frac);
        let value = -peak + 2.0 * peak * frac;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{left:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            width - right
        );
        let _ = writeln!(
            out,
            "<text class=\"axis-label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{value:.2}</text>",
            left - 6.0,
            y + 4.0
        );
    }
    for tick in 0..=5 {
        let frac = tick as f64 / 5.0;
        let x = left + plot_w * frac;
        let time = duration * frac;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{top:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            top + plot_h
        );
        let _ = writeln!(
            out,
            "<text class=\"axis-label\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{time:.2} s</text>",
            height - 14.0
        );
    }
    // Gap shading.
    for gap in &capture.gaps {
        let start = (gap.start_sample.saturating_sub(capture.start_sample_index)) as f64
            / capture.sample_rate_hz;
        let end = start + gap.len as f64 / capture.sample_rate_hz;
        let x1 = left + plot_w * (start / duration).clamp(0.0, 1.0);
        let x2 = left + plot_w * (end / duration).clamp(0.0, 1.0);
        let _ = writeln!(
            out,
            "<rect x=\"{x1:.1}\" y=\"{top:.1}\" width=\"{:.1}\" height=\"{plot_h:.1}\" fill=\"#f1d7d7\" opacity=\"0.55\"/>",
            (x2 - x1).max(1.0)
        );
    }
    // Min/max column rendering.
    let columns = plot_w as usize;
    if n > 0 {
        for column in 0..columns {
            let start = column * n / columns;
            let end = ((column + 1) * n / columns).max(start + 1).min(n);
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for value in &capture.samples[start..end] {
                lo = lo.min(*value);
                hi = hi.max(*value);
            }
            let x = left + column as f64 + 0.5;
            let y_hi = top + plot_h * (0.5 - hi as f64 / (2.0 * peak));
            let y_lo = top + plot_h * (0.5 - lo as f64 / (2.0 * peak));
            let _ = writeln!(
                out,
                "<line x1=\"{x:.1}\" y1=\"{y_hi:.1}\" x2=\"{x:.1}\" y2=\"{y_lo:.1}\" stroke=\"#1f6feb\" stroke-width=\"1\" opacity=\"0.8\"/>"
            );
        }
    }
    // Impact markers.
    for event in &analysis.events {
        let x = left + plot_w * (event.peak_time_s / duration).clamp(0.0, 1.0);
        let _ = writeln!(
            out,
            "<line class=\"marker\" x1=\"{x:.1}\" y1=\"{top:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            top + plot_h
        );
        let _ = writeln!(
            out,
            "<text class=\"marker-label\" x=\"{:.1}\" y=\"{:.1}\">E{}</text>",
            x + 3.0,
            top + 12.0,
            event.index + 1
        );
    }
    let _ = writeln!(
        out,
        "<line class=\"axis\" x1=\"{left:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        top + plot_h,
        width - right,
        top + plot_h
    );
    let _ = writeln!(
        out,
        "<line class=\"axis\" x1=\"{left:.1}\" y1=\"{top:.1}\" x2=\"{left:.1}\" y2=\"{:.1}\"/>",
        top + plot_h
    );
    out.push_str("</svg>");
    out
}

fn spectrum_svg(analysis: &CaptureAnalysis) -> String {
    let spectrum = &analysis.averaged_spectrum;
    if spectrum.is_empty() {
        return "<p class=\"muted\">No spectrum available.</p>".to_string();
    }
    let width = 1000.0;
    let height = 340.0;
    let left = 62.0;
    let right = 18.0;
    let top = 24.0;
    let bottom = 38.0;
    let plot_w = width - left - right;
    let plot_h = height - top - bottom;
    let fundamental = analysis.features.fundamental_hz;
    let max_freq = if fundamental > 0.0 {
        (fundamental * 4.0).max(4000.0)
    } else {
        8000.0
    }
    .min(analysis.sample_rate_hz * 0.45)
    .min(12_000.0);
    let max_db = spectrum
        .magnitude_db
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let min_db = max_db - 70.0;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg class=\"chart\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-label=\"spectrum\">"
    );
    let _ = writeln!(
        out,
        "<text class=\"chart-title\" x=\"{left:.0}\" y=\"16\">Averaged amplitude spectrum (0–{max_freq:.0} Hz)</text>"
    );
    for tick in 0..=5 {
        let frac = tick as f64 / 5.0;
        let x = left + plot_w * frac;
        let f = max_freq * frac;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{top:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            top + plot_h
        );
        let _ = writeln!(
            out,
            "<text class=\"axis-label\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{f:.0}</text>",
            height - 14.0
        );
    }
    for tick in 0..=4 {
        let frac = tick as f64 / 4.0;
        let y = top + plot_h * frac;
        let db = max_db - 70.0 * frac;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{left:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            width - right
        );
        let _ = writeln!(
            out,
            "<text class=\"axis-label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{db:.0} dB</text>",
            left - 6.0,
            y + 4.0
        );
    }
    let columns = plot_w as usize;
    let mut path = String::new();
    for column in 0..columns {
        let f = max_freq * (column as f64 + 0.5) / columns as f64;
        let db = nearest_db(spectrum, f).clamp(min_db, max_db + 1.0);
        let x = left + column as f64 + 0.5;
        let y = top + plot_h * ((max_db + 1.0 - db) / (max_db + 1.0 - min_db));
        if column == 0 {
            let _ = write!(path, "M {x:.2} {y:.2}");
        } else {
            let _ = write!(path, " L {x:.2} {y:.2}");
        }
    }
    let _ = writeln!(out, "<path class=\"series\" d=\"{path}\"/>");
    for (index, mode) in analysis.modes.iter().enumerate() {
        if mode.frequency_hz > max_freq {
            continue;
        }
        let db = nearest_db(spectrum, mode.frequency_hz).clamp(min_db, max_db);
        let x = left + plot_w * (mode.frequency_hz / max_freq);
        let y = top + plot_h * ((max_db + 1.0 - db) / (max_db + 1.0 - min_db));
        let _ = writeln!(
            out,
            "<circle class=\"mode-dot\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\"/>"
        );
        let _ = writeln!(
            out,
            "<text class=\"axis-label\" x=\"{:.1}\" y=\"{:.1}\">M{}</text>",
            x + 6.0,
            (y - 7.0).max(top + 10.0),
            index + 1
        );
    }
    let _ = writeln!(
        out,
        "<line class=\"axis\" x1=\"{left:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        top + plot_h,
        width - right,
        top + plot_h
    );
    let _ = writeln!(
        out,
        "<line class=\"axis\" x1=\"{left:.1}\" y1=\"{top:.1}\" x2=\"{left:.1}\" y2=\"{:.1}\"/>",
        top + plot_h
    );
    out.push_str("</svg>");
    out
}

fn nearest_db(spectrum: &crate::dsp::Spectrum, frequency_hz: f64) -> f64 {
    if spectrum.freqs_hz.is_empty() {
        return -200.0;
    }
    let index = spectrum
        .freqs_hz
        .partition_point(|f| *f < frequency_hz)
        .min(spectrum.freqs_hz.len() - 1);
    spectrum.magnitude_db[index]
}

fn score_gauge(score: f64, condition: Condition) -> String {
    let score = score.clamp(0.0, 100.0);
    let cx = 110.0;
    let cy = 108.0;
    let r = 82.0;
    let angle = std::f64::consts::PI * (1.0 - score / 100.0);
    let end_x = cx + r * angle.cos();
    let end_y = cy - r * angle.sin();
    let color = match condition {
        Condition::Healthy => "#17864b",
        Condition::Suspect => "#b7791f",
        Condition::Defect => "#c0392b",
        Condition::Unknown => "#6b7280",
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg viewBox=\"0 0 220 150\" class=\"chart\" role=\"img\" aria-label=\"anomaly score gauge\">"
    );
    let _ = writeln!(
        out,
        "<path d=\"M {:.1} {:.1} A {r:.1} {r:.1} 0 0 1 {:.1} {:.1}\" fill=\"none\" stroke=\"#e6ebf1\" stroke-width=\"16\" stroke-linecap=\"round\"/>",
        cx - r,
        cy,
        cx + r,
        cy
    );
    if score > 0.5 {
        let _ = writeln!(
            out,
            "<path d=\"M {:.1} {:.1} A {r:.1} {r:.1} 0 0 1 {end_x:.1} {end_y:.1}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"16\" stroke-linecap=\"round\"/>",
            cx - r,
            cy
        );
    }
    let _ = writeln!(
        out,
        "<text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"32\" font-weight=\"700\" fill=\"#17212b\">{score:.0}</text>",
        cy - 8.0
    );
    let _ = writeln!(
        out,
        "<text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"12\" fill=\"#5d6b7a\">anomaly score</text>",
        cy + 12.0
    );
    out.push_str("</svg>");
    out
}

fn spectrogram_data_uri(capture: &Capture) -> Option<String> {
    let n = capture.samples.len();
    if n < 256 || capture.sample_rate_hz <= 0.0 {
        return None;
    }
    let window = 2048.min(n.next_power_of_two() / 2).max(256);
    let hop = (window / 4).max(1);
    let frames = if n <= window {
        1
    } else {
        (n - window) / hop + 1
    };
    let max_freq = (capture.sample_rate_hz * 0.45).min(8000.0);
    let bins = ((max_freq / capture.sample_rate_hz) * window as f64).ceil() as usize + 1;
    let bins = bins.min(window / 2 + 1);
    let w = WindowKind::Hann.values(window);
    let mut values = vec![vec![-200.0f64; bins]; frames];
    let mut global_max = f64::NEG_INFINITY;
    for (frame_index, frame_values) in values.iter_mut().enumerate() {
        let start = frame_index * hop;
        let mut buf = vec![0.0f64; window];
        for i in 0..window {
            let sample = capture.samples.get(start + i).copied().unwrap_or(0.0) as f64;
            buf[i] = sample * w[i];
        }
        let fft = fft_real(&buf, window);
        for (bin, slot) in frame_values.iter_mut().enumerate().take(bins) {
            let db = 20.0 * fft[bin].magnitude().max(1e-15).log10();
            *slot = db;
            global_max = global_max.max(db);
        }
    }
    if !global_max.is_finite() {
        return None;
    }
    let width = 640usize;
    let height = 240usize;
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let freq_frac = 1.0 - (y as f64 + 0.5) / height as f64;
        let bin =
            ((freq_frac * max_freq / capture.sample_rate_hz) * window as f64).round() as usize;
        for x in 0..width {
            let frame = (x * frames / width).min(frames - 1);
            let db = values[frame][bin.min(bins - 1)];
            let t = ((db - (global_max - 60.0)) / 60.0).clamp(0.0, 1.0);
            let (r, g, b) = colormap(t);
            let offset = (y * width + x) * 3;
            rgb[offset] = r;
            rgb[offset + 1] = g;
            rgb[offset + 2] = b;
        }
    }
    let bmp = encode_bmp(width, height, &rgb);
    Some(format!("data:image/bmp;base64,{}", base64(&bmp)))
}

fn colormap(t: f64) -> (u8, u8, u8) {
    let stops = [
        (0.0, (8.0, 12.0, 40.0)),
        (0.25, (40.0, 40.0, 120.0)),
        (0.5, (150.0, 40.0, 120.0)),
        (0.75, (240.0, 120.0, 40.0)),
        (1.0, (255.0, 240.0, 180.0)),
    ];
    for pair in stops.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            return (
                (c0.0 + (c1.0 - c0.0) * f) as u8,
                (c0.1 + (c1.1 - c0.1) * f) as u8,
                (c0.2 + (c1.2 - c0.2) * f) as u8,
            );
        }
    }
    (255, 240, 180)
}

fn encode_bmp(width: usize, height: usize, rgb_top_down: &[u8]) -> Vec<u8> {
    let row_size = (width * 3 + 3) & !3;
    let image_size = row_size * height;
    let file_size = 54 + image_size;
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(image_size as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let padding = row_size - width * 3;
    for y in (0..height).rev() {
        for x in 0..width {
            let offset = (y * width + x) * 3;
            out.push(rgb_top_down[offset + 2]);
            out.push(rgb_top_down[offset + 1]);
            out.push(rgb_top_down[offset]);
        }
        out.resize(out.len() + padding, 0);
    }
    out
}

/// Base64-encode bytes.  Public so the animated scope can embed the capture
/// without pulling in an external crate.
pub fn base64_encode(data: &[u8]) -> String {
    base64(data)
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Write modal parameters as CSV.
pub fn write_modes_csv(path: &Path, analysis: &CaptureAnalysis) -> io::Result<()> {
    let mut out = String::new();
    out.push_str("index,frequency_hz,frequency_std_hz,damping_ratio,damping_std,amplitude,persistence,q_factor\n");
    for (index, mode) in analysis.modes.iter().enumerate() {
        let _ = writeln!(
            out,
            "{},{:.6},{:.6},{:.8},{:.8},{:.6},{:.6},{:.3}",
            index + 1,
            mode.frequency_hz,
            mode.frequency_std_hz,
            mode.damping_ratio,
            mode.damping_std,
            mode.amplitude,
            mode.persistence,
            mode.q_factor
        );
    }
    fs::write(path, out)
}

/// Write all scalar features as CSV.
pub fn write_features_csv(path: &Path, analysis: &CaptureAnalysis) -> io::Result<()> {
    let mut out = String::new();
    out.push_str("feature,value\n");
    for (key, value) in analysis.features.named_values() {
        let _ = writeln!(out, "{key},{value:.8}");
    }
    fs::write(path, out)
}

/// Write per-impact mode estimates as CSV.
pub fn write_events_csv(path: &Path, analysis: &CaptureAnalysis) -> io::Result<()> {
    let mut out = String::new();
    out.push_str("event,peak_time_s,frequency_hz,damping_ratio,amplitude,fit_r2,method\n");
    for event in &analysis.events {
        for mode in &event.modes {
            let _ = writeln!(
                out,
                "{},{:.6},{:.6},{:.8},{:.6},{:.4},{}",
                event.index + 1,
                event.peak_time_s,
                mode.frequency_hz,
                mode.damping_ratio,
                mode.amplitude,
                mode.fit_r2,
                mode.method.label()
            );
        }
    }
    fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze_capture;
    use crate::classify::{build_reference, classify, ClassifierConfig};
    use crate::time::format_unix_utc;

    fn test_data() -> ReportData {
        let fs = 48_000.0;
        let n = 48_000;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / fs;
                if t < 0.2 {
                    0.0
                } else {
                    let t = t - 0.2;
                    ((-0.004 * 2.0 * std::f64::consts::PI * 350.0 * t).exp()
                        * (2.0 * std::f64::consts::PI * 350.0 * t).cos()) as f32
                }
            })
            .collect();
        let capture = Capture::new(fs, samples, "test");
        let analysis = analyze_capture(&capture, &Default::default());
        let reference = build_reference("test reference", &analysis, None).ok();
        let assessment = reference
            .as_ref()
            .map(|reference| classify(&analysis, reference, &ClassifierConfig::default()));
        ReportData {
            title: "Granite NDT report".to_string(),
            subtitle: "Unit-test report".to_string(),
            generated_at: format_unix_utc(0),
            capture,
            analysis,
            assessment,
            reference,
            ground_truth: None,
            scenario: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn html_contains_core_sections() {
        let data = test_data();
        let html = render_html(&data);
        assert!(html.contains("Modal parameters"));
        assert!(html.contains("Time-domain signal"));
        assert!(html.contains("data:image/bmp;base64,"));
        assert!(html.contains("Assessment"));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn base64_known_vector() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn csv_export_works() {
        let data = test_data();
        let path = std::env::temp_dir().join(format!(
            "granite-features-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_features_csv(&path, &data.analysis).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert!(text.contains("fundamental_hz"));
    }
}
