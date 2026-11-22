//! Reassembly of UDP datagrams into a contiguous mono capture.
//!
//! Packets may arrive out of order, be duplicated, or be lost.  The assembler
//! sorts by sample-clock index, de-duplicates overlaps, zero-fills gaps and
//! records statistics for the report.

use std::collections::BTreeSet;
use std::fmt;

use granite_protocol::{PacketKind, PacketView, HEADER_LEN};

use crate::capture::{Capture, SampleGap};

/// Transport statistics accumulated while receiving a capture.
#[derive(Clone, Debug, Default)]
pub struct PacketStats {
    pub packets_received: u64,
    pub packets_lost: u64,
    pub packets_duplicate: u64,
    pub packets_out_of_order: u64,
    pub packets_invalid: u64,
    pub bytes_received: u64,
    pub sequence_span: u64,
    pub gap_samples: u64,
    pub gap_count: u64,
    pub max_jitter_ms: f64,
    pub first_sample_index: u64,
    pub last_sample_index: u64,
    pub device_id: u32,
    pub sample_rate_hz: f64,
    pub duration_s: f64,
    pub loss_fraction: f64,
}

impl PacketStats {
    pub fn packet_loss_fraction(&self) -> f64 {
        let unique_received = self.packets_received.saturating_sub(self.packets_duplicate);
        let total = unique_received + self.packets_lost;
        if total == 0 {
            0.0
        } else {
            self.packets_lost as f64 / total as f64
        }
    }
}

struct AssembledPacket {
    start: u64,
    samples: Vec<f32>,
    sequence: u32,
}

/// Reassembles packets from one device.
pub struct StreamAssembler {
    channel: usize,
    packets: Vec<AssembledPacket>,
    seen_sequences: BTreeSet<u32>,
    first_sequence: Option<u32>,
    max_sequence: Option<u32>,
    stats: PacketStats,
    sample_rate_hz: Option<f64>,
    channels: Option<u16>,
    device_id: Option<u32>,
    arrival_offset_s: Option<f64>,
    arrival_order: u64,
}

impl StreamAssembler {
    /// Create an assembler that extracts `channel` (0-based) from each packet.
    pub fn new(channel: usize) -> Self {
        Self {
            channel,
            packets: Vec::new(),
            seen_sequences: BTreeSet::new(),
            first_sequence: None,
            max_sequence: None,
            stats: PacketStats::default(),
            sample_rate_hz: None,
            channels: None,
            device_id: None,
            arrival_offset_s: None,
            arrival_order: 0,
        }
    }

    pub fn stats(&self) -> &PacketStats {
        &self.stats
    }

    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    /// Add one decoded packet.  `arrival_time_s` is a host monotonic clock
    /// reading; it is used only for jitter statistics.
    pub fn push(&mut self, view: &PacketView<'_>, arrival_time_s: f64) -> Result<(), StreamError> {
        if view.header.kind != PacketKind::Data {
            return Err(StreamError::NotData);
        }
        let header = view.header;
        let fs = header.sample_rate_hz as f64;
        let channels = header.channels;

        match self.sample_rate_hz {
            Some(existing) if (existing - fs).abs() > 1e-6 => {
                return Err(StreamError::InconsistentSampleRate {
                    expected: existing,
                    actual: fs,
                })
            }
            None => self.sample_rate_hz = Some(fs),
            _ => {}
        }
        match self.channels {
            Some(existing) if existing != channels => {
                return Err(StreamError::InconsistentChannels {
                    expected: existing,
                    actual: channels,
                })
            }
            None => self.channels = Some(channels),
            _ => {}
        }
        if self.channel >= channels as usize {
            return Err(StreamError::ChannelOutOfRange {
                channel: self.channel,
                channels,
            });
        }
        match self.device_id {
            Some(existing) if existing != header.device_id => {
                return Err(StreamError::InconsistentDevice {
                    expected: existing,
                    actual: header.device_id,
                })
            }
            None => {
                self.device_id = Some(header.device_id);
                self.stats.device_id = header.device_id;
            }
            _ => {}
        }

        self.arrival_order += 1;
        self.stats.packets_received += 1;
        self.stats.bytes_received += (HEADER_LEN + view.payload.len()) as u64;
        self.stats.sample_rate_hz = fs;

        if self.seen_sequences.contains(&header.sequence) {
            self.stats.packets_duplicate += 1;
            return Ok(());
        }
        self.seen_sequences.insert(header.sequence);

        if self.first_sequence.is_none() {
            self.first_sequence = Some(header.sequence);
        }
        if let Some(max_seq) = self.max_sequence {
            if header.sequence < max_seq {
                self.stats.packets_out_of_order += 1;
            }
            self.max_sequence = Some(max_seq.max(header.sequence));
        } else {
            self.max_sequence = Some(header.sequence);
        }

        let start = header.first_sample_index;

        // Jitter relative to the device sample clock, removing the arbitrary
        // offset between device boot and host start.
        let expected_s = start as f64 / fs;
        let offset = *self
            .arrival_offset_s
            .get_or_insert(arrival_time_s - expected_s);
        let jitter_ms = ((arrival_time_s - expected_s) - offset).abs() * 1000.0;
        self.stats.max_jitter_ms = self.stats.max_jitter_ms.max(jitter_ms);

        let mut samples = vec![0.0f32; view.samples_per_channel()];
        view.decode_channel(self.channel, &mut samples)?;

        self.stats.first_sample_index = if self.stats.first_sample_index == 0 {
            start
        } else {
            self.stats.first_sample_index.min(start)
        };
        self.stats.last_sample_index = self
            .stats
            .last_sample_index
            .max(start.saturating_add(samples.len() as u64 - 1));

        self.packets.push(AssembledPacket {
            start,
            samples,
            sequence: header.sequence,
        });
        Ok(())
    }

