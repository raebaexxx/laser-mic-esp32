//! Packetisation and impaired-network simulation.
//!
//! Samples are encoded with the real `granite-protocol` codec, split into UDP
//! datagrams, then optionally lost, duplicated, corrupted, delayed or
//! reordered.  The receiver feeds the datagrams through the same
//! `StreamAssembler` used by the live host, so transport statistics and gap
//! handling are exercised end-to-end.

use granite_core::capture::Capture;
use granite_core::rng::Rng;
use granite_core::stream::{StreamAssembler, StreamError};
use granite_protocol::{
    encode_samples, PacketHeader, PacketView, SampleFormat, HEADER_LEN, MAX_SAMPLES_PER_CHANNEL,
};

/// One simulated UDP datagram and its host arrival time.
#[derive(Clone, Debug)]
pub struct Datagram {
    pub sequence: u32,
    pub arrival_s: f64,
    pub bytes: Vec<u8>,
}

/// Impairments applied to the simulated link.
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    pub packet_samples: usize,
    pub loss: f64,
    pub duplicate: f64,
    pub corrupt: f64,
    pub jitter_ms: f64,
    pub reorder: f64,
    pub base_latency_ms: f64,
    pub sample_format: SampleFormat,
    pub device_id: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            packet_samples: 256,
            loss: 0.0,
            duplicate: 0.0,
            corrupt: 0.0,
            jitter_ms: 0.0,
            reorder: 0.0,
            base_latency_ms: 10.0,
            sample_format: SampleFormat::I24Packed,
            device_id: 0x4752_4e54,
        }
    }
}

impl NetworkConfig {
    pub fn clean() -> Self {
        Self::default()
    }

    pub fn impaired(loss: f64, jitter_ms: f64) -> Self {
        Self {
            loss,
            jitter_ms,
            reorder: (loss * 4.0).min(0.15),
            ..Default::default()
        }
    }
}

/// Split a capture into protocol datagrams and apply link impairments.
pub fn transmit(
    samples: &[f32],
    sample_rate_hz: f64,
    config: &NetworkConfig,
    rng: &mut Rng,
) -> Vec<Datagram> {
    let packet_samples = config
        .packet_samples
        .clamp(1, MAX_SAMPLES_PER_CHANNEL)
        .min(samples.len().max(1));
    let fs = sample_rate_hz.max(1.0);
    let packet_duration = packet_samples as f64 / fs;
    let mut datagrams = Vec::new();
    let mut sequence = 0u32;
    let mut start = 0usize;
    let mut buffer = vec![0u8; HEADER_LEN + packet_samples * 4 + 16];
    while start < samples.len() {
        let end = (start + packet_samples).min(samples.len());
        let chunk = &samples[start..end];
        let header = PacketHeader::data(
            sequence,
            config.device_id,
            start as u64,
            (start as f64 / fs * 1.0e9) as u64,
            fs.round() as u32,
            1,
            chunk.len() as u16,
            config.sample_format,
            match config.sample_format {
                SampleFormat::I16 => 16,
                SampleFormat::I24Packed => 24,
                SampleFormat::I32 => 32,
                SampleFormat::F32 => 32,
            },
        );
        let written = match encode_samples(&header, chunk, &mut buffer) {
            Ok(written) => written,
            Err(_) => break,
        };
        let mut bytes = buffer[..written].to_vec();
        let nominal = config.base_latency_ms / 1000.0 + start as f64 / fs;
        let mut jitter = 0.0;
        if config.jitter_ms > 0.0 {
            jitter = rng.normal() * config.jitter_ms / 1000.0;
            jitter = jitter.clamp(
                -3.0 * config.jitter_ms / 1000.0,
                3.0 * config.jitter_ms / 1000.0,
            );
        }
        let mut arrival = nominal + jitter;
        if config.reorder > 0.0 && rng.chance(config.reorder) {
            arrival += packet_duration * rng.range(1.0, 3.0);
        }
        if config.corrupt > 0.0 && rng.chance(config.corrupt) && bytes.len() > HEADER_LEN {
            let index = HEADER_LEN + rng.index(bytes.len() - HEADER_LEN);
            bytes[index] ^= 0x5a;
        }
        let dropped = config.loss > 0.0 && rng.chance(config.loss);
        if !dropped {
            datagrams.push(Datagram {
                sequence,
                arrival_s: arrival,
                bytes: bytes.clone(),
            });
            if config.duplicate > 0.0 && rng.chance(config.duplicate) {
                datagrams.push(Datagram {
                    sequence,
                    arrival_s: arrival + 0.0004,
                    bytes,
                });
            }
        }
        sequence += 1;
        start = end;
    }
    datagrams.sort_by(|a, b| {
        a.arrival_s
            .partial_cmp(&b.arrival_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    datagrams
}

/// Result of feeding datagrams through the production reassembler.
#[derive(Clone, Debug)]
pub struct ReceiveResult {
    pub capture: Capture,
    pub invalid_packets: u64,
    pub datagrams: usize,
}

/// Decode and reassemble a list of datagrams.
pub fn receive(datagrams: &[Datagram], channel: usize) -> Result<ReceiveResult, StreamError> {
    let mut assembler = StreamAssembler::new(channel);
    let mut invalid_packets = 0u64;
    for datagram in datagrams {
        match PacketView::decode(&datagram.bytes) {
            Ok(view) => {
                assembler.push(&view, datagram.arrival_s)?;
            }
            Err(_) => invalid_packets += 1,
        }
    }
    let mut capture = assembler.finish()?;
    capture.source = "digital-twin UDP".to_string();
    Ok(ReceiveResult {
        capture,
        invalid_packets,
        datagrams: datagrams.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_link_roundtrips_samples() {
        let fs = 48_000.0;
        let samples: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        let mut rng = Rng::new(7);
        let datagrams = transmit(&samples, fs, &NetworkConfig::clean(), &mut rng);
        let result = receive(&datagrams, 0).unwrap();
        assert_eq!(result.capture.samples.len(), samples.len());
        assert_eq!(result.invalid_packets, 0);
        for (a, b) in result.capture.samples.iter().zip(samples.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn loss_and_reorder_create_gaps_and_stats() {
        let fs = 48_000.0;
        let samples: Vec<f32> = (0..8000).map(|i| (i as f32 * 0.001).sin()).collect();
        let mut rng = Rng::new(11);
        let config = NetworkConfig {
            loss: 0.2,
            duplicate: 0.1,
            jitter_ms: 2.0,
            reorder: 0.2,
            ..Default::default()
        };
        let datagrams = transmit(&samples, fs, &config, &mut rng);
        let result = receive(&datagrams, 0).unwrap();
        let stats = result.capture.stats.unwrap();
        assert!(stats.packets_lost > 0 || !result.capture.gaps.is_empty());
        assert!(stats.max_jitter_ms >= 0.0);
    }

    #[test]
    fn corrupt_packets_are_counted() {
        let fs = 48_000.0;
        let samples: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut rng = Rng::new(3);
        let config = NetworkConfig {
            corrupt: 0.5,
            ..Default::default()
        };
        let datagrams = transmit(&samples, fs, &config, &mut rng);
        let result = receive(&datagrams, 0).unwrap();
        assert!(result.invalid_packets > 0);
    }
}
