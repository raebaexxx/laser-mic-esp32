//! Wire protocol shared by the ESP32 firmware and the host receiver.
//!
//! The protocol is deliberately small, fixed-layout and `no_std` friendly so
//! that the exact same encoder/decoder runs on the microcontroller and on the
//! PC.  Every datagram is self-describing: it carries a sample-clock index, a
//! device timestamp, the sample rate, the channel count, the sample format and
//! a CRC32 over the header and payload.
//!
//! See `docs/protocol.md` for the byte-level specification.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
#[macro_use]
extern crate std;

use core::fmt;

#[cfg(test)]
use std::vec::Vec;

/// Datagram magic: ASCII `GRNT`.
pub const MAGIC: [u8; 4] = *b"GRNT";

/// Current protocol version.
pub const VERSION: u8 = 1;

/// Fixed header size in bytes.
pub const HEADER_LEN: usize = 52;

/// Offset of the CRC32 field inside the header.
pub const CRC_OFFSET: usize = HEADER_LEN - 4;

/// Largest payload we put in a single UDP datagram.  Kept below the typical
/// 1500-byte Ethernet MTU once IP/UDP headers are accounted for.
pub const MAX_PAYLOAD_LEN: usize = 1400;

/// Largest number of samples per channel in one datagram.
pub const MAX_SAMPLES_PER_CHANNEL: usize = 512;

/// Largest supported channel count.
pub const MAX_CHANNELS: u16 = 8;

/// Packet kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PacketKind {
    /// Raw interleaved PCM samples.
    Data = 1,
    /// UTF-8 status / log message (no samples).
    Status = 2,
    /// Device announcement (UTF-8 JSON-like text).
    Hello = 3,
    /// Optional acknowledgement (unused by the current firmware).
    Ack = 4,
}

impl TryFrom<u8> for PacketKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Data),
            2 => Ok(Self::Status),
            3 => Ok(Self::Hello),
            4 => Ok(Self::Ack),
            other => Err(ProtocolError::UnsupportedKind(other)),
        }
    }
}

/// Sample encoding used in the payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SampleFormat {
    /// Signed 16-bit little-endian PCM.
    I16 = 1,
    /// Signed 24-bit little-endian PCM, packed in three bytes.
    I24Packed = 2,
    /// Signed 32-bit little-endian PCM.
    I32 = 3,
    /// IEEE-754 `f32` little-endian.  Useful for simulation and debugging.
    F32 = 4,
}

impl SampleFormat {
    /// Bytes occupied by one sample of this format.
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::I24Packed => 3,
            Self::I32 | Self::F32 => 4,
        }
    }

    /// Decode one sample to a normalised `f32` in roughly `[-1.0, 1.0]`.
    pub fn decode(self, bytes: &[u8]) -> Option<f32> {
        match self {
            Self::I16 => {
                if bytes.len() < 2 {
                    return None;
                }
                let v = i16::from_le_bytes([bytes[0], bytes[1]]);
                Some(v as f32 / 32_768.0)
            }
            Self::I24Packed => {
                if bytes.len() < 3 {
                    return None;
                }
                let v = read_i24_le(bytes);
                Some(v as f32 / 8_388_608.0)
            }
            Self::I32 => {
                if bytes.len() < 4 {
                    return None;
                }
                let v = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                Some(v as f32 / 2_147_483_648.0)
            }
            Self::F32 => {
                if bytes.len() < 4 {
                    return None;
                }
                Some(f32::from_bits(u32::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ])))
            }
        }
    }

    /// Encode one normalised sample into `out`.
    ///
    /// Values outside `[-1.0, 1.0]` are clamped.  Integer formats round to the
    /// nearest representable value.
    pub fn encode(self, value: f32, out: &mut [u8]) -> Option<()> {
        let x = clamp_unit(value);
        match self {
            Self::I16 => {
                if out.len() < 2 {
                    return None;
                }
                let v = quantize_i32(x, 32_767.0) as i16;
                out[..2].copy_from_slice(&v.to_le_bytes());
            }
            Self::I24Packed => {
                if out.len() < 3 {
                    return None;
                }
                let v = quantize_i32(x, 8_388_607.0);
                write_i24_le(v, out);
            }
            Self::I32 => {
                if out.len() < 4 {
                    return None;
                }
                // f32 has only 24 bits of mantissa; this is the best we can do
                // without accepting a wider input type.
                let v = quantize_i32(x, 2_147_483_520.0);
                out[..4].copy_from_slice(&v.to_le_bytes());
            }
            Self::F32 => {
                if out.len() < 4 {
                    return None;
                }
                out[..4].copy_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        Some(())
    }
}

impl TryFrom<u8> for SampleFormat {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::I16),
            2 => Ok(Self::I24Packed),
            3 => Ok(Self::I32),
            4 => Ok(Self::F32),
            other => Err(ProtocolError::UnsupportedFormat(other)),
        }
    }
}

