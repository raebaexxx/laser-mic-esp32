//! Animated, self-contained "instrument panel" for a digital-twin run.
//!
//! The generated HTML embeds the reconstructed capture, a moving-RMS envelope,
//! the averaged modal spectrum, the detected modes and the transport
//! statistics.  JavaScript animates a scrolling waveform and a packet stream,
//! and the capture is also playable as synthesized audio.  No server, external
//! library or network access is required.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use granite_core::analysis::CaptureAnalysis;
use granite_core::dsp::envelope::moving_rms;
use granite_core::report::{base64_encode, SuiteEntry};
use granite_core::stats;
use granite_core::time::{format_unix_utc, now_unix_s};

use crate::scenario::ScenarioRun;

/// Render the animated scope as one self-contained HTML document.
pub fn render_scope_html(run: &ScenarioRun) -> String {
    let capture = &run.capture;
    let analysis = &run.analysis;
    let fs = capture.sample_rate_hz.max(1.0);
    let duration = capture.duration_s().max(1e-6);

    let samples: Vec<i16> = capture
        .samples
        .iter()
        .map(|value| (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16)
        .collect();
    let mut sample_bytes = Vec::with_capacity(samples.len() * 2);
    for sample in &samples {
        sample_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let samples_b64 = base64_encode(&sample_bytes);

    let envelope = if capture.samples.is_empty() {
        Vec::new()
    } else {
        moving_rms(
            &capture.samples_f64(),
            ((fs * 0.002).round() as usize).max(3),
        )
    };
    let threshold = if envelope.is_empty() {
        1e-12
    } else {
        let median = stats::median(&envelope);
        let sigma = 1.4826 * stats::mad(&envelope);
        (median + 7.0 * sigma).max(median * 3.0).max(1e-12)
    };
    let envelope_points = downsample_max(&envelope, 2_000);
    let (spec_freqs, spec_db) = spectrum_points(analysis, (fs * 0.45).min(8_000.0), 720);

    let stats = capture.stats.as_ref();
    let packet_loss = stats
        .map(|stats| stats.packet_loss_fraction())
        .unwrap_or(0.0);

    let mut data = String::with_capacity(samples_b64.len() + 32 * 1024);
    data.push('{');
    let _ = write!(data, "\"title\":{},", json_string(&run.scenario.title));
    let _ = write!(
        data,
        "\"description\":{},",
        json_string(&run.scenario.description)
    );
    let _ = write!(
        data,
        "\"expected\":{},",
        json_string(run.scenario.expected.label())
    );
    let _ = write!(
        data,
        "\"predicted\":{},",
        json_string(run.assessment.condition.label())
    );
    let _ = write!(
        data,
        "\"condition_class\":{},",
        json_string(run.assessment.condition.css_class())
    );
    let _ = write!(
        data,
        "\"score\":{},",
        json_number(run.assessment.anomaly_score)
    );
    let _ = write!(
        data,
        "\"confidence\":{},",
        json_number(run.assessment.confidence)
    );
    let _ = write!(data, "\"fs\":{},", json_number(fs));
    let _ = write!(data, "\"duration\":{},", json_number(duration));
    let _ = write!(
        data,
        "\"generated\":{},",
        json_string(&format_unix_utc(now_unix_s()))
    );
    let _ = write!(data, "\"seed\":{},", run.seed);
    let _ = write!(data, "\"samples_b64\":{},", json_string(&samples_b64));
    let _ = write!(data, "\"sample_count\":{},", samples.len());
    let _ = write!(data, "\"threshold\":{},", json_number(threshold));
    data.push_str("\"envelope\":");
    push_f64_array(&mut data, &envelope_points);
    data.push(',');

    data.push_str("\"events\":[");
    for (event_index, event) in analysis.events.iter().enumerate() {
        if event_index > 0 {
            data.push(',');
        }
        let _ = write!(
            data,
            "{{\"index\":{},\"time\":{},\"snr\":{},\"modes\":[",
            event.index,
            json_number(event.peak_time_s),
            json_number(event.snr_db)
        );
        for (mode_index, mode) in event.modes.iter().enumerate() {
            if mode_index > 0 {
                data.push(',');
            }
            let _ = write!(
                data,
                "{{\"f\":{},\"damping\":{},\"amplitude\":{}}}",
                json_number(mode.frequency_hz),
                json_number(mode.damping_ratio),
                json_number(mode.amplitude)
            );
        }
        data.push_str("]}");
    }
    data.push_str("],");

    data.push_str("\"modes\":[");
    for (index, mode) in analysis.modes.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        let _ = write!(
            data,
            "{{\"f\":{},\"damping\":{},\"amplitude\":{},\"persistence\":{},\"q\":{}}}",
            json_number(mode.frequency_hz),
            json_number(mode.damping_ratio),
            json_number(mode.amplitude),
            json_number(mode.persistence),
            json_number(mode.q_factor)
        );
    }
    data.push_str("],");

    data.push_str("\"spectrum\":{\"freqs\":");
    push_f64_array(&mut data, &spec_freqs);
    data.push_str(",\"db\":");
    push_f64_array(&mut data, &spec_db);
    data.push_str("},");

    data.push_str("\"gaps\":[");
    for (index, gap) in capture.gaps.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        let start = gap.start_sample.saturating_sub(capture.start_sample_index) as f64 / fs;
        let len = gap.len as f64 / fs;
        let _ = write!(
            data,
            "{{\"start\":{},\"len\":{}}}",
            json_number(start),
            json_number(len)
        );
    }
    data.push_str("],");

    let (received, lost, duplicate, out_of_order, jitter, bytes, gap_count, gap_samples) =
        if let Some(stats) = stats {
            (
                stats.packets_received,
                stats.packets_lost,
                stats.packets_duplicate,
                stats.packets_out_of_order,
                stats.max_jitter_ms,
                stats.bytes_received,
                stats.gap_count,
                stats.gap_samples,
            )
        } else {
            (0, 0, 0, 0, 0.0, 0, 0, 0)
        };
    let _ = write!(
        data,
        "\"transport\":{{\"received\":{},\"lost\":{},\"duplicate\":{},\"out_of_order\":{},\"jitter_ms\":{},\"bytes\":{},\"gap_count\":{},\"gap_samples\":{},\"packet_loss\":{}}},",
        received,
        lost,
        duplicate,
        out_of_order,
        json_number(jitter),
        bytes,
        gap_count,
        gap_samples,
        json_number(packet_loss)
    );
    let _ = write!(
        data,
        "\"quality\":{{\"usable\":{},\"snr_db\":{},\"clipping\":{},\"gap_fraction\":{}}},",
        analysis.quality.usable,
        json_number(analysis.quality.snr_db),
        json_number(analysis.quality.clipping_fraction),
        json_number(analysis.quality.gap_fraction)
    );

    data.push_str("\"flags\":[");
    for (index, flag) in run.assessment.flags.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        data.push_str(&json_string(flag));
    }
    data.push_str("],");

    data.push_str("\"reference_modes\":[");
    for (index, mode) in run.reference.modes.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        let _ = write!(
            data,
            "{{\"f\":{},\"damping\":{}}}",
            json_number(mode.frequency_hz),
            json_number(mode.damping_ratio)
        );
    }
    data.push_str("],");

    let _ = write!(
        data,
        "\"truth\":{{\"condition\":{},\"description\":{},\"modal_frequencies\":",
        json_string(&run.truth.condition),
        json_string(&run.truth.description)
    );
    push_f64_array(&mut data, &run.truth.modal_frequencies_hz);
    data.push_str(",\"damping_ratios\":");
    push_f64_array(&mut data, &run.truth.damping_ratios);
    data.push_str(",\"notes\":[");
    for (index, note) in run.truth.notes.iter().enumerate() {
        if index > 0 {
            data.push(',');
        }
        data.push_str(&json_string(note));
    }
    data.push_str("]}");
    data.push('}');

    let mut html = String::with_capacity(data.len() + 32 * 1024);
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Granite NDT scope</title><style>",
    );
    html.push_str(SCOPE_CSS);
    html.push_str("</style></head><body><header><div class=\"wrap\">");
    let _ = write!(
        html,
        "<div class=\"eyebrow\"><span class=\"live\">● LIVE</span> DIGITAL TWIN · IMPACT-MODAL NDT · {} Hz</div>",
        fs as u32
    );
    let _ = write!(html, "<h1>{}</h1>", html_escape(&run.scenario.title));
    let _ = write!(
        html,
        "<p class=\"lede\">{}</p>",
        html_escape(&run.scenario.description)
    );
    let _ = write!(
        html,
        "<div class=\"meta\">Generated {} · seed {} · {} samples · {:.2} s</div>",
        html_escape(&format_unix_utc(now_unix_s())),
        run.seed,
        samples.len(),
        duration
    );
    html.push_str("</div></header><main class=\"wrap\">");

    html.push_str("<section class=\"card hero\"><div class=\"hero-left\">");
    let _ = write!(
        html,
        "<div class=\"badges\"><span class=\"badge {}\">PREDICTED · {}</span><span class=\"badge ghost\">EXPECTED · {}</span></div>",
        run.assessment.condition.css_class(),
        html_escape(run.assessment.condition.label()),
        html_escape(run.scenario.expected.label())
    );
    let _ = write!(
        html,
        "<div class=\"scoreline\"><span class=\"score\">{:.0}</span><span class=\"score-unit\">/ 100 anomaly score</span></div>",
        run.assessment.anomaly_score
    );
    let _ = write!(
        html,
        "<div class=\"muted\">confidence {:.0}% · reference “{}”</div>",
        run.assessment.confidence * 100.0,
        html_escape(&run.assessment.reference_name)
    );
    html.push_str("</div><div class=\"hero-right\">");
    let _ = write!(
        html,
        "<div class=\"kicker\">Fundamental mode</div><div class=\"big\">{:.1}<span> Hz</span></div>",
        analysis.features.fundamental_hz
    );
    let _ = write!(
        html,
        "<div class=\"muted\">mean damping {:.5} · T60 {:.2} s · {} modes / {} impacts</div>",
        analysis.features.mean_damping,
        analysis.features.decay_t60_s,
        analysis.modes.len(),
        analysis.events.len()
    );
    html.push_str("</div></section>");

    html.push_str("<div class=\"grid\">");

    // Live waveform.
    html.push_str("<section class=\"card wide\"><div class=\"card-head\"><h2>Live waveform</h2>");
    html.push_str("<div class=\"controls\">");
    html.push_str("<button id=\"play\" class=\"primary\" type=\"button\">⏸ Pause</button>");
    html.push_str(
        "<button id=\"next\" type=\"button\" title=\"jump to the next impact\">⏭ Next tap</button>",
    );
    html.push_str("<button id=\"restart\" type=\"button\">↺ Restart</button>");
    html.push_str("<select id=\"speed\" aria-label=\"playback speed\"><option value=\"0.25\">0.25×</option><option value=\"0.5\" selected>0.5×</option><option value=\"1\">1×</option><option value=\"2\">2×</option></select>");
    html.push_str("<button id=\"mute\" type=\"button\" title=\"enable sound\">🔇</button>");
    html.push_str("<span id=\"trigger\" class=\"led\" title=\"impact trigger\">TRIG</span>");
    html.push_str("<span id=\"time\" class=\"mono\">0.00 / 0.00 s</span>");
    html.push_str("<span id=\"readout\" class=\"readout\">click trace to seek</span>");
    html.push_str("</div></div>");
    html.push_str("<canvas id=\"wave\" width=\"1200\" height=\"320\" aria-label=\"animated waveform\"></canvas>");
    html.push_str(
        "<canvas id=\"env\" width=\"1200\" height=\"88\" aria-label=\"RMS envelope\"></canvas>",
    );
    html.push_str(
        "<input id=\"seek\" type=\"range\" min=\"0\" max=\"1000\" value=\"0\" aria-label=\"seek\">",
    );
    html.push_str("<div class=\"legend\"><span class=\"dot cyan\"></span>reconstructed signal <span class=\"dot orange\"></span>impact <span class=\"dot red\"></span>packet gap <span class=\"dot white\"></span>playhead</div>");
    html.push_str("<div class=\"hintline\">Auto-playing muted. Click 🔊 for sound, click the trace to seek, or use <kbd>space</kbd> / <kbd>←</kbd> <kbd>→</kbd>. Click a mode row to highlight it.</div>");
    html.push_str("</section>");

    // Measurement card.
    html.push_str("<aside class=\"card side\"><h2>Measurement</h2><div class=\"metrics\">");
    metric(
        &mut html,
        "Fundamental",
        &format!("{:.1}", analysis.features.fundamental_hz),
        "Hz",
    );
    metric(
        &mut html,
        "Mean damping",
        &format!("{:.5}", analysis.features.mean_damping),
        "",
    );
    metric(
        &mut html,
        "T60",
        &format!("{:.2}", analysis.features.decay_t60_s),
        "s",
    );
    metric(
        &mut html,
        "Modes / impacts",
        &format!("{} / {}", analysis.modes.len(), analysis.events.len()),
        "",
    );
    metric(
        &mut html,
        "Trigger SNR",
        &format!("{:.1}", analysis.quality.snr_db),
        "dB",
    );
    metric(
        &mut html,
        "Packet loss",
        &format!("{:.1}", packet_loss * 100.0),
        "%",
    );
    html.push_str("</div><div class=\"quality\">");
    let _ = write!(
        html,
        "<div>Quality: <strong>{}</strong></div><div class=\"small muted\">clipping {:.3}% · gaps {:.2}% · usable {}</div>",
        if analysis.quality.usable { "usable" } else { "check warnings" },
        analysis.quality.clipping_fraction * 100.0,
        analysis.quality.gap_fraction * 100.0,
        analysis.quality.usable
    );
    html.push_str("</div></aside>");

    // Spectrum.
    html.push_str("<section class=\"card wide\"><div class=\"card-head\"><h2>Modal spectrum</h2><span class=\"small muted\">averaged over impacts · dots = detected modes · triangles = reference</span></div>");
    html.push_str("<canvas id=\"spectrum\" width=\"1200\" height=\"340\" aria-label=\"modal spectrum\"></canvas></section>");

    // Transport.
    html.push_str("<aside class=\"card side\"><h2>UDP transport</h2><div id=\"packets\" class=\"packets\" aria-label=\"simulated packet stream\"></div><div class=\"small muted\">64 blocks · red = lost · orange outline = current</div><div class=\"metrics tight\">");
    metric(&mut html, "Received", &received.to_string(), "");
    metric(&mut html, "Lost", &lost.to_string(), "");
    metric(&mut html, "Reordered", &out_of_order.to_string(), "");
    metric(&mut html, "Duplicates", &duplicate.to_string(), "");
    metric(&mut html, "Jitter", &format!("{:.1}", jitter), "ms");
    metric(&mut html, "Bytes", &bytes.to_string(), "");
    html.push_str("</div></aside>");

    // Mode table.
    html.push_str("<section class=\"card wide\"><h2>Detected modes</h2><table><thead><tr><th>#</th><th class=\"num\">Frequency</th><th class=\"num\">Damping</th><th class=\"num\">Q</th><th class=\"num\">Amplitude</th><th class=\"num\">Persistence</th></tr></thead><tbody id=\"modes\">");
    if analysis.modes.is_empty() {
        html.push_str(
            "<tr><td colspan=\"6\" class=\"muted\">No stable modes were extracted.</td></tr>",
        );
    } else {
        for (index, mode) in analysis.modes.iter().enumerate() {
            let _ = write!(
                html,
                "<tr class=\"mode-row\" data-mode=\"{}\" tabindex=\"0\" title=\"highlight this mode in the spectrum\"><td>M{}</td><td class=\"num\">{:.1} Hz</td><td class=\"num\">{:.5}</td><td class=\"num\">{:.0}</td><td class=\"num\">{:.3}</td><td class=\"num\">{:.0}%</td></tr>",
                index,
                index + 1,
                mode.frequency_hz,
                mode.damping_ratio,
                mode.q_factor,
                mode.amplitude,
                mode.persistence * 100.0
            );
        }
    }
    html.push_str("</tbody></table></section>");

    // Flags.
    html.push_str("<aside class=\"card side\"><h2>Assessment</h2><ul class=\"flags\">");
    if run.assessment.flags.is_empty() {
        html.push_str("<li>No flags.</li>");
    } else {
        for flag in &run.assessment.flags {
            let _ = write!(html, "<li>{}</li>", html_escape(flag));
        }
    }
    html.push_str("</ul></aside>");

    // Truth.
    html.push_str("<section class=\"card full\"><h2>Digital-twin ground truth</h2>");
    let _ = write!(
        html,
        "<p><strong>{}</strong> — {}</p>",
        html_escape(&run.truth.condition),
        html_escape(&run.truth.description)
    );
    if !run.truth.modal_frequencies_hz.is_empty() {
        let values: Vec<String> = run
            .truth
            .modal_frequencies_hz
            .iter()
            .map(|f| format!("{f:.1} Hz"))
            .collect();
        let _ = write!(
            html,
            "<p class=\"small\"><strong>Simulated modes:</strong> {}</p>",
            html_escape(&values.join(", "))
        );
    }
    for note in &run.truth.notes {
        let _ = write!(html, "<p class=\"small muted\">{}</p>", html_escape(note));
    }
    html.push_str("</section>");

    html.push_str("</div></main><footer class=\"wrap\">Generated by <code>granite-twin scope</code>. The audio is synthesized from the simulated velocity signal; a real laser vibrometer would produce a similar modal ring-down, not speech.</footer>");

    html.push_str("<script>const DATA = ");
    html.push_str(&data);
    html.push_str(";</script><script>");
    html.push_str(SCOPE_JS);
    html.push_str("</script></body></html>");
    html
}

