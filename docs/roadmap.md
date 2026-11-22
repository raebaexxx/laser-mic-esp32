# Roadmap and bring-up plan

## Current state

- [x] Rust workspace with protocol, core, host and simulation crates.
- [x] Framed UDP protocol with CRC32, sample index, timestamp and rate.
- [x] Reassembly with loss, duplicate, reorder and jitter statistics.
- [x] DSP pipeline and modal parameter estimation.
- [x] Reference profiles, anomaly scoring and condition labels.
- [x] Self-contained HTML report with waveform, spectrum and spectrogram.
- [x] WAV/CSV export.
- [x] Physics-inspired digital twin with eight validation scenarios (8/8 pass).
- [x] Host UDP loopback test.
- [x] Rust ESP-IDF firmware source for I2S + WiFi/UDP.
- [x] Firmware compiles and links with ESP-IDF v5.5.
- [ ] Firmware flashed and validated on real hardware.
- [ ] Analog front-end built and characterised.
- [ ] Laser vibrometer tested on granite.
- [ ] Reference measured on a real pristine slab.
- [ ] Controlled-defect validation on a scrap slab.

## Phase 0 — software baseline (done)

Deliverables:

- `granite-protocol`, `granite-core`, `granite-host`, `granite-sim`;
- digital-twin suite and reports;
- self-contained animated HTML scope with audio playback;
- documentation.

Acceptance:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p granite-sim --bin granite-twin -- selftest
cargo run -p granite-sim --bin granite-twin -- suite --out out/suite
```

All tests pass; suite reports 8/8.

## Phase 1 — electronics bring-up

Tasks:

1. Build the TIA and anti-alias filter on a breadboard or breakout.
2. Verify the PCM1808 clocks and I2S data with a logic analyser or scope.
3. Compile and flash the firmware; confirm the host receives packets.
4. Inject a 1 kHz sine and confirm the WAV/report.
5. Measure the noise floor with the laser blocked and on a stationary target.
6. Tap a metal plate and confirm event detection and modes.

Acceptance:

- stable 48 kHz stream for at least 60 s;
- packet loss < 0.1% on a local network;
- no CRC errors;
- clean 1 kHz tone in the spectrum;
- modal frequencies repeat within 1% over 10 taps.

Risks:

- ESP-IDF/esp-idf-hal API drift; pin mapping errors;
- PCM1808 breakout logic levels;
- I2S clocking/MCLK configuration;
- WiFi power-save latency.  If jitter is high, disable WiFi power save in
  sdkconfig (`CONFIG_ESP_WIFI_POWER_SAVE=n` or the appropriate option) and
  keep the PC and ESP32 on the same access point.

## Phase 2 — optical bench

Tasks:

1. Compare single-photodiode/knife-edge and bicell/PSD.
2. Test retroreflective marker, painted dot and bare granite.
3. Measure SNR versus laser power, focus, distance and angle.
4. Compare the laser signal with an accelerometer or piezo reference on a
   shaker or a tapped plate.
5. Characterise the TIA bandwidth and noise.

Acceptance:

- laser SNR at least 20 dB on a granite slab at the intended distance;
- no saturation or ambient-light domination;
- modal frequencies agree with the accelerometer reference within ~2%;
- repeatable damping estimates on a healthy slab.

Risks:

- speckle noise;
- insufficient returned light from diffuse granite;
- mechanical resonances in the optical mount;
- 50/100 Hz ambient light and mains pickup;
- thermal drift.

## Phase 3 — granite measurements

Tasks:

1. Select three or four granite slabs of the same nominal geometry.
2. Measure each slab on the same support with the same tap protocol.
3. Build a reference for one slab and test repeatability on the same slab.
4. Compare slabs; quantify slab-to-slab variation.
5. Introduce controlled changes (mass, clamp, saw cut on scrap).
6. Tune `AnalysisConfig` and `ClassifierConfig` on real data; keep the
   digital-twin suite as a regression test.

Acceptance:

- healthy same-slab repeatability: score < 20;
- healthy slab-to-slab variation: documented, ideally < 30 or handled by
  per-slab references;
- controlled defect: score > 55 or a clear mode-level signature;
- report explains the decision.

Risks:

- slab-to-slab material variation larger than the defect signal;
- support/boundary changes dominating;
- only one sensor point, so some defects are invisible;
- classifier thresholds tuned on simulation not transferring.

## Phase 4 — demo and documentation

Tasks:

1. Build a robust enclosure and cable strain relief.
2. Add a simple measurement checklist to the report.
3. Record a demo video: laser alignment, tap, live report.
4. Write the course report: theory, method, digital twin, results, limitations.
5. Prepare a 5-minute presentation with the healthy/defect reports.

Acceptance:

- a non-expert can follow the checklist and produce a report;
- the demo works without a network engineer;
- all safety precautions are visible.

## Possible future work

### Analysis

- Rational fraction polynomial or ERA modal fitting for damping, instead of
  the envelope/half-power approximation.
- Multi-reference classification (several healthy slabs per geometry).
- Uncertainty propagation from frequency/damping standard deviations into the
  anomaly score.
- Automatic mode tracking across captures.
- Feature selection and a small, interpretable classifier (logistic regression
  or a decision tree) trained on real labelled data.
- Temperature compensation using a reference thermistor.

### Hardware

- Bicell/PSD front-end with normalized difference output.
- Better TIA (OPA657/OPA355) and a proper PCB.
- 4th-order anti-alias filter and a quiet analog LDO.
- ESP32-S3/C6 for a simpler Rust toolchain and more RAM.
- Battery power and a physical trigger button.
- Enclosure with a laser interlock and a beam dump.
- Optional accelerometer reference channel on the second I2S slot.

### Software

- Real-time sliding-window display (egui or a web UI).
- PDF export.
- Multi-capture comparison and trend plots.
- Profile averaging/merging tool.
- Automatic report metadata from a YAML/TOML measurement sheet.
- CI job that runs the digital-twin suite and archives the reports.

### Field use

- A measurement database keyed by slab ID.
- Operator prompts for slab dimensions, support and tap point.
- Audit trail with raw WAV, profile and software version.
- Clearly labelled "research use only" output.

## Non-goals

- Certified structural inspection.
- Safety-critical acceptance without a human expert.
- Replacing a commercial ultrasonic or impact-echo instrument.
- A trained machine-learning model without a labelled real dataset.
