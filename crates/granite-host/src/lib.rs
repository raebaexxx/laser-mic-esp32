#![forbid(unsafe_code)]
//! Host-side receiver, analysis and reporting for the granite NDT system.
//!
//! The library is deliberately small and dependency-free.  It accepts framed
//! UDP datagrams from the ESP32, reconstructs a capture, runs the shared
//! `granite-core` pipeline and writes a self-contained HTML report plus WAV and
//! CSV exports.

use std::fs;
use std::io;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use granite_core::analysis::{analyze_capture, AnalysisConfig, CaptureAnalysis};
use granite_core::capture::Capture;
use granite_core::classify::{
    build_reference, classify, Assessment, ClassifierConfig, ReferenceProfile, SlabDescriptor,
};
use granite_core::report::{
    render_html, write_events_csv, write_features_csv, write_modes_csv, ReportData,
};
use granite_core::stream::{PacketStats, StreamAssembler, StreamError};
use granite_core::time::{format_unix_utc, now_unix_s};
use granite_core::wav::{read_wav, write_wav_f32};
use granite_protocol::{PacketKind, PacketView};

/// Options for a live UDP capture.
#[derive(Clone, Debug)]
pub struct ReceiveOptions {
    pub bind_addr: String,
    /// Capture duration after the first data packet.  `None` means "until the
    /// link has been idle for `idle_timeout`".
    pub duration_s: Option<f64>,
    pub idle_timeout_s: f64,
    pub startup_timeout_s: f64,
    pub channel: usize,
}

impl Default for ReceiveOptions {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:5005".to_string(),
            duration_s: Some(10.0),
            idle_timeout_s: 2.0,
            startup_timeout_s: 20.0,
            channel: 0,
        }
    }
}

/// Result of a live receive session.
#[derive(Clone, Debug)]
pub struct ReceiveOutcome {
    pub capture: Capture,
    pub stats: PacketStats,
    pub invalid_packets: u64,
    pub status_messages: Vec<String>,
    pub elapsed_s: f64,
}

/// Receive framed UDP packets and reassemble them into a mono capture.
pub fn receive_udp(options: &ReceiveOptions) -> io::Result<ReceiveOutcome> {
    let socket = UdpSocket::bind(&options.bind_addr)?;
    receive_on_socket(socket, options)
}

/// Receive on an already-bound socket.  Useful for tests and for embedding the
/// receiver in a larger application that owns its socket.
pub fn receive_on_socket(
    socket: UdpSocket,
    options: &ReceiveOptions,
) -> io::Result<ReceiveOutcome> {
    socket.set_read_timeout(Some(Duration::from_millis(200)))?;
    let mut buffer = vec![0u8; 65_535];
    let mut assembler = StreamAssembler::new(options.channel);
    let mut invalid_packets = 0u64;
    let mut status_messages = Vec::new();
    let started = Instant::now();
    let mut first_packet: Option<Instant> = None;
    let mut last_data: Option<Instant> = None;

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((len, peer)) => match PacketView::decode(&buffer[..len]) {
                Ok(view) => match view.header.kind {
                    PacketKind::Data => {
                        if first_packet.is_none() {
                            first_packet = Some(Instant::now());
                            eprintln!(
                                "receiving from {peer}: {} Hz, {} channel(s), {} samples/packet",
                                view.header.sample_rate_hz,
                                view.header.channels,
                                view.header.samples_per_channel
                            );
                        }
                        last_data = Some(Instant::now());
                        assembler
                            .push(&view, started.elapsed().as_secs_f64())
                            .map_err(stream_to_io)?;
                    }
                    PacketKind::Status | PacketKind::Hello => {
                        let text = String::from_utf8_lossy(view.payload).to_string();
                        eprintln!("device: {text}");
                        status_messages.push(text);
                    }
                    PacketKind::Ack => {}
                },
                Err(error) => {
                    invalid_packets += 1;
                    eprintln!("dropped invalid datagram ({len} bytes): {error}");
                }
            },
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error),
        }

        let now = Instant::now();
        if let Some(first) = first_packet {
            if let Some(duration) = options.duration_s {
                if now.duration_since(first).as_secs_f64() >= duration {
                    break;
                }
            } else if let Some(last) = last_data {
                if now.duration_since(last).as_secs_f64() >= options.idle_timeout_s {
                    break;
                }
            }
        } else if now.duration_since(started).as_secs_f64() >= options.startup_timeout_s {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "no data packets received on {} within {:.0} s",
                    options.bind_addr, options.startup_timeout_s
                ),
            ));
        }
    }

    let mut capture = assembler.finish().map_err(stream_to_io)?;
    // `finish` fills in loss/gap/duration statistics, so read them back from
    // the capture rather than from the assembler's pre-finish snapshot.
    let stats = capture.stats.clone().unwrap_or_default();
    capture.source = format!(
        "udp {} (device 0x{:08x})",
        options.bind_addr, stats.device_id
    );
    Ok(ReceiveOutcome {
        capture,
        stats,
        invalid_packets,
        status_messages,
        elapsed_s: started.elapsed().as_secs_f64(),
    })
}

