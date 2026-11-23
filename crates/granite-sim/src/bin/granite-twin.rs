//! `granite-twin`: command-line digital twin.

use std::path::PathBuf;
use std::process::ExitCode;

use granite_core::classify::Condition;
use granite_sim::lab::write_lab;
use granite_sim::scenario::{catalog, find, run_scenario};
use granite_sim::suite::{format_table, is_match, run_suite, write_bundle, SuiteOptions};
use granite_sim::visual::write_scope;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("granite-twin: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("help");
    match command {
        "help" | "--help" | "-h" => {
            usage();
            Ok(ExitCode::SUCCESS)
        }
        "list" => {
            for scenario in catalog() {
                println!(
                    "{:<16} {:<10} {}",
                    scenario.id,
                    scenario.expected.label(),
                    scenario.title
                );
                println!("  {}", scenario.description);
            }
            Ok(ExitCode::SUCCESS)
        }
        "run" => {
            let id = flag(&args, "--scenario").unwrap_or_else(|| "healthy".to_string());
            let seed = flag(&args, "--seed")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0x5eed_1234);
            let scenario = find(&id).ok_or_else(|| format!("unknown scenario '{id}'"))?;
            let output = flag(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(&scenario.id));
            let run = run_scenario(&scenario, seed).map_err(|error| error.to_string())?;
            let bundle = write_bundle(&run, &output).map_err(|error| error.to_string())?;
            print_run(&run);
            println!("report: {}", bundle.report_html.display());
            println!("scope:  {}", bundle.scope_html.display());
            println!("wav:    {}", bundle.capture_wav.display());
            println!("index:  {}", output.display());
            Ok(ExitCode::SUCCESS)
        }
        "scope" => {
            let id = flag(&args, "--scenario").unwrap_or_else(|| "crack".to_string());
            let seed = flag(&args, "--seed")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0x5eed_1234);
            let scenario = find(&id).ok_or_else(|| format!("unknown scenario '{id}'"))?;
            let output = flag(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out").join(format!("scope-{id}.html")));
            if let Some(parent) = output.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
            }
            let run = run_scenario(&scenario, seed).map_err(|error| error.to_string())?;
            write_scope(&run, &output).map_err(|error| error.to_string())?;
            print_run(&run);
            println!("scope:  {}", output.display());
            Ok(ExitCode::SUCCESS)
        }
        "lab" => {
            let output = flag(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out/lab.html"));
            let seed = flag(&args, "--seed")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0x5eed_1234);
            let mut runs = Vec::new();
            for scenario in catalog() {
                runs.push(run_scenario(&scenario, seed).map_err(|error| error.to_string())?);
            }
            write_lab(&runs, &output).map_err(|error| error.to_string())?;
            println!(
                "interactive 3D lab: {} ({} scenarios)",
                output.display(),
                runs.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        "suite" => {
            let output = flag(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("out/suite"));
            let seed = flag(&args, "--seed")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0x5eed_1234);
            let ids = flag(&args, "--only")
                .map(|value| {
                    value
                        .split(',')
                        .map(|part| part.trim().to_string())
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let options = SuiteOptions {
                output_dir: output,
                seed,
                scenario_ids: ids,
            };
            let result = run_suite(&options, |run, bundle| {
                let status = if is_match(
                    run.scenario.expected.label(),
                    run.assessment.condition.label(),
                ) {
                    "PASS"
                } else {
                    "FAIL"
                };
                println!(
                    "[{status}] {:<16} expected {:<8} predicted {:<8} score {:>5.1}  -> {}",
                    run.scenario.id,
                    run.scenario.expected.label(),
                    run.assessment.condition.label(),
                    run.assessment.anomaly_score,
                    bundle.report_html.display()
                );
            })
            .map_err(|error| error.to_string())?;
            println!();
            print!("{}", format_table(&result.entries));
            println!();
            println!(
                "{}/{} scenarios matched; index: {}",
                result.matched,
                result.entries.len(),
                result.index_path.display()
            );
            println!("interactive showcase: {}", result.showcase_path.display());
            println!("interactive 3D lab:  {}", result.lab_path.display());
            if result.matched != result.entries.len() {
                println!(
                    "warning: {} scenario(s) did not match the expected label",
                    result.entries.len() - result.matched
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        "selftest" => {
            granite_core::selftest().map_err(|error| format!("core self-test: {error}"))?;
            println!("core self-test: ok");
            let healthy = run_scenario(&find("healthy").ok_or("missing healthy scenario")?, 1234)
                .map_err(|error| error.to_string())?;
            if healthy.analysis.modes.len() < 3 {
                return Err(format!(
                    "healthy scenario found only {} modes",
                    healthy.analysis.modes.len()
                ));
            }
            let crack = run_scenario(&find("crack").ok_or("missing crack scenario")?, 1234)
                .map_err(|error| error.to_string())?;
            if crack.assessment.condition == Condition::Healthy {
                return Err("crack scenario was classified as healthy".to_string());
            }
            println!(
                "healthy: {} modes, f1 {:.1} Hz, score {:.1}",
                healthy.analysis.modes.len(),
                healthy.analysis.features.fundamental_hz,
                healthy.assessment.anomaly_score
            );
            println!(
                "crack:   {} modes, f1 {:.1} Hz, score {:.1} ({})",
                crack.analysis.modes.len(),
                crack.analysis.features.fundamental_hz,
                crack.assessment.anomaly_score,
                crack.assessment.condition
            );
            println!("digital-twin self-test: ok");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command '{other}' (try --help)")),
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn print_run(run: &granite_sim::ScenarioRun) {
    println!("scenario:  {} ({})", run.scenario.title, run.scenario.id);
    println!(
        "expected:  {} / predicted: {}",
        run.scenario.expected.label(),
        run.assessment.condition.label()
    );
    println!(
        "score:     {:.1} (confidence {:.0}%)",
        run.assessment.anomaly_score,
        run.assessment.confidence * 100.0
    );
    println!(
        "modes:     {} stable / {} impacts",
        run.analysis.modes.len(),
        run.analysis.events.len()
    );
    println!(
        "fundamental {:.1} Hz, mean damping {:.5}, T60 {:.2} s",
        run.analysis.features.fundamental_hz,
        run.analysis.features.mean_damping,
        run.analysis.features.decay_t60_s
    );
    println!(
        "quality:   usable={}, trigger SNR {:.1} dB, clipping {:.3}%, gaps {:.2}%",
        run.analysis.quality.usable,
        run.analysis.quality.snr_db,
        run.analysis.quality.clipping_fraction * 100.0,
        run.analysis.quality.gap_fraction * 100.0
    );
    for flag in &run.assessment.flags {
        println!("  - {flag}");
    }
}

fn usage() {
    println!(
        "granite-twin — physics-inspired digital twin for granite impact-modal NDT\n\
         \n\
         USAGE:\n\
         \x20 granite-twin list\n\
         \x20 granite-twin run [--scenario ID] [--seed N] [--out DIR]\n\
         \x20 granite-twin scope [--scenario ID] [--seed N] [--out FILE]\n\
         \x20 granite-twin suite [--out DIR] [--seed N] [--only id1,id2,...]\n\
         \x20 granite-twin lab [--out FILE] [--seed N]\n\
         \x20 granite-twin selftest\n\
         \n\
         SCENARIOS:\n\
         \x20 healthy, healthy-noisy, lossy-link, crack, delamination, void,\n\
         \x20 thickness-plus, overloaded\n\
         \n\
         OUTPUT:\n\
         \x20 Each run writes capture.wav, report.html, scope.html, modes.csv,\n\
         \x20 features.csv, events.csv, reference.txt and summary.txt.  `suite`\n\
         \x20 also writes index.html, showcase.html and the interactive 3D\n\
         \x20 lab.html.  Open lab.html to see the rig and click through the\n\
         \x20 slabs; scope.html is the single-capture instrument view."
    );
}