#[inline]
fn clamp_unit(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

#[inline]
fn quantize_i32(value: f32, scale: f32) -> i32 {
    // No `f32::round` in `core`; this is equivalent for our purposes.
    if value >= 0.0 {
        (value * scale + 0.5) as i32
    } else {
        (value * scale - 0.5) as i32
    }
}

/// Read a signed 24-bit little-endian integer, sign-extended to `i32`.
#[inline]
pub fn read_i24_le(bytes: &[u8]) -> i32 {
    let raw = (bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
    (raw << 8) >> 8
}

/// Write the low 24 bits of `value` as little-endian bytes.
#[inline]
pub fn write_i24_le(value: i32, out: &mut [u8]) {
    out[0] = (value & 0xff) as u8;
    out[1] = ((value >> 8) & 0xff) as u8;
    out[2] = ((value >> 16) & 0xff) as u8;
}

/// Fixed packet header.  All multi-byte fields are little-endian.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketHeader {
    pub version: u8,
    pub kind: PacketKind,
    pub flags: u16,
    pub sequence: u32,
    pub device_id: u32,
    /// Index of the first sample in the device's sample clock.
    pub first_sample_index: u64,
    /// Device monotonic timestamp of the first sample, in nanoseconds.
    pub timestamp_ns: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub samples_per_channel: u16,
    pub sample_format: SampleFormat,
    pub bits_per_sample: u8,
    pub payload_len: u16,
}

impl PacketHeader {
    /// Build a data-packet header.
    #[allow(clippy::too_many_arguments)]
    pub const fn data(
        sequence: u32,
        device_id: u32,
        first_sample_index: u64,
        timestamp_ns: u64,
        sample_rate_hz: u32,
        channels: u16,
        samples_per_channel: u16,
        sample_format: SampleFormat,
        bits_per_sample: u8,
    ) -> Self {
        let payload_len =
            samples_per_channel as usize * channels as usize * sample_format.bytes_per_sample();
        Self {
            version: VERSION,
            kind: PacketKind::Data,
            flags: 0,
            sequence,
            device_id,
            first_sample_index,
            timestamp_ns,
            sample_rate_hz,
            channels,
            samples_per_channel,
            sample_format,
            bits_per_sample,
            payload_len: payload_len as u16,
        }
    }

    /// Build a status packet header.
    pub fn status(
        sequence: u32,
        device_id: u32,
        timestamp_ns: u64,
        sample_rate_hz: u32,
        text: &str,
    ) -> Self {
        Self {
            version: VERSION,
            kind: PacketKind::Status,
            flags: 0,
            sequence,
            device_id,
            first_sample_index: 0,
            timestamp_ns,
            sample_rate_hz,
            channels: 0,
            samples_per_channel: 0,
            sample_format: SampleFormat::F32,
            bits_per_sample: 0,
            payload_len: text.len().min(MAX_PAYLOAD_LEN) as u16,
        }
    }

    /// Check internal consistency.  Does not verify the CRC.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.version));
        }
        if self.payload_len as usize > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::InvalidField(
                "payload_len exceeds MTU budget",
            ));
        }
        if self.kind == PacketKind::Data {
            if self.channels == 0 || self.channels > MAX_CHANNELS {
                return Err(ProtocolError::InvalidField("channels"));
            }
            if self.samples_per_channel == 0
                || self.samples_per_channel as usize > MAX_SAMPLES_PER_CHANNEL
            {
                return Err(ProtocolError::InvalidField("samples_per_channel"));
            }
            if self.sample_rate_hz == 0 || self.sample_rate_hz > 1_000_000 {
                return Err(ProtocolError::InvalidField("sample_rate_hz"));
            }
            if self.bits_per_sample == 0 || self.bits_per_sample > 32 {
                return Err(ProtocolError::InvalidField("bits_per_sample"));
            }
            let expected = self.samples_per_channel as usize
                * self.channels as usize
                * self.sample_format.bytes_per_sample();
            if expected != self.payload_len as usize {
                return Err(ProtocolError::BadPayloadLen {
                    expected,
                    actual: self.payload_len as usize,
                });
            }
        }
        Ok(())
    }

    fn write_into(&self, out: &mut [u8]) {
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = self.version;
        out[5] = self.kind as u8;
        out[6..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..10].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        out[10..14].copy_from_slice(&self.sequence.to_le_bytes());
        out[14..18].copy_from_slice(&self.device_id.to_le_bytes());
        out[18..26].copy_from_slice(&self.first_sample_index.to_le_bytes());
        out[26..34].copy_from_slice(&self.timestamp_ns.to_le_bytes());
        out[34..38].copy_from_slice(&self.sample_rate_hz.to_le_bytes());
        out[38..40].copy_from_slice(&self.channels.to_le_bytes());
        out[40..42].copy_from_slice(&self.samples_per_channel.to_le_bytes());
        out[42] = self.sample_format as u8;
        out[43] = self.bits_per_sample;
        out[44..46].copy_from_slice(&self.payload_len.to_le_bytes());
        out[46..48].copy_from_slice(&0u16.to_le_bytes());
        out[48..52].copy_from_slice(&0u32.to_le_bytes());
    }

    fn read_from(buf: &[u8]) -> Result<Self, ProtocolError> {
        if buf.len() < HEADER_LEN {
            return Err(ProtocolError::TooShort {
                needed: HEADER_LEN,
                actual: buf.len(),
            });
        }
        if buf[0..4] != MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        let version = buf[4];
        if version != VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let header_len = u16::from_le_bytes([buf[8], buf[9]]) as usize;
        if header_len != HEADER_LEN {
            return Err(ProtocolError::BadHeaderLen(header_len as u16));
        }
        let kind = PacketKind::try_from(buf[5])?;
        let sample_format = SampleFormat::try_from(buf[42])?;
        Ok(Self {
            version,
            kind,
            flags: u16::from_le_bytes([buf[6], buf[7]]),
            sequence: u32::from_le_bytes([buf[10], buf[11], buf[12], buf[13]]),
            device_id: u32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]),
            first_sample_index: u64::from_le_bytes([
                buf[18], buf[19], buf[20], buf[21], buf[22], buf[23], buf[24], buf[25],
            ]),
            timestamp_ns: u64::from_le_bytes([
                buf[26], buf[27], buf[28], buf[29], buf[30], buf[31], buf[32], buf[33],
            ]),
            sample_rate_hz: u32::from_le_bytes([buf[34], buf[35], buf[36], buf[37]]),
            channels: u16::from_le_bytes([buf[38], buf[39]]),
            samples_per_channel: u16::from_le_bytes([buf[40], buf[41]]),
            sample_format,
            bits_per_sample: buf[43],
            payload_len: u16::from_le_bytes([buf[44], buf[45]]),
        })
    }
}

