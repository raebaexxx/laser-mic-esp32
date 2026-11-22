//! Minimal RIFF/WAVE reader and writer.
//!
//! The writer produces 32-bit float mono WAV, which preserves the full
//! dynamic range of the simulated and received captures.  The reader accepts
//! PCM 8/16/24/32-bit, IEEE float 32/64-bit, and WAVE_FORMAT_EXTENSIBLE, and
//! mixes any channel count down to mono.

use std::fs;
use std::io;
use std::path::Path;

use granite_protocol::read_i24_le;

#[derive(Clone, Debug)]
pub struct WavData {
    pub sample_rate_hz: f64,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub format_tag: u16,
    pub frames: usize,
    pub samples: Vec<f32>,
}

/// Read a WAV file and mix it to mono.
pub fn read_wav(path: &Path) -> io::Result<WavData> {
    let bytes = fs::read(path)?;
    parse_wav(&bytes)
}

/// Parse an in-memory WAV file.
pub fn parse_wav(bytes: &[u8]) -> io::Result<WavData> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid("not a RIFF/WAVE file"));
    }
    let mut offset = 12usize;
    let mut fmt: Option<FmtChunk> = None;
    let mut data: Option<&[u8]> = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body_start = offset + 8;
        let body_end = body_start.saturating_add(size);
        if body_end > bytes.len() {
            return Err(invalid("truncated WAV chunk"));
        }
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => fmt = Some(parse_fmt(body)?),
            b"data" => data = Some(body),
            _ => {}
        }
        offset = body_end + (size & 1);
    }
    let fmt = fmt.ok_or_else(|| invalid("missing fmt chunk"))?;
    let data = data.ok_or_else(|| invalid("missing data chunk"))?;
    let block_align = if fmt.block_align > 0 {
        fmt.block_align as usize
    } else {
        fmt.channels as usize * (fmt.bits_per_sample as usize / 8)
    };
    if block_align == 0 {
        return Err(invalid("invalid block alignment"));
    }
    let frames = data.len() / block_align;
    if frames == 0 {
        return Ok(WavData {
            sample_rate_hz: fmt.sample_rate as f64,
            channels: fmt.channels,
            bits_per_sample: fmt.bits_per_sample,
            format_tag: fmt.format_tag,
            frames: 0,
            samples: Vec::new(),
        });
    }
    let channels = fmt.channels.max(1) as usize;
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * block_align;
        let mut sum = 0.0f64;
        for channel in 0..channels {
            let start = base + channel * (fmt.bits_per_sample as usize / 8);
            let value = decode_sample(&data[start..], fmt.format_tag, fmt.bits_per_sample)?;
            sum += value as f64;
        }
        samples.push((sum / channels as f64) as f32);
    }
    Ok(WavData {
        sample_rate_hz: fmt.sample_rate as f64,
        channels: fmt.channels,
        bits_per_sample: fmt.bits_per_sample,
        format_tag: fmt.format_tag,
        frames,
        samples,
    })
}

#[derive(Clone, Copy, Debug)]
struct FmtChunk {
    format_tag: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
}

fn parse_fmt(body: &[u8]) -> io::Result<FmtChunk> {
    if body.len() < 16 {
        return Err(invalid("fmt chunk too short"));
    }
    let mut format_tag = u16::from_le_bytes([body[0], body[1]]);
    let channels = u16::from_le_bytes([body[2], body[3]]);
    let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let block_align = u16::from_le_bytes([body[12], body[13]]);
    let bits_per_sample = u16::from_le_bytes([body[14], body[15]]);
    if format_tag == 0xfffe {
        if body.len() < 26 {
            return Err(invalid("extensible fmt chunk too short"));
        }
        format_tag = u16::from_le_bytes([body[24], body[25]]);
    }
    if channels == 0 {
        return Err(invalid("zero channels"));
    }
    if sample_rate == 0 {
        return Err(invalid("zero sample rate"));
    }
    Ok(FmtChunk {
        format_tag,
        channels,
        sample_rate,
        block_align,
        bits_per_sample,
    })
}

fn decode_sample(bytes: &[u8], format_tag: u16, bits: u16) -> io::Result<f32> {
    match (format_tag, bits) {
        (1, 8) => Ok((bytes[0] as f32 - 128.0) / 128.0),
        (1, 16) => {
            let v = i16::from_le_bytes([bytes[0], bytes[1]]);
            Ok(v as f32 / 32_768.0)
        }
        (1, 24) => Ok(read_i24_le(bytes) as f32 / 8_388_608.0),
        (1, 32) => {
            let v = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Ok(v as f32 / 2_147_483_648.0)
        }
        (3, 32) => Ok(f32::from_bits(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ]))),
        (3, 64) => {
            let v = f64::from_bits(u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]));
            Ok(v as f32)
        }
        _ => Err(invalid("unsupported WAV sample format")),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Write a mono 32-bit float WAV file.
pub fn write_wav_f32(path: &Path, sample_rate_hz: f64, samples: &[f32]) -> io::Result<()> {
    let sample_rate = sample_rate_hz.round().max(1.0) as u32;
    let data_len = samples.len() * 4;
    let riff_len = 36u32.saturating_add(data_len as u32);
    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_len.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        bytes.extend_from_slice(&clamped.to_bits().to_le_bytes());
    }
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "granite-{}-{}-{}.wav",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn float_roundtrip() {
        let path = temp_path("float");
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin() * 0.7).collect();
        write_wav_f32(&path, 48_000.0, &samples).unwrap();
        let wav = read_wav(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(wav.sample_rate_hz, 48_000.0);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.frames, samples.len());
        for (a, b) in samples.iter().zip(wav.samples.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }

    #[test]
    fn pcm16_stereo_is_mixed_to_mono() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        let data: Vec<u8> = [1i16, 3, -1, 1]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        bytes.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        let wav = parse_wav(&bytes).unwrap();
        assert_eq!(wav.frames, 2);
        assert!((wav.samples[0] - 2.0 / 32_768.0).abs() < 1e-7);
        assert!((wav.samples[1] - 0.0).abs() < 1e-7);
    }
}