fn stream_to_io(error: StreamError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// Load a WAV file as a mono capture.
pub fn load_wav(path: &Path) -> io::Result<Capture> {
    let wav = read_wav(path)?;
    Ok(Capture::new(
        wav.sample_rate_hz,
        wav.samples,
        path.display().to_string(),
    ))
}

/// Analyse a capture with the shared pipeline.
pub fn analyse(capture: &Capture, config: &AnalysisConfig) -> CaptureAnalysis {
    analyze_capture(capture, config)
}

/// Build a healthy reference profile from a capture.
pub fn reference_from_capture(
    name: impl Into<String>,
    capture: &Capture,
    config: &AnalysisConfig,
    slab: Option<SlabDescriptor>,
) -> Result<ReferenceProfile, String> {
    let analysis = analyze_capture(capture, config);
    build_reference(name, &analysis, slab)
}

/// Files written for one host analysis.
#[derive(Clone, Debug)]
pub struct HostBundle {
    pub directory: PathBuf,
    pub report_html: PathBuf,
    pub capture_wav: PathBuf,
    pub modes_csv: PathBuf,
    pub features_csv: PathBuf,
    pub events_csv: PathBuf,
    pub summary: PathBuf,
    pub reference: Option<PathBuf>,
}

/// Write a complete, self-contained analysis bundle.
#[allow(clippy::too_many_arguments)]
pub fn write_bundle(
    directory: &Path,
    title: &str,
    subtitle: &str,
    capture: &Capture,
    analysis: &CaptureAnalysis,
    assessment: Option<&Assessment>,
    reference: Option<&ReferenceProfile>,
    notes: &[String],
) -> io::Result<HostBundle> {
    fs::create_dir_all(directory)?;
    let report_html = directory.join("report.html");
    let capture_wav = directory.join("capture.wav");
    let modes_csv = directory.join("modes.csv");
    let features_csv = directory.join("features.csv");
    let events_csv = directory.join("events.csv");
    let summary = directory.join("summary.txt");

    write_wav_f32(&capture_wav, capture.sample_rate_hz, &capture.samples)?;
    write_modes_csv(&modes_csv, analysis)?;
    write_features_csv(&features_csv, analysis)?;
    write_events_csv(&events_csv, analysis)?;

    let mut summary_text = String::new();
    summary_text.push_str(&format!("title={title}\n"));
    summary_text.push_str(&format!("source={}\n", capture.source));
    summary_text.push_str(&format!(
        "sample_rate_hz={:.1}\nduration_s={:.3}\n",
        capture.sample_rate_hz,
        capture.duration_s()
    ));
    if let Some(assessment) = assessment {
        summary_text.push_str(&format!("condition={}\n", assessment.condition.label()));
        summary_text.push_str(&format!(
            "anomaly_score={:.2}\nconfidence={:.3}\n",
            assessment.anomaly_score, assessment.confidence
        ));
        for flag in &assessment.flags {
            summary_text.push_str(&format!("flag={flag}\n"));
        }
    }
    summary_text.push_str(&format!(
        "fundamental_hz={:.3}\nmean_damping={:.6}\nmodes={}\nevents={}\nquality_usable={}\n",
        analysis.features.fundamental_hz,
        analysis.features.mean_damping,
        analysis.modes.len(),
        analysis.events.len(),
        analysis.quality.usable
    ));
    for warning in &analysis.quality.warnings {
        summary_text.push_str(&format!("warning={warning}\n"));
    }
    fs::write(&summary, summary_text)?;

    let reference_path = if let Some(reference) = reference {
        let path = directory.join("reference.txt");
        reference.save(&path)?;
        Some(path)
    } else {
        None
    };

    let data = ReportData {
        title: title.to_string(),
        subtitle: subtitle.to_string(),
        generated_at: format_unix_utc(now_unix_s()),
        capture: capture.clone(),
        analysis: analysis.clone(),
        assessment: assessment.cloned(),
        reference: reference.cloned(),
        ground_truth: None,
        scenario: None,
        notes: notes.to_vec(),
    };
    fs::write(&report_html, render_html(&data))?;

    Ok(HostBundle {
        directory: directory.to_path_buf(),
        report_html,
        capture_wav,
        modes_csv,
        features_csv,
        events_csv,
        summary,
        reference: reference_path,
    })
}

/// Classify an analysis against a loaded reference.
pub fn assess(analysis: &CaptureAnalysis, reference: &ReferenceProfile) -> Assessment {
    classify(analysis, reference, &ClassifierConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use granite_protocol::{encode_samples, PacketHeader, SampleFormat};
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn receives_and_reassembles_udp() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
            let fs = 48_000u32;
            let per_packet = 256usize;
            for sequence in 0..8u32 {
                let start = sequence as usize * per_packet;
                let samples: Vec<f32> = (0..per_packet)
                    .map(|i| (((start + i) as f32) * 0.001).sin() * 0.5)
                    .collect();
                let header = PacketHeader::data(
                    sequence,
                    7,
                    start as u64,
                    (start as u64 * 1_000_000_000) / fs as u64,
                    fs,
                    1,
                    per_packet as u16,
                    SampleFormat::I24Packed,
                    24,
                );
                let mut buffer = vec![0u8; 2048];
                let written = encode_samples(&header, &samples, &mut buffer).unwrap();
                sender.send_to(&buffer[..written], address).unwrap();
                thread::sleep(StdDuration::from_millis(5));
            }
        });
        let options = ReceiveOptions {
            bind_addr: address.to_string(),
            duration_s: Some(0.2),
            idle_timeout_s: 0.5,
            startup_timeout_s: 2.0,
            channel: 0,
        };
        let outcome = receive_on_socket(socket, &options).unwrap();
        sender.join().unwrap();
        assert_eq!(outcome.capture.samples.len(), 8 * 256);
        assert_eq!(outcome.stats.packets_received, 8);
        assert!(outcome.stats.loss_fraction < 1e-9);
        assert_eq!(outcome.stats.gap_count, 0);
        assert!(outcome.stats.duration_s > 0.0);
    }
}
