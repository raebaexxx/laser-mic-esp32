#![forbid(unsafe_code)]
//! Core library for the granite-slab laser-vibrometer NDT system.
//!
//! The crate is deliberately dependency-free.  It contains the wire protocol
//! consumer, stream reassembly, DSP, modal analysis, classification, WAV/CSV
//! I/O and HTML reporting.  The same code is used by the real host receiver
//! and by the digital twin.

pub mod analysis;
pub mod capture;
pub mod classify;
pub mod dsp;
pub mod report;
pub mod rng;
pub mod stats;
pub mod stream;
pub mod time;
pub mod wav;

pub use analysis::{
    analyze_capture, AnalysisConfig, CaptureAnalysis, DampingMethod, EventAnalysis, FeatureSet,
    ModalFeature, ModeEstimate, QualityMetrics,
};
pub use capture::{Capture, SampleGap};
pub use classify::{
    build_reference, classify, Assessment, ClassifierConfig, Condition, ReferenceProfile,
    SlabDescriptor,
};
pub use report::{render_html, render_suite_html, GroundTruth, ReportData, SuiteEntry};
pub use stream::{PacketStats, StreamAssembler, StreamError};

/// Run a few fast numerical/protocol sanity checks.
pub fn selftest() -> Result<(), String> {
    let fs = 48_000.0;
    let n = 4096;
    let signal: Vec<f64> = (0..n)
        .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / fs).sin())
        .collect();
    let spectrum = dsp::spectrum::amplitude_spectrum(&signal, fs, dsp::WindowKind::Hann, n * 2);
    let (index, _) = spectrum.peak().ok_or("empty spectrum")?;
    let peak = spectrum.interpolated_frequency(index);
    if (peak - 1000.0).abs() > 1.0 {
        return Err(format!("FFT self-test failed: peak at {peak} Hz"));
    }
    if granite_protocol::crc32(b"123456789") != 0xcbf4_3926 {
        return Err("CRC32 self-test failed".to_string());
    }
    Ok(())
}
