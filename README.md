# Granite NDT — laser vibrometer + impact-modal analysis

A complete Rust implementation — with a dependency-free host workspace — of a
non-destructive testing (NDT) system for granite slabs.  A low-power laser vibrometer measures
the surface velocity of a slab after a controlled hammer tap; the host extracts
modal frequencies and damping ratios, compares them with a healthy reference,
and produces a condition report.

This is a rewrite of the original
[`laser-mic-esp32`](https://github.com/ByteWhiz/laser-mic-esp32) educational
laser-microphone project.  The original firmware sampled the ESP32's internal
12-bit ADC in a busy loop at roughly 8 kHz and sent raw UDP bytes.  That is not
adequate for impact-modal NDT: the sample rate is too low for thickness and
higher-order modes, the ADC is noisy and non-linear, and raw UDP has no
sequence numbers, timestamps, sample-rate metadata, or integrity check.

The rewrite keeps the same physical idea but replaces the acquisition,
transport, analysis and reporting chain end-to-end.

> **Course context.** The software in this repository is the author's part of a
> two-person course project.  The analog front-end, laser mounting and slab
> fixture are the hardware partner's part.  A physics-inspired digital twin is
> included so the complete software pipeline can be validated before the
> physical instrument is assembled.

> **По-русски:** краткое объяснение сути и принципа работы — в
> [`docs/overview.ru.md`](docs/overview.ru.md).

**Живая 3D-демонстрация:** [raebaexxx.github.io/laser-mic-esp32](https://raebaexxx.github.io/laser-mic-esp32/)
— интерактивная установка прямо в браузере телефона.

---

## What works today

| Component | Status |
|---|---|
| Wire protocol, CRC32, packet codec | Implemented and unit-tested |
| UDP reassembly with loss/reorder/duplicate/jitter statistics | Implemented and unit-tested |
| DSP: FFT, windows, filters, Hilbert envelope, spectra | Implemented and unit-tested |
| Impact detection, modal peak picking, damping estimation | Implemented and tested on synthetic modes |
| Reference profiles, anomaly scoring, condition labels | Implemented and tested |
| Self-contained HTML report with inline SVG + BMP spectrogram | Implemented |
| Animated HTML scope: scrolling waveform, spectrum, packet stream, audio | Implemented |
| Interactive 3D lab: full rig, clickable slab library, animated mode shapes | Implemented |
| Interactive suite dashboard with all scenarios in one page | Implemented |
| Digital twin: plate model, defects, sensor, network impairments | Implemented; 8/8 validation scenarios pass |
| Host: live UDP receiver, WAV analyser, reference builder | Implemented; UDP loopback test passes |
| ESP32 firmware (Rust, ESP-IDF, I2S + WiFi/UDP) | Compiles and links with ESP-IDF v5.5; not yet flashed on hardware |
| Hardware schematic / PCB | Not in this repository; see `hardware/README.md` |

The host workspace builds and tests with stable Rust and **no external
crates**.  The firmware is a separate, workspace-excluded crate because it
needs the Xtensa ESP-IDF toolchain.

---

## Why this architecture

```
 laser ──► photodiode/PSD ──► TIA + anti-alias ──► PCM1808 ──► I2S
                                                                  │
                                                           ESP32 firmware
                                                        framed UDP packets
                                                                  │
                                                                  ▼
 PC host:  reassembly ──► preprocessing ──► impact detection ──► FFT
                                                                  │
                                          modal peaks + damping ◄─┘
                                                     │
                                          feature extraction
                                                     │
                                    reference comparison / classifier
                                                     │
                                        HTML + WAV + CSV report
```

The microcontroller only acquires and streams raw samples.  All DSP,
classification and reporting run on the PC.  This keeps the measurement method
versionable and testable, avoids floating-point and memory constraints on the
MCU, and makes it possible to re-analyse stored captures when the algorithm
changes.  At 48 kHz, mono, 24-bit, the raw stream is only ~115 kB/s.

---

## Quick start

### 1. Build and test the host side

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### 2. Run the digital twin

```bash
cargo run -p granite-sim --bin granite-twin -- suite --out out/suite
```

This writes one report bundle per scenario plus three entry points:

```text
out/suite/lab.html        interactive 3D rig — start here
out/suite/showcase.html   multi-scenario instrument dashboard
out/suite/index.html      validation index
out/suite/healthy/report.html
out/suite/healthy/scope.html
out/suite/healthy/capture.wav
out/suite/healthy/modes.csv
...
```

Open **`out/suite/lab.html`**.  It is a perspective 3D view of the whole
installation: the granite slab on its supports, the laser vibrometer with the
TIA/filter, the PCM1808 ADC, the ESP32 and its Wi-Fi link, and the analysis
laptop.  The rack behind the bench is the slab library: click any tile, or a
name in the bottom strip, and that scenario is loaded.  The verdict, modal
frequencies, damping and flags update, and the plate visibly rings with the
detected modes.  Drag to orbit, use the wheel to zoom, press **⚡ Tap now** to
excite the plate, and **🔇** for synthesized audio.  The laptop screen mirrors
the reconstructed signal and the packet dots show the UDP link, including lost
packets.  The animation is slowed down and the deflection is exaggerated so the
motion is visible; the frequencies and damping come from the Rust analysis.

**`out/suite/showcase.html`** is the companion instrument dashboard: pick a
scenario on the left and the full animated scope loads on the right without a
page reload.  Each scope is a scrolling waveform with impact markers and packet
gaps, a moving-RMS trigger strip, the averaged modal spectrum with detected and
reference modes, transport counters, and the mode table.  It starts animating
immediately (muted); click **🔇** for sound, click the trace to seek, use
**⏭ Next tap** to jump to an impact, and click a mode row to highlight it in the
spectrum.  Nothing is fetched from the network — the HTML embeds the samples
itself.

A single animated scope without the full bundle:

```bash
cargo run -p granite-sim --bin granite-twin -- scope --scenario crack \
  --out out/scope-crack.html
```

The 3D lab alone (it runs all eight scenarios):

```bash
cargo run -p granite-sim --bin granite-twin -- lab --out out/lab.html
```

For the full demo — tests, suite, host analysis and automatic browser launch
when a desktop is available:

```bash
./scripts/demo.sh
```

> **Why not Wokwi?** Wokwi is useful for WiFi/UDP and GPIO experiments, but it
> cannot run this whole acquisition chain: ESP32 I2S simulation is only partial,
> the Rust firmware must be built locally and uploaded as a `.bin`/`.elf`, and
> the public gateway does not allow the PC host to receive packets from the
> simulated board.  The native HTML scope runs the real host DSP on the real
> digital-twin byte stream, so it is the reliable visual demo.  Wokwi remains an
> option for a small WiFi/protocol-only test; see
> [`docs/digital-twin.md`](docs/digital-twin.md#10-why-not-wokwi).

The current validation result is **8/8 scenarios matched**:

| Scenario | Expected | Predicted | Score |
|---|---|---:|---:|
| healthy | HEALTHY | HEALTHY | 1.9 |
| healthy-noisy | HEALTHY | HEALTHY | 4.0 |
| lossy-link | HEALTHY | HEALTHY | 4.0 |
| crack | DEFECT | DEFECT | 70.5 |
| delamination | DEFECT | DEFECT | 71.4 |
| void | DEFECT | DEFECT | 57.6 |
| thickness-plus | SUSPECT | SUSPECT | 35.1 |
| overloaded | HEALTHY | HEALTHY | 24.9 |

A single scenario:

```bash
cargo run -p granite-sim --bin granite-twin -- run --scenario crack
cargo run -p granite-sim --bin granite-twin -- list
cargo run -p granite-sim --bin granite-twin -- selftest
```

### 3. Analyse a WAV file with the host

The digital-twin WAVs are ordinary 32-bit float mono WAV files, so the host can
analyse them exactly like a live capture:

```bash
cargo run -p granite-host --bin granite-host -- analyze \
  --wav out/suite/healthy/capture.wav \
  --reference out/suite/healthy/reference.txt \
  --out out/host-healthy
```

Build a reference profile from a pristine slab capture:

```bash
cargo run -p granite-host --bin granite-host -- reference \
  --wav healthy.wav --out reference.txt \
  --name "Black Galaxy slab A" \
  --length 0.600 --width 0.400 --thickness 0.020 --material granite
```

### 4. Receive live data from the ESP32

```bash
cargo run -p granite-host --bin granite-host -- listen \
  --bind 0.0.0.0:5005 --duration 10 \
  --reference reference.txt --out out/live
```

Use `--duration 0` to stop after `--idle` seconds of silence instead of a fixed
duration.  The receiver prints packet loss, jitter and invalid-datagram counts.

### 5. Build the firmware

The firmware is excluded from the workspace and needs the Espressif Rust
toolchain:

```bash
cargo install espup espflash
espup install
. "$HOME/export-esp.sh"

cd firmware/granite-firmware
GRANITE_WIFI_SSID="my-network" \
GRANITE_WIFI_PASS="my-password" \
GRANITE_HOST_IP="192.168.1.50" \
cargo run --release
```

See [`firmware/README.md`](firmware/README.md) for wiring, pin assignments,
sdkconfig notes and troubleshooting.

> The firmware was built with `esp-idf-hal 0.47`, `esp-idf-svc 0.53` and
> ESP-IDF v5.5 (`cargo build --release` produces a 32-bit Xtensa ELF).  It has
> **not** been flashed or run on hardware.  Treat the first flash as a bring-up
> step; check the clock pins, ADC slot width and WiFi link before trusting the
> data.

---

## Repository layout

```text
crates/
  granite-protocol/   no_std wire format, CRC32, sample codecs
  granite-core/       DSP, analysis, classifier, WAV I/O, HTML report
  granite-sim/        plate model, defects, sensor/network simulation, CLI
  granite-host/       UDP receiver, WAV analysis, reference builder, CLI
firmware/
  granite-firmware/   Rust ESP-IDF firmware (I2S + WiFi/UDP)
docs/
  architecture.md     system design and data flow
  protocol.md         byte-level UDP specification
  dsp.md              algorithms and parameter choices
  digital-twin.md     plate model, scenarios and validation
  hardware.md         analog front-end, ADC, wiring and safety
  calibration.md      reference measurement and tap procedure
  roadmap.md          bring-up plan and future work
  BUILD_NOTES.md      toolchain prerequisites and troubleshooting
bom/
  bom.csv             updated bill of materials
hardware/
  README.md           schematic status, block diagram and design checklist
```

---

## What was wrong with the original

| Original | Rewrite |
|---|---|
| Arduino sketch, `analogRead(GPIO34)` in a busy loop | Rust ESP-IDF, external 24-bit I2S ADC at 48 kHz |
| ~8 kHz, 12-bit, noisy internal ADC | 48 kHz, 24-bit, proper anti-alias front-end |
| Raw UDP, no metadata | 52-byte header with magic, version, sequence, sample index, timestamp, rate, channels, format, CRC32 |
| No loss/reorder handling | Jitter buffer/reassembler with gaps, duplicates, reorder and jitter statistics |
| 20-line Python raw dump | Full host pipeline: WAV, DSP, modes, damping, features, classifier, HTML/CSV |
| No reference/calibration concept | Healthy reference profiles with tolerances and anomaly scoring |
| README claims missing files and a full schematic | Docs match the repository; missing hardware is explicitly marked |
| No way to test without hardware | Physics-inspired digital twin with impairments and validation suite |

The original repository's `hardware/laser_mic_schematic.pdf` was a 38-byte
placeholder containing the text `% placeholder schematic`; the claimed
`audio_capture.h`, `network_utils.h`, `calibration_guide.md`,
`theory_background.md` and `safety_guidelines.md` did not exist.  This rewrite
does not pretend otherwise.

---

## Measurement principle

A hammer tap injects broadband energy into the slab.  The slab responds with a
sum of decaying bending modes:

```text
v(t) = Σ A_i · exp(-ζ_i ω_i t) · sin(ω_i √(1-ζ_i²) t + φ_i)
```

A crack, void or delamination changes the local stiffness and/or dissipation,
which appears as:

- a downward shift of one or more modal frequencies (reduced stiffness);
- an increase in modal damping (energy dissipated at the defect);
- mode splitting (symmetry breaking);
- a new low-frequency mode for a delaminated layer;
- changed high-frequency energy ratio and spectral centroid.

The classifier compares the measured modes and scalar features with a reference
profile measured on a pristine slab of the same geometry and support condition.
It is a **relative** method: the same slab, sensor position and boundary
conditions are essential for a meaningful comparison.

Granite is diffuse and only weakly reflective.  A single photodiode with a
knife-edge or a bicell/PSD detector is more practical than a Michelson
interferometer.  A small retroreflective marker at the measurement point
dramatically improves SNR.  This is discussed in
[`docs/hardware.md`](docs/hardware.md).

---

## Safety and limitations

- **Laser safety:** use the lowest power that gives adequate SNR, never aim at
  people, vehicles, aircraft or reflective surfaces that can send the beam
  elsewhere, and wear appropriate goggles.  A 650 nm module in the 1–5 mW class
  is not a toy.
- **Not a certified inspection.** The classifier is a research/educational
  tool.  It must not be used as the sole basis for structural acceptance,
  safety-critical decisions or commercial quality release.
- **Granite is difficult.** Speckle, surface finish, ambient light, thermal
  drift and support conditions can dominate the measurement.  The digital twin
  is optimistic compared with a real diffuse slab.
- **Boundary conditions matter.** A slab resting on foam, suspended on wires,
  or clamped will have different modes.  The reference and the test capture must
  use the same support.
- **Firmware is untested on hardware.** The host and twin are tested; the
  firmware needs a bring-up session with the actual board and ADC.

---

## Credits

The original educational laser-microphone project was created by **Tommy Tal**
and **Zion Swissa** (`ByteWhiz/laser-mic-esp32`) and is MIT-licensed.  This
rewrite keeps the physical concept, replaces the acquisition/transport/analysis
chain, and adds a digital twin and a full host pipeline.

## License

MIT.  See [`LICENSE`](LICENSE).