/// A decoded packet borrowing its payload from the receive buffer.
#[derive(Clone, Copy, Debug)]
pub struct PacketView<'a> {
    pub header: PacketHeader,
    pub payload: &'a [u8],
}

impl<'a> PacketView<'a> {
    /// Decode and validate one datagram.
    pub fn decode(buf: &'a [u8]) -> Result<Self, ProtocolError> {
        let header = PacketHeader::read_from(buf)?;
        let payload_len = header.payload_len as usize;
        if buf.len() != HEADER_LEN + payload_len {
            return Err(ProtocolError::BadPayloadLen {
                expected: HEADER_LEN + payload_len,
                actual: buf.len(),
            });
        }
        let payload = &buf[HEADER_LEN..];
        let expected_crc = crc32_join(&buf[..CRC_OFFSET], payload);
        let actual_crc = u32::from_le_bytes([buf[48], buf[49], buf[50], buf[51]]);
        if expected_crc != actual_crc {
            return Err(ProtocolError::CrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }
        header.validate()?;
        Ok(Self { header, payload })
    }

    /// Number of samples per channel.
    pub fn samples_per_channel(&self) -> usize {
        self.header.samples_per_channel as usize
    }

    /// Decode sample `index` of `channel`.
    pub fn sample(&self, channel: usize, index: usize) -> Option<f32> {
        if channel >= self.header.channels as usize
            || index >= self.header.samples_per_channel as usize
        {
            return None;
        }
        let bps = self.header.sample_format.bytes_per_sample();
        let offset = (index * self.header.channels as usize + channel) * bps;
        self.header
            .sample_format
            .decode(self.payload.get(offset..offset + bps)?)
    }

    /// Decode all samples of one channel into `out`.
    pub fn decode_channel(&self, channel: usize, out: &mut [f32]) -> Result<(), ProtocolError> {
        if channel >= self.header.channels as usize {
            return Err(ProtocolError::InvalidField("channel index"));
        }
        let n = self.samples_per_channel();
        if out.len() < n {
            return Err(ProtocolError::BufferTooSmall);
        }
        for (i, slot) in out.iter_mut().take(n).enumerate() {
            *slot = self
                .sample(channel, i)
                .ok_or(ProtocolError::BufferTooSmall)?;
        }
        Ok(())
    }
}

/// Encode a complete packet into `out`.  Returns the number of bytes written.
pub fn encode_packet(
    header: &PacketHeader,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, ProtocolError> {
    header.validate()?;
    if payload.len() != header.payload_len as usize {
        return Err(ProtocolError::BadPayloadLen {
            expected: header.payload_len as usize,
            actual: payload.len(),
        });
    }
    let total = HEADER_LEN + payload.len();
    if out.len() < total {
        return Err(ProtocolError::BufferTooSmall);
    }
    header.write_into(&mut out[..HEADER_LEN]);
    out[HEADER_LEN..total].copy_from_slice(payload);
    let crc = crc32_join(&out[..CRC_OFFSET], payload);
    out[CRC_OFFSET..HEADER_LEN].copy_from_slice(&crc.to_le_bytes());
    Ok(total)
}

/// Encode a data packet from normalised `f32` samples (one channel).
pub fn encode_samples(
    header: &PacketHeader,
    samples: &[f32],
    out: &mut [u8],
) -> Result<usize, ProtocolError> {
    if header.channels != 1 {
        return Err(ProtocolError::InvalidField(
            "encode_samples expects one channel",
        ));
    }
    if samples.len() != header.samples_per_channel as usize {
        return Err(ProtocolError::BadPayloadLen {
            expected: header.samples_per_channel as usize,
            actual: samples.len(),
        });
    }
    let bps = header.sample_format.bytes_per_sample();
    let payload_len = samples.len() * bps;
    if payload_len > MAX_PAYLOAD_LEN || out.len() < HEADER_LEN + payload_len {
        return Err(ProtocolError::BufferTooSmall);
    }
    // Encode into a temporary payload so that `encode_packet` never has to
    // copy from a region that overlaps its destination.
    let mut payload = [0u8; MAX_PAYLOAD_LEN];
    for (i, sample) in samples.iter().enumerate() {
        header
            .sample_format
            .encode(*sample, &mut payload[i * bps..(i + 1) * bps])
            .ok_or(ProtocolError::BufferTooSmall)?;
    }
    encode_packet(header, &payload[..payload_len], out)
}

/// Standard CRC-32 (IEEE 802.3), reflected polynomial `0xEDB88320`.
pub fn crc32(data: &[u8]) -> u32 {
    !crc32_update(0xffff_ffff, data)
}

/// Continue a CRC-32 calculation.  `crc` is the un-inverted running value.
pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc
}

/// CRC over two byte slices without allocating.
pub fn crc32_join(first: &[u8], second: &[u8]) -> u32 {
    let crc = crc32_update(0xffff_ffff, first);
    !crc32_update(crc, second)
}

/// Protocol errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    TooShort { needed: usize, actual: usize },
    BadMagic,
    UnsupportedVersion(u8),
    BadHeaderLen(u16),
    BadPayloadLen { expected: usize, actual: usize },
    CrcMismatch { expected: u32, actual: u32 },
    InvalidField(&'static str),
    UnsupportedKind(u8),
    UnsupportedFormat(u8),
    BufferTooSmall,
}

impl core::error::Error for ProtocolError {}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { needed, actual } => {
                write!(f, "packet too short: need {needed} bytes, got {actual}")
            }
            Self::BadMagic => write!(f, "bad magic (expected GRNT)"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            Self::BadHeaderLen(v) => write!(f, "bad header length {v}"),
            Self::BadPayloadLen { expected, actual } => {
                write!(f, "bad payload length: expected {expected}, got {actual}")
            }
            Self::CrcMismatch { expected, actual } => {
                write!(f, "CRC mismatch: expected {expected:08x}, got {actual:08x}")
            }
            Self::InvalidField(name) => write!(f, "invalid field: {name}"),
            Self::UnsupportedKind(v) => write!(f, "unsupported packet kind {v}"),
            Self::UnsupportedFormat(v) => write!(f, "unsupported sample format {v}"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn i24_roundtrip() {
        let values = [-8_388_608, -1_000_000, -1, 0, 1, 1_000_000, 8_388_607];
        let mut buf = [0u8; 3];
        for v in values {
            write_i24_le(v, &mut buf);
            assert_eq!(read_i24_le(&buf), v);
        }
    }

    #[test]
    fn sample_formats_roundtrip() {
        for format in [
            SampleFormat::I16,
            SampleFormat::I24Packed,
            SampleFormat::I32,
            SampleFormat::F32,
        ] {
            let mut buf = [0u8; 4];
            for x in [-0.5f32, 0.0, 0.25, 0.9] {
                format.encode(x, &mut buf).unwrap();
                let y = format.decode(&buf).unwrap();
                assert!((x - y).abs() < 1e-3, "{format:?}: {x} -> {y}");
            }
        }
    }

    #[test]
    fn packet_roundtrip() {
        let samples: Vec<f32> = (0..64)
            .map(|i| (i as f32 / 64.0 * core::f32::consts::TAU).sin() * 0.5)
            .collect();
        let header = PacketHeader::data(
            7,
            42,
            123_456,
            987_654_321,
            48_000,
            1,
            samples.len() as u16,
            SampleFormat::I24Packed,
            24,
        );
        let mut buf = [0u8; 512];
        let n = encode_samples(&header, &samples, &mut buf).unwrap();
        let view = PacketView::decode(&buf[..n]).unwrap();
        assert_eq!(view.header.sequence, 7);
        assert_eq!(view.header.device_id, 42);
        assert_eq!(view.samples_per_channel(), 64);
        for (i, expected) in samples.iter().enumerate() {
            let got = view.sample(0, i).unwrap();
            assert!((got - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn corrupted_crc_is_rejected() {
        let header = PacketHeader::data(1, 1, 0, 0, 48_000, 1, 4, SampleFormat::I16, 16);
        let samples = [0.0f32, 0.1, -0.1, 0.2];
        let mut buf = [0u8; 128];
        let n = encode_samples(&header, &samples, &mut buf).unwrap();
        buf[n - 1] ^= 0xff;
        assert!(matches!(
            PacketView::decode(&buf[..n]),
            Err(ProtocolError::CrcMismatch { .. })
        ));
    }

    #[test]
    fn truncated_packet_is_rejected() {
        let header = PacketHeader::data(1, 1, 0, 0, 48_000, 1, 4, SampleFormat::I16, 16);
        let samples = [0.0f32, 0.1, -0.1, 0.2];
        let mut buf = [0u8; 128];
        let n = encode_samples(&header, &samples, &mut buf).unwrap();
        assert!(PacketView::decode(&buf[..n - 1]).is_err());
    }

    #[test]
    fn status_packet_has_no_samples() {
        let header = PacketHeader::status(1, 9, 123, 48_000, "hello");
        let mut buf = [0u8; 128];
        let n = encode_packet(&header, b"hello", &mut buf).unwrap();
        let view = PacketView::decode(&buf[..n]).unwrap();
        assert_eq!(view.header.kind, PacketKind::Status);
        assert_eq!(view.samples_per_channel(), 0);
        assert_eq!(view.payload, b"hello");
    }
}
