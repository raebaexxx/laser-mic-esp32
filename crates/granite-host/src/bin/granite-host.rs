//! `granite-host`: live UDP receiver, WAV analyser and report generator.

use std::path::PathBuf;
use std::process::ExitCode;

use granite_core::analysis::AnalysisConfig;
use granite_core::classify::{Condition, ReferenceProfile, SlabDescriptor};
use granite_host::{
    assess, load_wav, receive_udp, reference_from_capture, write_bundle, ReceiveOptions,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("granite-host: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str).unwrap_or("help") {
        "help" | "--help" | "-h" => {
            usage();
            Ok(ExitCode::SUCCESS)
        }
        "listen" => listen(&args),
        "analyze" => analyze(&args),
        "reference" => reference(&args),
        "selftest" => {
            granite_core::selftest().map_err(|error| error.to_string())?;
            println!("core self-test: ok");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command '{other}' (try --help)")),
    }
}

fn listen(args: &[String]) -> Result<ExitCode, String> {
    let duration = flag(args, "--duration")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(10.0);
    let idle = flag(args, "--idle")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(2.0);
    let options = ReceiveOptions {
        bind_addr: flag(args, "--bind").unwrap_or_else(|| "0.0.0.0:5005".to_string()),
        duration_s: if duration > 0.0 { Some(duration) } else { None },
        idle_timeout_s: idle.max(0.2),
        startup_timeout_s: flag(args, "--startup-timeout")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(20.0),
        channel: flag(args, "--channel")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0),
    };
    eprintln!(
        "listening on {} for {} ...",
        options.bind_addr,
        match options.duration_s {
            Some(seconds) => format!("{seconds:.1} s after the first packet"),
            None => format!("until {:.1} s of silence", options.idle_timeout_s),
        }
    );
    let outcome = receive_udp(&options).map_err(|error| error.to_string())?;
    if outcome.capture.samples.is_empty() {
        return Err("received no samples".to_string());
    }
    println!(
        "received {:.2} s, {} samples, {} packets, {} invalid, packet loss {:.2}%, gap samples {:.2}%, jitter {:.1} ms",
        outcome.capture.duration_s(),
        outcome.capture.samples.len(),
        outcome.stats.packets_received,
        outcome.invalid_packets,
        outcome.stats.packet_loss_fraction() * 100.0,
        outcome.capture.gap_fraction() * 100.0,
        outcome.stats.max_jitter_ms
    );
    let config = analysis_config();
    let analysis = granite_host::analyse(&outcome.capture, &config);
    let reference = load_reference(args)?;
    let assessment = reference
        .as_ref()
        .map(|reference| assess(&analysis, reference));
    let title = flag(args, "--title").unwrap_or_else(|| "Granite slab — live capture".to_string());
    let subtitle = flag(args, "--subtitle")
        .unwrap_or_else(|| "Live laser-vibrometer capture received over framed UDP.".to_string());
    let output = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out/live"));
    let mut notes = vec![format!(
        "Received {} packets in {:.2} s; {} invalid datagrams; {} reassembly gaps.",
        outcome.stats.packets_received,
        outcome.elapsed_s,
        outcome.invalid_packets,
        outcome.stats.gap_count
    )];
    notes.extend(outcome.status_messages.iter().cloned());
    let bundle = write_bundle(
        &output,
        &title,
        &subtitle,
        &outcome.capture,
        &analysis,
        assessment.as_ref(),
        reference.as_ref(),
        &notes,
    )
    .map_err(|error| error.to_string())?;
    print_summary(&analysis, assessment.as_ref(), &bundle.report_html);
    Ok(ExitCode::SUCCESS)
}

fn analyze(args: &[String]) -> Result<ExitCode, String> {
    let wav_path = flag(args, "--wav").ok_or("missing --wav FILE")?;
    let capture = load_wav(&PathBuf::from(&wav_path)).map_err(|error| error.to_string())?;
    println!(
        "loaded {}: {:.2} s at {:.1} Hz ({} samples)",
        wav_path,
        capture.duration_s(),
        capture.sample_rate_hz,
        capture.samples.len()
    );
    let config = analysis_config();
    let analysis = granite_host::analyse(&capture, &config);
    let reference = load_reference(args)?;
    let assessment = reference
        .as_ref()
        .map(|reference| assess(&analysis, reference));
    let title = flag(args, "--title").unwrap_or_else(|| "Granite slab — file analysis".to_string());
    let subtitle =
        flag(args, "--subtitle").unwrap_or_else(|| format!("Offline analysis of {wav_path}."));
    let output = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out/analysis"));
    let notes = vec![format!(
        "Input file: {wav_path}. Analysis is identical to the live UDP path; only the transport statistics differ."
    )];
    let bundle = write_bundle(
        &output,
        &title,
        &subtitle,
        &capture,
        &analysis,
        assessment.as_ref(),
        reference.as_ref(),
        &notes,
    )
    .map_err(|error| error.to_string())?;
    print_summary(&analysis, assessment.as_ref(), &bundle.report_html);
    Ok(ExitCode::SUCCESS)
}

