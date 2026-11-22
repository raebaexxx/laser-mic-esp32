# Wire protocol

The firmware and host share the exact same `no_std` codec in
`crates/granite-protocol`.  Every UDP datagram is self-describing and
CRC-protected.  All multi-byte fields are **little-endian**.

## Data packet layout

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| 0 | 4 | `magic` | ASCII `GRNT` (`0x47 0x52 0x4e 0x54`) |
| 4 | 1 | `version` | currently `1` |
| 5 | 1 | `kind` | `1` data, `2` status, `3` hello, `4` ack |
| 6 | 2 | `flags` | reserved, zero |
| 8 | 2 | `header_len` | `52` |
| 10 | 4 | `sequence` | per-device packet counter, wraps at `u32::MAX` |
| 14 | 4 | `device_id` | arbitrary device identifier |
| 18 | 8 | `first_sample_index` | index of the first sample in the device sample clock |
| 26 | 8 | `timestamp_ns` | device monotonic time of the first sample, nanoseconds |
| 34 | 4 | `sample_rate_hz` | e.g. `48000` |
| 38 | 2 | `channels` | `1..=8` |
| 40 | 2 | `samples_per_channel` | `1..=512` |
| 42 | 1 | `sample_format` | `1` I16, `2` I24 packed, `3` I32, `4` F32 |
| 43 | 1 | `bits_per_sample` | `1..=32`; for I24 use `24` |
| 44 | 2 | `payload_len` | bytes after the header, `<= 1400` |
| 46 | 2 | `reserved` | zero |
| 48 | 4 | `crc32` | CRC-32 over bytes `0..48` and the payload |
| 52 | ... | payload | interleaved samples |

The header is exactly **52 bytes**.  The maximum payload is **1400 bytes**, so
a datagram stays below a typical 1500-byte Ethernet MTU after IP/UDP headers.

## Payload order

Samples are interleaved by frame:

```text
frame 0 channel 0
frame 0 channel 1
...
frame 1 channel 0
frame 1 channel 1
...
```

With mono data and `I24Packed`, a 256-frame packet is:

```text
52 + 256 × 3 = 820 bytes
```

At 48 kHz this is 187.5 packets/s and about 154 kB/s on the wire.

## Sample formats

| Value | Format | Bytes/sample | Decode |
|---:|---|---:|---|
| 1 | signed 16-bit PCM | 2 | `i16::from_le_bytes / 32768.0` |
| 2 | signed 24-bit PCM, packed | 3 | sign-extended 24-bit / 8388608.0 |
| 3 | signed 32-bit PCM | 4 | `i32::from_le_bytes / 2147483648.0` |
| 4 | IEEE-754 `f32` | 4 | `f32::from_bits(u32::from_le_bytes)` |

Encoding clamps to `[-1.0, 1.0]` and rounds to the nearest representable
integer.  The firmware uses `I24Packed`, which is the natural format for a
PCM1808-style 24-bit ADC and keeps the packet small.

## CRC-32

The checksum is the standard CRC-32 (IEEE 802.3), reflected polynomial
`0xEDB88320`, initial value `0xFFFFFFFF`, final XOR `0xFFFFFFFF`.  The check
value for the ASCII string `123456789` is `0xCBF43926`.

The CRC covers:

```text
bytes[0..48] || payload[0..payload_len]
```

It does **not** include the CRC field itself (bytes 48..52) or any UDP/IP
headers.

## Status and hello packets

Status and hello packets use the same 52-byte header but carry UTF-8 text and
no samples.  `channels = 0`, `samples_per_channel = 0`, `bits_per_sample = 0`,
and `sample_format` is set to `F32` but ignored.  The host logs the text.

The firmware sends one hello/status packet after WiFi connects:

```text
granite-esp32 ready: fs=48000 channels=1 format=i24 packet_frames=256
```

## Reassembly rules

The host's `StreamAssembler`:

1. rejects datagrams with bad magic, version, header length, payload length or
   CRC;
2. extracts one selected channel from interleaved packets;
3. de-duplicates by `sequence`;
4. sorts packets by `first_sample_index` (then by sequence);
5. zero-fills missing sample ranges and records each gap;
6. reports packets received/lost/duplicate/out-of-order, bytes, gap samples,
   gap count, sequence span, device id, duration and maximum jitter.

The reassembler tolerates a mid-stream sample-rate, channel-count or device-id
change by returning an error rather than silently mixing incompatible data.
A real deployment can run one assembler per device id.

## Loss and jitter model in the digital twin

`crates/granite-sim/src/network.rs` encodes real protocol datagrams and then
applies:

- independent packet loss;
- duplication (same sequence, later arrival);
- payload corruption (CRC failure);
- Gaussian delay jitter;
- extra random delay to force reordering.

The impaired datagrams are fed through the production `StreamAssembler`, so the
transport statistics in the report are produced by the same code that will run
on live data.

## Versioning

`version = 1` is intentionally strict.  A future version can add fields in the
reserved area or define new `kind` values; decoders reject unknown versions and
unknown packet kinds rather than guessing.  The host and firmware must be
updated together when the version changes.