/// Write the animated scope to `path`.
pub fn write_scope(run: &ScenarioRun, path: &Path) -> io::Result<()> {
    fs::write(path, render_scope_html(run))
}

/// Render a dashboard that switches between the per-scenario scopes without
/// reloading the page.  Each scope is loaded in an iframe, so the heavy sample
/// payloads stay isolated and only the selected scenario is decoded.
pub fn render_showcase_html(entries: &[SuiteEntry]) -> String {
    let selected = entries
        .iter()
        .find(|entry| entry.scenario == "crack")
        .or_else(|| entries.first());
    let matched = entries
        .iter()
        .filter(|entry| {
            entry.expected.eq_ignore_ascii_case(&entry.predicted)
                || (entry.expected.eq_ignore_ascii_case("SUSPECT")
                    && !entry.predicted.eq_ignore_ascii_case("HEALTHY"))
        })
        .count();

    let mut html = String::with_capacity(24 * 1024 + entries.len() * 1024);
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Granite NDT — interactive twin</title><style>",
    );
    html.push_str(SCOPE_CSS);
    html.push_str(SHOWCASE_CSS);
    html.push_str(
        "</style></head><body class=\"showcase-body\"><header><div class=\"wrap\"><div class=\"eyebrow\"><span class=\"live\">● LIVE</span> INTERACTIVE DIGITAL TWIN</div><h1>Granite NDT — the whole suite, one control room</h1><p class=\"lede\">Choose a scenario on the left. Each view is the full animated scope for that run: waveform, trigger envelope, modal spectrum, packet stream and synthesized audio. Everything is generated from the Rust digital twin — no server and no external assets.</p>",
    );
    let _ = write!(
        html,
        "<div class=\"meta\">{} scenarios · {} matched · seed-deterministic · click a scenario to switch</div>",
        entries.len(),
        matched
    );
    html.push_str("</div></header><main class=\"showcase-main\"><aside class=\"scenario-list\"><h2>Scenarios</h2><div class=\"scenario-buttons\">");
    for (index, entry) in entries.iter().enumerate() {
        let active = selected
            .map(|selected| selected.scenario == entry.scenario)
            .unwrap_or(index == 0);
        let class = entry.predicted.to_lowercase();
        let _ = write!(
            html,
            "<button type=\"button\" class=\"scenario-btn{}\" data-scope=\"{}\" data-report=\"{}\" data-title=\"{}\" aria-pressed=\"{}\">\
             <span class=\"status-dot {}\"></span>\
             <span class=\"scenario-name\">{}</span>\
             <span class=\"scenario-score\">{:.0}</span>\
             <span class=\"scenario-tags\">expected {} · predicted {}</span>\
             </button>",
            if active { " active" } else { "" },
            html_escape(&entry.scope_path),
            html_escape(&entry.report_path),
            html_escape(&entry.title),
            active,
            class,
            html_escape(&entry.scenario),
            entry.score,
            html_escape(&entry.expected),
            html_escape(&entry.predicted)
        );
    }
    html.push_str("</div><div class=\"showcase-links\">");
    if let Some(entry) = selected {
        let _ = write!(
            html,
            "<a id=\"report-link\" href=\"{}\">Open report</a><a id=\"scope-link\" href=\"{}\" target=\"_blank\" rel=\"noopener\">Open scope in a new tab</a>",
            html_escape(&entry.report_path),
            html_escape(&entry.scope_path)
        );
    }
    html.push_str("</div><div class=\"hintline\">Tip: press <kbd>space</kbd> inside the scope to pause, click the trace to seek, and click 🔊 for sound.</div></aside>");
    html.push_str(
        "<section class=\"viewer\"><div class=\"viewer-head\"><span id=\"viewer-title\">",
    );
    if let Some(entry) = selected {
        html.push_str(&html_escape(&entry.title));
    }
    html.push_str("</span><span class=\"small muted\">live canvas · interactive · audio</span></div><iframe id=\"view\" title=\"Animated granite NDT scope\" allow=\"autoplay\" src=\"");
    if let Some(entry) = selected {
        html.push_str(&html_escape(&entry.scope_path));
    }
    html.push_str("\"></iframe></section></main><footer class=\"wrap\">Generated by <code>granite-twin suite</code>. The iframe is a self-contained scope file; the dashboard itself is also a single offline HTML file.</footer>");
    html.push_str("<script>");
    html.push_str(SHOWCASE_JS);
    html.push_str("</script>");
    html.push_str("</body></html>");
    html
}

