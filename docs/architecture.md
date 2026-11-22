# Architecture

## Goals

1. Measure impact-modal parameters of a granite slab with a non-contact laser
   vibrometer.
2. Keep the measurement chain transparent and testable without hardware.
3. Make the PC responsible for all signal processing and classification.
4. Recover cleanly from packet loss, reordering and duplicates on WiFi.
5. Produce a report that can be opened offline or attached to a course
   submission.
6. Build with stable Rust and no third-party host dependencies.

## Crate graph

```text
granite-protocol  (no_std, no deps)
       ▲     ▲
       │     │
granite-core  │
   ▲     ▲    │
   │     │    │
granite-sim  granite-host
   ▲           ▲
   └───────────┘
        both are binaries + libraries
```

| Crate | Responsibility | Depends on |
|---|---|---|
| `granite-protocol` | Header, sample codecs, CRC32, packet validation | none |
| `granite-core` | Reassembly, DSP, modal analysis, classifier, WAV, HTML report | `granite-protocol` |
| `granite-sim` | Plate model, defects, sensor/network simulation, scenario suite, animated HTML scope, `granite-twin` CLI | protocol + core |
| `granite-host` | Live UDP receiver, WAV input, reference builder, `granite-host` CLI | protocol + core |

The firmware is intentionally outside this graph and outside the Cargo
workspace.  It depends only on `granite-protocol`, so the exact wire format is
shared, but it does not pull in `std`-only analysis or reporting code.

## End-to-end data flow

### Acquisition

```text
laser diode
   │  reflected beam
   ▼
photodiode / bicell / PSD
   │
   ▼
transimpedance amplifier ──► anti-alias low-pass ──► PCM1808 (24-bit ADC)
                                                          │ I2S
                                                          ▼
                                                   ESP32 (ESP-IDF)
```

The ESP32 configures I2S0 as a controller:

- 48 kHz sample rate;
- 32-bit slots, 24 valid bits, Philips I2S format;
- MCLK = 256 × fs = 12.288 MHz;
- stereo frames from the ADC, left channel used;
- 256 frames per UDP packet.

The firmware loop is single-threaded:

1. blocking `I2sDriver::read` into a 2048-byte buffer (256 stereo frames);
2. take the left 32-bit slot, shift right by 8 to get a signed 24-bit value;
3. normalise to `f32`;
4. encode a `granite-protocol` data packet;
5. `UdpSocket::send` to the configured host;
6. advance `sequence` and `first_sample_index`.

At 48 kHz, one packet is 5.33 ms of data and the packet rate is 187.5/s.  There
is no separate sender thread; a blocking WiFi send is acceptable at this rate.
If the link stalls, the I2S DMA buffers absorb a few packets and the host sees
a gap or a sequence jump.  This is preferable to unbounded buffering on a
microcontroller with limited RAM.

### Transport

The host binds a UDP socket and feeds each datagram to `PacketView::decode`:

- header validation (magic, version, lengths, ranges);
- CRC32 over header + payload;
- sample decode;
- `StreamAssembler::push`.

The assembler de-duplicates by sequence, sorts by sample index and zero-fills
missing ranges.  `finish` returns a `Capture` with:

```rust
Capture {
    sample_rate_hz,
    samples: Vec<f32>,
    start_sample_index,
    gaps: Vec<SampleGap>,
    stats: Option<PacketStats>,
    source,
}
```

The rest of the pipeline never needs to know whether the samples came from UDP,
a WAV file or the digital twin.

### Analysis

`granite-core::analysis::analyze_capture` performs:

```text
DC removal
  → 20 Hz high-pass
  → 50/100/150 Hz notches
  → optional low-pass
  → 2 ms moving-RMS envelope
  → robust threshold (median + k·MAD, k = 7–8)
  → threshold-crossing impact detection
  → per-impact half-Hann window
  → zero-padded FFT
  → modal peak picking
  → per-mode band-pass + Hilbert envelope + log-linear damping fit
  → mode clustering across taps
  → feature extraction and quality metrics
```

Details and parameter choices are in [`dsp.md`](dsp.md).

### Classification

A `ReferenceProfile` stores:

- the slab descriptor (length, width, thickness, material);
- the stable modal frequencies and damping ratios;
- tolerances derived from the reference measurement;
- scalar feature means and standard deviations.

The classifier matches measured modes to reference modes, computes weighted
z-scores, combines them into an anomaly score in `[0, 100]`, and labels the
capture:

| Score | Label |
|---:|---|
| `< 30` | `HEALTHY` |
| `30..55` | `SUSPECT` |
| `>= 55` | `DEFECT` |
| no usable modes | `UNKNOWN` |