    /// Sort, de-duplicate and concatenate the buffered packets.
    pub fn finish(mut self) -> Result<Capture, StreamError> {
        if self.packets.is_empty() {
            return Err(StreamError::NoPackets);
        }
        let fs = self.sample_rate_hz.ok_or(StreamError::NoPackets)?;
        self.packets
            .sort_by_key(|packet| (packet.start, packet.sequence));

        let stream_start = self.packets[0].start;
        let stream_end = self
            .packets
            .iter()
            .map(|packet| packet.start + packet.samples.len() as u64)
            .max()
            .unwrap_or(stream_start);
        let total = (stream_end - stream_start) as usize;
        if total > 200_000_000 {
            return Err(StreamError::TooLong { samples: total });
        }

        let mut samples = vec![0.0f32; total];
        let mut gaps = Vec::new();
        let mut cursor = stream_start;
        for packet in &self.packets {
            let packet_end = packet.start + packet.samples.len() as u64;
            if packet.start > cursor {
                gaps.push(SampleGap {
                    start_sample: cursor,
                    len: packet.start - cursor,
                });
                cursor = packet.start;
            }
            if packet_end <= cursor {
                continue;
            }
            let copy_start = cursor.max(packet.start);
            let skip = (copy_start - packet.start) as usize;
            let dest = (copy_start - stream_start) as usize;
            let len = packet.samples.len() - skip;
            samples[dest..dest + len].copy_from_slice(&packet.samples[skip..]);
            cursor = packet_end;
        }

        self.stats.gap_samples = gaps.iter().map(|gap| gap.len).sum();
        self.stats.gap_count = gaps.len() as u64;
        self.stats.duration_s = total as f64 / fs;
        self.stats.loss_fraction = if total == 0 {
            0.0
        } else {
            (self.stats.gap_samples as f64 / total as f64).clamp(0.0, 1.0)
        };
        self.stats.sequence_span = self
            .first_sequence
            .zip(self.max_sequence)
            .map(|(first, last)| (last - first) as u64 + 1)
            .unwrap_or(0);
        // Compute packet loss from the observed sequence span and the number of
        // unique sequences.  This is accurate even when a late packet arrives
        // after a gap was first noticed.
        self.stats.packets_lost = self
            .stats
            .sequence_span
            .saturating_sub(self.seen_sequences.len() as u64);

        Ok(Capture {
            sample_rate_hz: fs,
            samples,
            start_sample_index: stream_start,
            gaps,
            stats: Some(self.stats),
            source: "udp".to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub enum StreamError {
    NoPackets,
    NotData,
    InconsistentSampleRate { expected: f64, actual: f64 },
    InconsistentChannels { expected: u16, actual: u16 },
    InconsistentDevice { expected: u32, actual: u32 },
    ChannelOutOfRange { channel: usize, channels: u16 },
    TooLong { samples: usize },
    Protocol(granite_protocol::ProtocolError),
}

impl From<granite_protocol::ProtocolError> for StreamError {
    fn from(value: granite_protocol::ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPackets => write!(f, "no data packets received"),
            Self::NotData => write!(f, "packet is not a data packet"),
            Self::InconsistentSampleRate { expected, actual } => write!(
                f,
                "sample rate changed mid-stream: {expected} Hz -> {actual} Hz"
            ),
            Self::InconsistentChannels { expected, actual } => {
                write!(
                    f,
                    "channel count changed mid-stream: {expected} -> {actual}"
                )
            }
            Self::InconsistentDevice { expected, actual } => {
                write!(f, "device id changed mid-stream: {expected} -> {actual}")
            }
            Self::ChannelOutOfRange { channel, channels } => {
                write!(f, "channel {channel} requested but packet has {channels}")
            }
            Self::TooLong { samples } => write!(f, "capture too long: {samples} samples"),
            Self::Protocol(err) => write!(f, "protocol error: {err}"),
        }
    }
}

impl std::error::Error for StreamError {}

#[cfg(test)]
mod tests {
    use super::*;
    use granite_protocol::{encode_samples, PacketHeader, SampleFormat};

    fn make_packet(seq: u32, start: u64, samples: &[f32]) -> Vec<u8> {
        let header = PacketHeader::data(
            seq,
            7,
            start,
            start * 1_000_000_000 / 48_000,
            48_000,
            1,
            samples.len() as u16,
            SampleFormat::I24Packed,
            24,
        );
        let mut buf = vec![0u8; 2048];
        let n = encode_samples(&header, samples, &mut buf).unwrap();
        buf.truncate(n);
        buf
    }

    #[test]
    fn reassembles_out_of_order_with_gap() {
        let packets: Vec<Vec<u8>> = (0..10)
            .map(|i| {
                let samples: Vec<f32> = (0..16).map(|j| (i * 16 + j) as f32 / 1000.0).collect();
                make_packet(i as u32, (i * 16) as u64, &samples)
            })
            .collect();

        let order = [0usize, 2, 1, 4, 5, 7, 8, 9]; // packet 3 and 6 lost
        let mut assembler = StreamAssembler::new(0);
        for (arrival, index) in order.iter().enumerate() {
            let view = PacketView::decode(&packets[*index]).unwrap();
            assembler.push(&view, arrival as f64 * 0.001).unwrap();
        }
        let capture = assembler.finish().unwrap();
        assert_eq!(capture.samples.len(), 160);
        assert!(capture.samples[0].abs() < 1e-6);
        assert!((capture.samples[15] - 15.0 / 1000.0).abs() < 1e-6);
        assert!(capture.samples[48].abs() < 1e-6); // start of missing packet 3
        assert!((capture.samples[64] - 64.0 / 1000.0).abs() < 1e-6);
        assert!((capture.samples[80] - 80.0 / 1000.0).abs() < 1e-6);
        assert_eq!(capture.gaps.len(), 2);
        assert_eq!(capture.gaps[0].len, 16);
        assert_eq!(capture.gaps[1].len, 16);
        let stats = capture.stats.unwrap();
        assert!(stats.packets_lost >= 2);
        assert!(stats.packets_out_of_order >= 1);
    }

    #[test]
    fn duplicate_is_ignored() {
        let samples = [0.1f32; 8];
        let packet = make_packet(1, 0, &samples);
        let mut assembler = StreamAssembler::new(0);
        let view = PacketView::decode(&packet).unwrap();
        assembler.push(&view, 0.0).unwrap();
        assembler.push(&view, 0.001).unwrap();
        let capture = assembler.finish().unwrap();
        assert_eq!(capture.samples.len(), 8);
        assert_eq!(capture.stats.unwrap().packets_duplicate, 1);
    }

    #[test]
    fn empty_assembler_fails() {
        let assembler = StreamAssembler::new(0);
        assert!(matches!(assembler.finish(), Err(StreamError::NoPackets)));
    }
}