/// Write the interactive suite dashboard next to the individual scopes.
pub fn write_showcase(entries: &[SuiteEntry], path: &Path) -> io::Result<()> {
    fs::write(path, render_showcase_html(entries))
}

fn metric(out: &mut String, label: &str, value: &str, unit: &str) {
    let _ = write!(
        out,
        "<div class=\"metric\"><div class=\"label\">{}</div><div class=\"value\">{} <span>{}</span></div></div>",
        html_escape(label),
        html_escape(value),
        html_escape(unit)
    );
}

fn downsample_max(values: &[f64], target: usize) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let target = target.min(values.len()).max(1);
    let mut out = Vec::with_capacity(target);
    for index in 0..target {
        let start = index * values.len() / target;
        let end = ((index + 1) * values.len() / target)
            .max(start + 1)
            .min(values.len());
        let value = values[start..end].iter().cloned().fold(0.0f64, f64::max);
        out.push(value);
    }
    out
}

fn spectrum_points(
    analysis: &CaptureAnalysis,
    max_frequency_hz: f64,
    target: usize,
) -> (Vec<f64>, Vec<f64>) {
    let spectrum = &analysis.averaged_spectrum;
    if spectrum.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let end = spectrum
        .freqs_hz
        .partition_point(|frequency| *frequency <= max_frequency_hz)
        .max(1)
        .min(spectrum.len());
    let target = target.min(end).max(1);
    let mut freqs = Vec::with_capacity(target);
    let mut db = Vec::with_capacity(target);
    for index in 0..target {
        let start = index * end / target;
        let stop = ((index + 1) * end / target).max(start + 1).min(end);
        let mut best = start;
        for candidate in start..stop {
            if spectrum.magnitude_db[candidate] > spectrum.magnitude_db[best] {
                best = candidate;
            }
        }
        freqs.push(spectrum.freqs_hz[best]);
        db.push(spectrum.magnitude_db[best]);
    }
    (freqs, db)
}