The score is not a probability.  It is a normalized distance from the reference
that is easy to explain and tune.  Flags and explanations are generated from
the same evidence (frequency shift, damping increase, new low mode, splitting,
missing modes, data-quality warnings).

### Reporting

`granite-core::report` renders a single HTML file with:

- condition badge, anomaly score and gauge;
- assessment flags and interpretation;
- waveform SVG with impact markers and gap shading;
- averaged amplitude spectrum SVG with modal markers;
- spectrogram as an inline base64 BMP (no JavaScript, no external image);
- modal parameter table;
- feature comparison table against the reference;
- transport statistics and data-quality warnings;
- digital-twin ground truth, when present;
- method notes.

WAV, modes CSV, features CSV, events CSV, reference profile and a text summary
are written alongside the report.  The CSV files are intentionally simple so
they can be imported into a spreadsheet or a Python notebook.

## Concurrency and real-time behaviour

- **Firmware:** one acquisition/send loop.  No dynamic allocation after the
  buffers are created.  I2S read is blocking; UDP send has a 100 ms write
  timeout.  A status line is logged every 5 s.
- **Host live receiver:** blocking `recv_from` with a 200 ms socket timeout so
  the loop can check the duration/idle conditions.  Analysis is post-capture,
  not streaming.  A 10 s capture at 48 kHz is 480k samples (1.9 MB as `f32`),
  which is trivial for a PC.
- **Digital twin:** one scenario at a time; deterministic seeds.  The suite is
  CPU-bound and takes a few seconds in a debug build.
- **Animated scope / showcase:** generated after analysis.  The browser only
  replays embedded samples and precomputed analysis; it does not reimplement
  DSP.  `showcase.html` switches between `scope.html` files in an iframe.

A future real-time display can run the same analysis on a sliding window, but
the current design deliberately favours reproducibility over latency.

## Error handling

- `granite-protocol` returns typed `ProtocolError` values and never panics on
  malformed input.
- `StreamAssembler` returns `StreamError` for incompatible streams and
  `NoPackets` for an empty capture.
- The host receiver counts invalid datagrams and continues; it only aborts on
  socket errors or if no data arrives within the startup timeout.
- Analysis handles silence, clipping, gaps and missing modes by returning an
  empty/unusable result with warnings rather than panicking.
- The CLI prints a human-readable error and exits non-zero.

## Testing strategy

1. **Unit tests** for CRC, packet round-trips, chunked FFT/IFFT, windows,
   spectra, biquads, envelope, statistics, RNG, reassembly and WAV.
2. **Synthetic modal tests** with known frequencies and damping ratios.
3. **Protocol/transport tests** for loss, reorder, duplication and corruption.
4. **Digital-twin scenarios** with known ground truth: healthy, noisy, lossy,
   crack, delamination, void, thickness mismatch, overload.
5. **Host UDP loopback test**: encode packets, send them over a real socket,
   receive and reassemble them with the production receiver.
6. **CLI smoke tests**: `granite-twin selftest`, `suite`, and host `analyze`.

The test suite does not require hardware, network access or external crates.

## Design decisions and trade-offs

| Decision | Reason | Trade-off |
|---|---|---|
| PC-side DSP | testable, versionable, no MCU float/memory limits | not a standalone instrument; needs a host |
| Fixed 52-byte header | simple, predictable, no serialization library | less flexible than protobuf/CBOR |
| CRC32 | catches corruption cheaply on MCU | not cryptographic |
| 24-bit I2S ADC | much better than the ESP32 internal ADC | extra component and analog design |
| 48 kHz | enough for plate modes up to ~20 kHz | more data than the original 8 kHz |
| Threshold-crossing event detection | robust to multi-mode beat lobes | assumes separated taps |
| Half-Hann window | matches an impact that starts at the window start | not ideal for continuous signals |
| Log-linear envelope fit | direct damping estimate | sensitive to neighbouring modes and noise |
| Relative classifier | no absolute calibration needed | needs a good reference and matching setup |
| Inline HTML/BMP | offline, no dependencies | larger report files |
| No host crates | fast, reproducible builds | no plotting/GUI library |

## Where to change things

- **Sample rate / packet size:** `firmware/granite-firmware/src/main.rs` and
  `crates/granite-sim/src/scenario.rs`.
- **Trigger/damping thresholds:** `AnalysisConfig` in
  `crates/granite-core/src/analysis.rs`.
- **Classifier thresholds/weights:** `ClassifierConfig` in
  `crates/granite-core/src/classify.rs`.
- **Defect signatures:** `crates/granite-sim/src/defect.rs`.
- **Plate model:** `crates/granite-sim/src/plate.rs`.
- **Report layout:** `crates/granite-core/src/report.rs`.
- **Wire format:** `crates/granite-protocol/src/lib.rs` and
  [`protocol.md`](protocol.md).