fn reference(args: &[String]) -> Result<ExitCode, String> {
    let wav_path = flag(args, "--wav").ok_or("missing --wav FILE")?;
    let capture = load_wav(&PathBuf::from(&wav_path)).map_err(|error| error.to_string())?;
    let name = flag(args, "--name").unwrap_or_else(|| "healthy reference".to_string());
    let slab = SlabDescriptor {
        length_m: flag(args, "--length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.600),
        width_m: flag(args, "--width")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.400),
        thickness_m: flag(args, "--thickness")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.020),
        material: flag(args, "--material").unwrap_or_else(|| "granite".to_string()),
    };
    let profile = reference_from_capture(name.clone(), &capture, &analysis_config(), Some(slab))
        .map_err(|error| format!("cannot build reference: {error}"))?;
    let output = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reference.txt"));
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    profile.save(&output).map_err(|error| error.to_string())?;
    println!(
        "saved reference '{name}' with {} modes to {}",
        profile.modes.len(),
        output.display()
    );
    for (index, mode) in profile.modes.iter().enumerate() {
        println!(
            "  mode {}: {:.1} Hz, damping {:.4}, tolerance ±{:.1} Hz / ±{:.4}",
            index + 1,
            mode.frequency_hz,
            mode.damping_ratio,
            mode.frequency_tol_hz,
            mode.damping_tol
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn load_reference(args: &[String]) -> Result<Option<ReferenceProfile>, String> {
    match flag(args, "--reference") {
        Some(path) => ReferenceProfile::load(&PathBuf::from(&path))
            .map(Some)
            .map_err(|error| format!("cannot load reference '{path}': {error}")),
        None => Ok(None),
    }
}

fn analysis_config() -> AnalysisConfig {
    AnalysisConfig {
        // Keep the full band above the 20 Hz high-pass; larger slabs can have
        // fundamentals below 40 Hz.
        min_frequency_hz: 20.0,
        ..Default::default()
    }
}

fn print_summary(
    analysis: &granite_core::analysis::CaptureAnalysis,
    assessment: Option<&granite_core::classify::Assessment>,
    report: &std::path::Path,
) {
    println!(
        "modes: {} stable / {} impacts; f1 {:.1} Hz; mean damping {:.5}; T60 {:.2} s",
        analysis.modes.len(),
        analysis.events.len(),
        analysis.features.fundamental_hz,
        analysis.features.mean_damping,
        analysis.features.decay_t60_s
    );
    println!(
        "quality: usable={}, trigger SNR {:.1} dB, clipping {:.3}%, gaps {:.2}%",
        analysis.quality.usable,
        analysis.quality.snr_db,
        analysis.quality.clipping_fraction * 100.0,
        analysis.quality.gap_fraction * 100.0
    );
    if let Some(assessment) = assessment {
        println!(
            "condition: {} (score {:.1}, confidence {:.0}%)",
            assessment.condition.label(),
            assessment.anomaly_score,
            assessment.confidence * 100.0
        );
        for flag in &assessment.flags {
            println!("  - {flag}");
        }
        if assessment.condition == Condition::Unknown {
            println!("  note: no reliable classification; check the data-quality warnings");
        }
    } else {
        println!("condition: not classified (no reference profile supplied)");
    }
    for warning in &analysis.quality.warnings {
        println!("  warning: {warning}");
    }
    println!("report: {}", report.display());
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn usage() {
    println!(
        "granite-host — receiver, analyser and report generator for granite NDT\n\
         \n\
         USAGE:\n\
         \x20 granite-host listen [options]\n\
         \x20 granite-host analyze --wav FILE [options]\n\
         \x20 granite-host reference --wav FILE [options]\n\
         \x20 granite-host selftest\n\
         \n\
         LISTEN OPTIONS:\n\
         \x20 --bind ADDR        UDP bind address (default 0.0.0.0:5005)\n\
         \x20 --duration SEC     capture length after the first packet (default 10; 0 = idle mode)\n\
         \x20 --idle SEC         idle timeout in idle mode (default 2)\n\
         \x20 --channel N        channel to extract from interleaved packets (default 0)\n\
         \x20 --reference FILE   healthy profile for classification\n\
         \x20 --out DIR          output directory (default out/live)\n\
         \x20 --title TEXT       report title\n\
         \n\
         ANALYZE OPTIONS:\n\
         \x20 --wav FILE         input WAV (PCM 8/16/24/32 or float)\n\
         \x20 --reference FILE   healthy profile for classification\n\
         \x20 --out DIR          output directory (default out/analysis)\n\
         \n\
         REFERENCE OPTIONS:\n\
         \x20 --wav FILE         healthy-slab capture\n\
         \x20 --out FILE         profile path (default reference.txt)\n\
         \x20 --name TEXT        profile name\n\
         \x20 --length/--width/--thickness METRES, --material NAME\n\
         \n\
         OUTPUT:\n\
         \x20 report.html, capture.wav, modes.csv, features.csv, events.csv,\n\
         \x20 summary.txt and (when supplied) reference.txt."
    );
}