fn push_f64_array(out: &mut String, values: &[f64]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_number(*value));
    }
    out.push(']');
}

fn json_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "0".to_string()
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

const SCOPE_CSS: &str = r##"
:root {
  --bg: #060b14;
  --panel: #0d1727;
  --panel2: #101e32;
  --line: #20324b;
  --ink: #e8f0fb;
  --muted: #8fa6c4;
  --cyan: #4cc9f0;
  --orange: #ffb703;
  --red: #ef476f;
  --green: #06d6a0;
  --purple: #b388ff;
}
* { box-sizing: border-box; }
html, body { margin: 0; background: radial-gradient(circle at 20% 0%, #10233c 0, #060b14 48%, #04070d 100%); color: var(--ink); font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; line-height: 1.45; }
body { min-height: 100vh; }
.wrap { max-width: 1280px; margin: 0 auto; padding: 0 20px; }
header { padding: 30px 0 18px; border-bottom: 1px solid rgba(76, 201, 240, .16); background: linear-gradient(180deg, rgba(16, 35, 60, .72), rgba(6, 11, 20, 0)); }
.eyebrow { font-size: 11px; letter-spacing: .18em; color: var(--cyan); text-transform: uppercase; font-weight: 700; }
h1 { margin: 8px 0 6px; font-size: clamp(22px, 3vw, 34px); letter-spacing: -.03em; }
h2 { margin: 0 0 12px; font-size: 16px; letter-spacing: -.01em; }
.lede { margin: 0; color: #b9c9df; max-width: 900px; }
.meta { margin-top: 10px; color: var(--muted); font-size: 12px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
main { padding: 20px 20px 8px; }
.card { background: linear-gradient(180deg, rgba(16, 30, 50, .92), rgba(10, 19, 33, .92)); border: 1px solid var(--line); border-radius: 16px; padding: 16px; box-shadow: 0 18px 50px rgba(0, 0, 0, .25); }
.card-head { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; flex-wrap: wrap; }
.hero { display: flex; justify-content: space-between; gap: 20px; align-items: center; margin-bottom: 16px; flex-wrap: wrap; }
.hero-right { text-align: right; }
.badges { display: flex; gap: 8px; flex-wrap: wrap; }
.badge { display: inline-block; padding: 5px 10px; border-radius: 999px; font-size: 11px; font-weight: 800; letter-spacing: .06em; color: #04121c; background: var(--cyan); }
.badge.ghost { background: transparent; color: var(--muted); border: 1px solid var(--line); }
.badge.healthy { background: var(--green); }
.badge.suspect { background: var(--orange); }
.badge.defect { background: var(--red); color: #fff; }
.badge.unknown { background: var(--muted); }
.scoreline { display: flex; align-items: baseline; gap: 8px; margin-top: 8px; }
.score { font-size: 44px; font-weight: 800; letter-spacing: -.05em; line-height: 1; }
.score-unit { color: var(--muted); font-size: 13px; }
.kicker { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .12em; }
.big { font-size: 34px; font-weight: 800; letter-spacing: -.04em; }
.big span { font-size: 14px; color: var(--muted); font-weight: 600; }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; color: #cfe0f5; }
.grid { display: grid; grid-template-columns: minmax(0, 1fr) 340px; gap: 16px; align-items: start; }
.wide { grid-column: 1; }
.side { grid-column: 2; }
.full { grid-column: 1 / -1; margin-top: 16px; }
@media (max-width: 920px) { .grid { grid-template-columns: 1fr; } .wide, .side { grid-column: 1; } .hero-right { text-align: left; } }
canvas { width: 100%; height: auto; display: block; border-radius: 10px; background: #060c16; border: 1px solid #172840; }
#wave { margin-top: 4px; }
#env { margin-top: 6px; height: 88px; }
input[type=range] { width: 100%; margin: 10px 0 4px; accent-color: var(--cyan); }
.controls { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
button, select { background: #16263d; color: var(--ink); border: 1px solid #2a405e; border-radius: 9px; padding: 7px 11px; font: inherit; font-size: 13px; cursor: pointer; }
button:hover, select:hover { border-color: var(--cyan); }
button.primary { background: var(--cyan); color: #04121c; border-color: var(--cyan); font-weight: 800; }
button.primary:hover { background: #7adcf7; }
.led { font-size: 10px; letter-spacing: .08em; padding: 4px 8px; border-radius: 999px; border: 1px solid #2a405e; color: var(--muted); }
.led.on { background: var(--orange); color: #1b1200; border-color: var(--orange); box-shadow: 0 0 18px rgba(255, 183, 3, .55); }
.live { color: var(--red); margin-right: 8px; animation: pulse 1.8s ease-in-out infinite; }
@keyframes pulse { 0%, 100% { opacity: .35; } 50% { opacity: 1; text-shadow: 0 0 12px rgba(239, 71, 111, .9); } }
#wave, #env { cursor: crosshair; }
.readout { color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11px; min-width: 132px; }
.hintline { color: var(--muted); font-size: 11px; margin-top: 8px; }
kbd { background: #16263d; border: 1px solid #2a405e; border-bottom-width: 2px; border-radius: 5px; padding: 1px 5px; font: inherit; font-size: 10px; color: #cfe0f5; }
tr.mode-row { cursor: pointer; transition: background .12s, box-shadow .12s; }
tr.mode-row:hover { background: rgba(76, 201, 240, .08); }
tr.mode-row.selected { background: rgba(255, 183, 3, .13); box-shadow: inset 3px 0 0 var(--orange); }
.legend { display: flex; gap: 14px; flex-wrap: wrap; color: var(--muted); font-size: 11px; margin-top: 6px; }
.dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; margin-right: 4px; }
.dot.cyan { background: var(--cyan); }
.dot.orange { background: var(--orange); }
.dot.red { background: var(--red); }
.dot.white { background: #fff; }
.metrics { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.metrics.tight { margin-top: 10px; }
.metric { background: rgba(8, 16, 29, .7); border: 1px solid #172840; border-radius: 10px; padding: 8px 10px; }
.metric .label { color: var(--muted); font-size: 10px; text-transform: uppercase; letter-spacing: .08em; }
.metric .value { font-size: 18px; font-weight: 700; font-variant-numeric: tabular-nums; }
.metric .value span { color: var(--muted); font-size: 11px; font-weight: 500; }
.quality { margin-top: 12px; padding: 10px; border-radius: 10px; background: rgba(76, 201, 240, .06); border: 1px solid rgba(76, 201, 240, .18); font-size: 13px; }
.packets { display: grid; grid-template-columns: repeat(16, minmax(0, 1fr)); gap: 3px; margin: 2px 0 6px; }
.pkt { height: 13px; border-radius: 3px; background: #1a2a41; transition: outline .1s, background .1s; }
.pkt.lost { background: #5a1b2a; }
.pkt.active { outline: 2px solid var(--orange); outline-offset: 1px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 7px 8px; border-bottom: 1px solid #172840; }
th { color: var(--muted); font-size: 10px; text-transform: uppercase; letter-spacing: .07em; }
tr:last-child td { border-bottom: 0; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
.flags { margin: 0; padding-left: 18px; color: #c8d8ed; font-size: 13px; }
.flags li { margin: 6px 0; }
code { background: #16263d; padding: 1px 5px; border-radius: 5px; }
footer { padding: 16px 20px 34px; color: var(--muted); font-size: 12px; }
"##;

const SCOPE_JS: &str = r##"
const FS = DATA.fs;
const DUR = Math.max(DATA.duration, 0.001);
const WAVE = document.getElementById('wave');
const WCTX = WAVE.getContext('2d');
const ENV = document.getElementById('env');
const ECTX = ENV.getContext('2d');
const SPEC = document.getElementById('spectrum');
const SCTX = SPEC.getContext('2d');
const playBtn = document.getElementById('play');
const nextBtn = document.getElementById('next');
const restartBtn = document.getElementById('restart');
const speedSel = document.getElementById('speed');
const muteBtn = document.getElementById('mute');
const seek = document.getElementById('seek');
const timeEl = document.getElementById('time');
const triggerEl = document.getElementById('trigger');
const readoutEl = document.getElementById('readout');
const packetGrid = document.getElementById('packets');

function b64ToBytes(b64) {
  const bin = atob(b64 || '');
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
function decodeSamples(b64) {
  const bytes = b64ToBytes(b64);
  const view = new DataView(bytes.buffer);
  const out = new Int16Array(bytes.length >> 1);
  for (let i = 0; i < out.length; i++) out[i] = view.getInt16(i * 2, true);
  return out;
}
const samples = decodeSamples(DATA.samples_b64);

function makeWav(int16, rate) {
  const n = int16.length;
  const buffer = new ArrayBuffer(44 + n * 2);
  const view = new DataView(buffer);
  const write = (offset, text) => {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
  };
  write(0, 'RIFF'); view.setUint32(4, 36 + n * 2, true); write(8, 'WAVE');
  write(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, 1, true); view.setUint32(24, rate, true); view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true); write(36, 'data');
  view.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) view.setInt16(44 + i * 2, int16[i], true);
  return new Blob([buffer], {type: 'audio/wav'});
}

const audio = new Audio();
audio.preload = 'auto';
audio.volume = 0.85;
audio.loop = true;
let audioClock = false;
if (samples.length) {
  audio.src = URL.createObjectURL(makeWav(samples, Math.round(FS)));
}
audio.addEventListener('playing', () => { audioClock = true; });
audio.addEventListener('pause', () => { audioClock = false; });
audio.addEventListener('ended', () => { audioClock = false; });

let simTime = 0;
let playing = true;
let speed = 0.5;
let muted = true;
let dragging = false;
let hoverTime = null;
let selectedMode = -1;
let lastFrame = performance.now();

// Try a muted autoplay so the scope is visibly alive on open.  Browsers that
// block it simply fall back to the internal animation clock; unmuting with the
// speaker button starts the audio from a real user gesture.
if (audio.src) {
  audio.muted = true;
  try {
    const promise = audio.play();
    if (promise && promise.catch) promise.catch(() => {});
  } catch (error) {}
}

function updateControls() {
  playBtn.textContent = playing ? '⏸ Pause' : '▶ Play';
  muteBtn.textContent = muted ? '🔇' : '🔊';
  muteBtn.title = muted ? 'enable sound' : 'mute audio';
  timeEl.textContent = simTime.toFixed(2) + ' / ' + DUR.toFixed(2) + ' s';
}
function setSimTime(value) {
  simTime = Math.max(0, Math.min(DUR, value));
  if (audio.src) {
    try { audio.currentTime = simTime; } catch (error) {}
  }
  updateControls();
}
function startAudio() {
  if (!audio.src) return;
  try {
    audio.currentTime = simTime;
    audio.playbackRate = speed;
    audio.muted = muted;
    const promise = audio.play();
    if (promise && promise.catch) promise.catch(() => {});
  } catch (error) {}
}
function togglePlay() {
  if (!samples.length) return;
  if (!playing) {
    if (simTime >= DUR - 0.001) simTime = 0;
    playing = true;
    startAudio();
  } else {
    playing = false;
    audio.pause();
  }
  lastFrame = performance.now();
  updateControls();
}
function restart() {
  setSimTime(0);
  if (playing) startAudio();
}
function nextTap() {
  if (!DATA.events.length) return;
  let target = DATA.events[0];
  for (const event of DATA.events) {
    if (event.time > simTime + 0.02) { target = event; break; }
  }
  setSimTime(Math.max(0, target.time - 0.04));
  playing = true;
  startAudio();
  lastFrame = performance.now();
  updateControls();
}

playBtn.addEventListener('click', togglePlay);
nextBtn.addEventListener('click', nextTap);
restartBtn.addEventListener('click', restart);
speedSel.addEventListener('change', () => {
  speed = parseFloat(speedSel.value) || 0.5;
  if (audio.src) audio.playbackRate = speed;
});
muteBtn.addEventListener('click', () => {
  muted = !muted;
  audio.muted = muted;
  if (!muted && audio.paused) startAudio();
  updateControls();
});
seek.addEventListener('pointerdown', () => { dragging = true; });
window.addEventListener('pointerup', () => { dragging = false; });
seek.addEventListener('input', () => { setSimTime((parseInt(seek.value, 10) / 1000) * DUR); });

function canvasSeek(event, canvas) {
  const rect = canvas.getBoundingClientRect();
  if (!rect.width) return;
  const [start, end] = viewWindow();
  const x = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  setSimTime(start + x * (end - start));
}
WAVE.addEventListener('click', (event) => canvasSeek(event, WAVE));
ENV.addEventListener('click', (event) => canvasSeek(event, ENV));
WAVE.addEventListener('pointermove', (event) => {
  const rect = WAVE.getBoundingClientRect();
  if (!rect.width) return;
  const [start, end] = viewWindow();
  const x = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  hoverTime = start + x * (end - start);
  const index = Math.min(samples.length - 1, Math.max(0, Math.floor(hoverTime * FS)));
  const value = samples.length ? samples[index] / 32767 : 0;
  readoutEl.textContent = hoverTime.toFixed(3) + ' s · ' + value.toFixed(3);
});
WAVE.addEventListener('pointerleave', () => {
  hoverTime = null;
  readoutEl.textContent = 'click trace to seek';
});

const modeRows = Array.from(document.querySelectorAll('#modes tr[data-mode]'));
function selectMode(index) {
  selectedMode = selectedMode === index ? -1 : index;
  modeRows.forEach((row) => {
    row.classList.toggle('selected', Number(row.dataset.mode) === selectedMode);
  });
}
modeRows.forEach((row) => {
  row.addEventListener('click', () => selectMode(Number(row.dataset.mode)));
  row.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      selectMode(Number(row.dataset.mode));
    }
  });
});

window.addEventListener('keydown', (event) => {
  const tag = event.target && event.target.tagName;
  if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA' || tag === 'BUTTON') return;
  if (event.code === 'Space') {
    event.preventDefault();
    togglePlay();
  } else if (event.key === 'ArrowRight') {
    event.preventDefault();
    setSimTime(simTime + 0.05);
  } else if (event.key === 'ArrowLeft') {
    event.preventDefault();
    setSimTime(simTime - 0.05);
  }
});

function viewWindow() {
  const length = 0.38;
  let end = Math.min(DUR, Math.max(length, simTime + 0.06));
  let start = Math.max(0, end - length);
  return [start, end];
}

function drawWave() {
  const W = WAVE.width;
  const H = WAVE.height;
  WCTX.clearRect(0, 0, W, H);
  const [start, end] = viewWindow();
  WCTX.strokeStyle = 'rgba(76,201,240,.09)';
  WCTX.lineWidth = 1;
  for (let i = 0; i <= 6; i++) {
    const x = (i / 6) * W;
    WCTX.beginPath(); WCTX.moveTo(x, 0); WCTX.lineTo(x, H); WCTX.stroke();
  }
  for (let i = 0; i <= 2; i++) {
    const y = H * 0.13 + i * H * 0.37;
    WCTX.beginPath(); WCTX.moveTo(0, y); WCTX.lineTo(W, y); WCTX.stroke();
  }
  for (const gap of DATA.gaps) {
    const a = gap.start;
    const b = gap.start + gap.len;
    if (b < start || a > end) continue;
    const x1 = ((a - start) / (end - start)) * W;
    const x2 = ((b - start) / (end - start)) * W;
    WCTX.fillStyle = 'rgba(239,71,111,.14)';
    WCTX.fillRect(x1, 0, Math.max(1, x2 - x1), H);
  }
  if (samples.length) {
    const i0 = Math.max(0, Math.floor(start * FS));
    const i1 = Math.min(samples.length, Math.ceil(end * FS));
    if (i1 > i0) {
      const mid = H * 0.53;
      const amp = H * 0.40;
      WCTX.strokeStyle = '#4cc9f0';
      WCTX.lineWidth = 1.1;
      WCTX.shadowColor = 'rgba(76,201,240,.55)';
      WCTX.shadowBlur = 6;
      WCTX.beginPath();
      for (let x = 0; x < W; x++) {
        const a = i0 + Math.floor(((i1 - i0) * x) / W);
        const b = Math.max(a + 1, i0 + Math.floor(((i1 - i0) * (x + 1)) / W));
        let lo = 1;
        let hi = -1;
        for (let i = a; i < b; i++) {
          const value = samples[i] / 32767;
          if (value < lo) lo = value;
          if (value > hi) hi = value;
        }
        const y1 = mid - hi * amp;
        const y2 = Math.max(mid - lo * amp, y1 + 0.8);
        WCTX.moveTo(x + 0.5, y1);
        WCTX.lineTo(x + 0.5, y2);
      }
      WCTX.stroke();
      WCTX.shadowBlur = 0;
    }
  }
  WCTX.strokeStyle = 'rgba(143,166,196,.25)';
  WCTX.beginPath(); WCTX.moveTo(0, H * 0.53); WCTX.lineTo(W, H * 0.53); WCTX.stroke();
  for (const event of DATA.events) {
    if (event.time < start || event.time > end) continue;
    const x = ((event.time - start) / (end - start)) * W;
    WCTX.save();
    WCTX.strokeStyle = 'rgba(255,183,3,.85)';
    WCTX.setLineDash([5, 5]);
    WCTX.beginPath(); WCTX.moveTo(x, 0); WCTX.lineTo(x, H); WCTX.stroke();
    WCTX.restore();
    WCTX.fillStyle = '#ffb703';
    WCTX.font = '11px ui-monospace, monospace';
    WCTX.fillText('E' + (event.index + 1), Math.min(W - 24, x + 4), 14);
  }
  const cx = ((simTime - start) / (end - start)) * W;
  if (cx >= 0 && cx <= W) {
    WCTX.strokeStyle = 'rgba(255,255,255,.75)';
    WCTX.beginPath(); WCTX.moveTo(cx, 0); WCTX.lineTo(cx, H); WCTX.stroke();
    WCTX.fillStyle = '#ffffff';
    WCTX.beginPath(); WCTX.arc(cx, H * 0.53, 3, 0, Math.PI * 2); WCTX.fill();
  }
  if (hoverTime !== null && hoverTime >= start && hoverTime <= end) {
    const hx = ((hoverTime - start) / (end - start)) * W;
    WCTX.save();
    WCTX.strokeStyle = 'rgba(76,201,240,.55)';
    WCTX.setLineDash([3, 4]);
    WCTX.beginPath(); WCTX.moveTo(hx, 0); WCTX.lineTo(hx, H); WCTX.stroke();
    WCTX.restore();
  }
  WCTX.fillStyle = '#8fa6c4';
  WCTX.font = '11px ui-monospace, monospace';
  WCTX.fillText('+1', 6, 14);
  WCTX.fillText('0', 6, H * 0.53 + 4);
  WCTX.fillText('-1', 6, H * 0.53 + 18);
  WCTX.fillText(start.toFixed(2) + 's', 42, H - 8);
  WCTX.fillText(end.toFixed(2) + 's', W - 54, H - 8);
}

function drawEnv() {
  const W = ENV.width;
  const H = ENV.height;
  ECTX.clearRect(0, 0, W, H);
  const [start, end] = viewWindow();
  let maxEnv = Math.max(DATA.threshold * 1.25, 1e-9);
  for (const value of DATA.envelope) maxEnv = Math.max(maxEnv, value);
  ECTX.fillStyle = 'rgba(76,201,240,.08)';
  ECTX.fillRect(0, 0, W, H);
  if (DATA.envelope.length) {
    ECTX.beginPath();
    ECTX.moveTo(0, H);
    for (let x = 0; x < W; x++) {
      const t = start + ((end - start) * x) / W;
      const index = Math.min(DATA.envelope.length - 1, Math.max(0, Math.floor((t / DUR) * DATA.envelope.length)));
      const y = H - 4 - (DATA.envelope[index] / maxEnv) * (H - 10);
      ECTX.lineTo(x, y);
    }
    ECTX.lineTo(W, H);
    ECTX.closePath();
    ECTX.fillStyle = 'rgba(76,201,240,.22)';
    ECTX.fill();
    ECTX.strokeStyle = '#4cc9f0';
    ECTX.lineWidth = 1.4;
    ECTX.stroke();
  }
  const thresholdY = H - 4 - (DATA.threshold / maxEnv) * (H - 10);
  ECTX.save();
  ECTX.strokeStyle = 'rgba(255,183,3,.9)';
  ECTX.setLineDash([4, 4]);
  ECTX.beginPath(); ECTX.moveTo(0, thresholdY); ECTX.lineTo(W, thresholdY); ECTX.stroke();
  ECTX.restore();
  ECTX.fillStyle = '#ffb703';
  ECTX.font = '10px ui-monospace, monospace';
  ECTX.fillText('trigger threshold', 6, Math.max(11, thresholdY - 4));
  const cx = ((simTime - start) / (end - start)) * W;
  if (cx >= 0 && cx <= W) {
    ECTX.strokeStyle = 'rgba(255,255,255,.55)';
    ECTX.beginPath(); ECTX.moveTo(cx, 0); ECTX.lineTo(cx, H); ECTX.stroke();
  }
}

function drawSpectrum() {
  const W = SPEC.width;
  const H = SPEC.height;
  SCTX.clearRect(0, 0, W, H);
  const spec = DATA.spectrum;
  if (!spec.freqs.length) {
    SCTX.fillStyle = '#8fa6c4';
    SCTX.font = '14px ui-sans-serif, sans-serif';
    SCTX.fillText('No spectrum available.', 20, 32);
    return;
  }
  const padL = 54;
  const padR = 16;
  const padT = 20;
  const padB = 30;
  const pw = W - padL - padR;
  const ph = H - padT - padB;
  const maxF = Math.max(1, spec.freqs[spec.freqs.length - 1]);
  let maxDb = -Infinity;
  for (const value of spec.db) maxDb = Math.max(maxDb, value);
  if (!isFinite(maxDb)) maxDb = 0;
  const minDb = maxDb - 65;
  const xFor = (f) => padL + (f / maxF) * pw;
  const yFor = (db) => padT + ((maxDb - Math.max(minDb, Math.min(maxDb, db))) / (maxDb - minDb)) * ph;
  SCTX.strokeStyle = 'rgba(76,201,240,.10)';
  SCTX.lineWidth = 1;
  const fStep = maxF > 5000 ? 2000 : maxF > 2000 ? 1000 : 500;
  for (let f = 0; f <= maxF; f += fStep) {
    const x = xFor(f);
    SCTX.beginPath(); SCTX.moveTo(x, padT); SCTX.lineTo(x, padT + ph); SCTX.stroke();
    SCTX.fillStyle = '#8fa6c4';
    SCTX.font = '10px ui-monospace, monospace';
    SCTX.fillText(f >= 1000 ? (f / 1000).toFixed(f % 1000 === 0 ? 0 : 1) + 'k' : f.toFixed(0), x - 8, H - 10);
  }
  for (let d = 0; d <= 60; d += 10) {
    const y = padT + (d / 65) * ph;
    SCTX.beginPath(); SCTX.moveTo(padL, y); SCTX.lineTo(padL + pw, y); SCTX.stroke();
    SCTX.fillStyle = '#8fa6c4';
    SCTX.font = '10px ui-monospace, monospace';
    SCTX.fillText('-' + d + ' dB', 4, y + 3);
  }
  const gradient = SCTX.createLinearGradient(0, padT, 0, padT + ph);
  gradient.addColorStop(0, 'rgba(76,201,240,.35)');
  gradient.addColorStop(1, 'rgba(76,201,240,.02)');
  SCTX.beginPath();
  SCTX.moveTo(xFor(spec.freqs[0]), yFor(spec.db[0]));
  for (let i = 1; i < spec.freqs.length; i++) SCTX.lineTo(xFor(spec.freqs[i]), yFor(spec.db[i]));
  SCTX.lineTo(xFor(spec.freqs[spec.freqs.length - 1]), padT + ph);
  SCTX.lineTo(xFor(spec.freqs[0]), padT + ph);
  SCTX.closePath();
  SCTX.fillStyle = gradient;
  SCTX.fill();
  SCTX.strokeStyle = '#4cc9f0';
  SCTX.lineWidth = 1.5;
  SCTX.beginPath();
  SCTX.moveTo(xFor(spec.freqs[0]), yFor(spec.db[0]));
  for (let i = 1; i < spec.freqs.length; i++) SCTX.lineTo(xFor(spec.freqs[i]), yFor(spec.db[i]));
  SCTX.stroke();
  const nearestDb = (f) => {
    let best = 0;
    for (let i = 1; i < spec.freqs.length; i++) {
      if (Math.abs(spec.freqs[i] - f) < Math.abs(spec.freqs[best] - f)) best = i;
    }
    return spec.db[best];
  };
  for (const ref of DATA.reference_modes) {
    if (ref.f > maxF) continue;
    const x = xFor(ref.f);
    SCTX.fillStyle = 'rgba(179,136,255,.8)';
    SCTX.beginPath();
    SCTX.moveTo(x, padT - 2);
    SCTX.lineTo(x - 4, padT - 10);
    SCTX.lineTo(x + 4, padT - 10);
    SCTX.closePath();
    SCTX.fill();
  }
  DATA.modes.forEach((mode, index) => {
    if (mode.f > maxF) return;
    const x = xFor(mode.f);
    const y = yFor(nearestDb(mode.f));
    const selected = index === selectedMode;
    if (selected) {
      SCTX.save();
      SCTX.strokeStyle = 'rgba(255,255,255,.9)';
      SCTX.lineWidth = 1.5;
      SCTX.setLineDash([3, 3]);
      SCTX.beginPath(); SCTX.arc(x, y, 9, 0, Math.PI * 2); SCTX.stroke();
      SCTX.restore();
    }
    SCTX.fillStyle = selected ? '#ffffff' : '#ffb703';
    SCTX.beginPath(); SCTX.arc(x, y, selected ? 5 : 4, 0, Math.PI * 2); SCTX.fill();
    SCTX.fillStyle = selected ? '#ffffff' : '#ffe6a8';
    SCTX.font = selected ? 'bold 11px ui-monospace, monospace' : '10px ui-monospace, monospace';
    SCTX.fillText('M' + (index + 1), x + 6, Math.max(padT + 10, y - 6));
  });
  SCTX.fillStyle = '#8fa6c4';
  SCTX.font = '10px ui-monospace, monospace';
  SCTX.fillText('Hz', padL + pw - 16, H - 10);
}

let seed = 0x12345678;
function random() {
  seed = (seed * 1664525 + 1013904223) >>> 0;
  return seed / 4294967296;
}
const packets = [];
for (let i = 0; i < 64; i++) {
  const block = document.createElement('div');
  const lost = random() < DATA.transport.packet_loss;
  block.className = 'pkt' + (lost ? ' lost' : '');
  block.title = 'packet ' + i + (lost ? ' (lost)' : '');
  packetGrid.appendChild(block);
  packets.push(block);
}
function updatePacket() {
  if (!packets.length) return;
  const active = Math.min(63, Math.max(0, Math.floor((simTime / DUR) * 64)));
  for (let i = 0; i < packets.length; i++) packets[i].classList.toggle('active', i === active);
}
function updateTrigger() {
  if (!DATA.envelope.length) {
    triggerEl.classList.remove('on');
    return;
  }
  const index = Math.min(DATA.envelope.length - 1, Math.max(0, Math.floor((simTime / DUR) * DATA.envelope.length)));
  triggerEl.classList.toggle('on', DATA.envelope[index] > DATA.threshold);
}

function frame(now) {
  const dt = Math.min(0.1, (now - lastFrame) / 1000);
  lastFrame = now;
  if (playing) {
    if (audioClock && !audio.paused) {
      simTime = audio.currentTime;
    } else {
      simTime += dt * speed;
    }
    if (simTime >= DUR) {
      if (audio.loop) {
        simTime = 0;
      } else {
        simTime = DUR;
        playing = false;
        audio.pause();
        updateControls();
      }
    }
  }
  if (!dragging) seek.value = Math.round((simTime / DUR) * 1000);
  timeEl.textContent = simTime.toFixed(2) + ' / ' + DUR.toFixed(2) + ' s';
  drawWave();
  drawEnv();
  updatePacket();
  updateTrigger();
  requestAnimationFrame(frame);
}

drawSpectrum();
updateControls();
requestAnimationFrame(frame);
"##;

const SHOWCASE_CSS: &str = r##"
.showcase-body { height: 100vh; overflow: hidden; }
.showcase-body header { padding: 20px 0 14px; }
.showcase-body h1 { margin: 6px 0 4px; font-size: clamp(20px, 2.4vw, 28px); }
.showcase-body .lede { font-size: 14px; max-width: 1080px; }
.showcase-main { display: grid; grid-template-columns: 310px minmax(0, 1fr); gap: 14px; padding: 14px 20px 8px; height: calc(100vh - 176px); }
.scenario-list { overflow-y: auto; padding-right: 4px; min-height: 0; }
.scenario-list h2 { margin: 2px 0 10px; }
.scenario-buttons { display: flex; flex-direction: column; gap: 6px; }
.scenario-btn { position: relative; display: grid; grid-template-columns: 10px minmax(0, 1fr) auto; grid-template-areas: "dot name score" ". tags tags"; row-gap: 2px; column-gap: 7px; width: 100%; padding: 9px 10px; background: rgba(13, 23, 39, .82); border: 1px solid #1d2f47; border-radius: 10px; color: var(--ink); text-align: left; font: inherit; cursor: pointer; transition: border-color .12s, background .12s, transform .12s; }
.scenario-btn:hover { border-color: rgba(76, 201, 240, .55); background: rgba(22, 38, 61, .95); transform: translateX(2px); }
.scenario-btn.active { border-color: var(--cyan); background: linear-gradient(90deg, rgba(76, 201, 240, .14), rgba(16, 30, 50, .92)); box-shadow: inset 3px 0 0 var(--cyan); }
.status-dot { grid-area: dot; width: 9px; height: 9px; border-radius: 50%; margin-top: 5px; background: var(--green); box-shadow: 0 0 10px rgba(6, 214, 160, .55); }
.status-dot.suspect { background: var(--orange); box-shadow: 0 0 10px rgba(255, 183, 3, .55); }
.status-dot.defect { background: var(--red); box-shadow: 0 0 10px rgba(239, 71, 111, .65); }
.status-dot.unknown { background: var(--muted); box-shadow: none; }
.scenario-name { grid-area: name; font-weight: 700; font-size: 13px; }
.scenario-score { grid-area: score; color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; font-weight: 800; }
.scenario-btn.active .scenario-score { color: var(--ink); }
.scenario-tags { grid-area: tags; color: var(--muted); font-size: 9px; letter-spacing: .06em; text-transform: uppercase; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.showcase-links { display: flex; gap: 14px; flex-wrap: wrap; margin-top: 12px; font-size: 12px; }
.showcase-links a { color: var(--cyan); text-decoration: none; border-bottom: 1px dashed rgba(76, 201, 240, .45); padding-bottom: 1px; }
.showcase-links a:hover { border-bottom-style: solid; }
.viewer { min-width: 0; height: 100%; display: flex; flex-direction: column; }
.viewer-head { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 0 4px 8px; font-weight: 700; font-size: 13px; }
#view { flex: 1; width: 100%; min-height: 0; border: 1px solid var(--line); border-radius: 16px; background: #060b14; box-shadow: 0 18px 50px rgba(0, 0, 0, .3); }
.showcase-body footer { padding: 8px 20px 14px; }
@media (max-width: 900px) {
  .showcase-body { height: auto; overflow: auto; }
  .showcase-main { grid-template-columns: 1fr; height: auto; }
  .scenario-list { overflow: visible; }
  .scenario-buttons { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); }
  .viewer { height: 80vh; }
}
"##;

const SHOWCASE_JS: &str = r##"
const view = document.getElementById('view');
const viewerTitle = document.getElementById('viewer-title');
const reportLink = document.getElementById('report-link');
const scopeLink = document.getElementById('scope-link');
const scenarioButtons = Array.from(document.querySelectorAll('.scenario-btn'));
function selectScenario(button) {
  scenarioButtons.forEach((candidate) => {
    const active = candidate === button;
    candidate.classList.toggle('active', active);
    candidate.setAttribute('aria-pressed', active ? 'true' : 'false');
  });
  view.src = button.dataset.scope;
  viewerTitle.textContent = button.dataset.title;
  if (reportLink) reportLink.href = button.dataset.report;
  if (scopeLink) scopeLink.href = button.dataset.scope;
}
scenarioButtons.forEach((button) => {
  button.addEventListener('click', () => selectScenario(button));
});
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{catalog, run_scenario};

    /// Check that the embedded data block is a single balanced JSON object and
    /// cannot terminate the surrounding `<script>` element.
    #[test]
    fn embedded_data_is_balanced_json() {
        let scenario = catalog()
            .into_iter()
            .find(|scenario| scenario.id == "crack")
            .expect("crack scenario");
        let run = run_scenario(&scenario, 42).expect("run");
        let html = render_scope_html(&run);

        let start = html.find("const DATA = ").expect("data block") + "const DATA = ".len();
        let end = html[start..].find("</script>").expect("closing tag") + start;
        let body = html[start..end].trim();
        assert!(body.ends_with(';'), "data block must end with a semicolon");
        let json = &body[..body.len() - 1];
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(
            !json.contains("</script>"),
            "data must not close the script tag"
        );

        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        let mut minimum = 0i32;
        for ch in json.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    minimum = minimum.min(depth);
                }
                _ => {}
            }
        }
        assert!(!in_string, "unterminated string in data block");
        assert_eq!(depth, 0, "unbalanced JSON in data block");
        assert_eq!(minimum, 0, "JSON closes before the end");

        assert!(json.contains("\"samples_b64\""));
        assert!(json.contains("\"spectrum\""));
        assert!(html.contains("id=\"wave\""));
        assert!(html.contains("id=\"spectrum\""));
        assert!(html.contains("let playing = true;"));
        assert!(html.contains("audio.loop = true;"));
    }

    #[test]
    fn showcase_links_every_scope() {
        let entry = |scenario: &str, predicted: &str, score: f64| SuiteEntry {
            title: scenario.to_string(),
            scenario: scenario.to_string(),
            expected: predicted.to_string(),
            predicted: predicted.to_string(),
            score,
            fundamental_hz: 350.0,
            damping_ratio: 0.002,
            packet_loss: 0.0,
            report_path: format!("{scenario}/report.html"),
            scope_path: format!("{scenario}/scope.html"),
        };
        let entries = vec![
            entry("healthy", "HEALTHY", 1.9),
            entry("crack", "DEFECT", 70.5),
        ];
        let html = render_showcase_html(&entries);
        assert!(html.contains("<script>"));
        assert!(html.contains("data-scope=\"healthy/scope.html\""));
        assert!(html.contains("data-scope=\"crack/scope.html\""));
        assert!(
            html.contains("<iframe id=\"view\" title=\"Animated granite NDT scope\" allow=\"autoplay\" src=\"crack/scope.html\""),
            "the crack scenario should be selected by default"
        );
        assert!(html.contains("class=\"scenario-btn active\""));
    }
}
