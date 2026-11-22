# Digital twin

The digital twin exists so the complete measurement chain — acquisition,
packetisation, impaired transport, reassembly, DSP, classification and
reporting — can be exercised before the physical instrument is built.  It is
**physics-inspired**, not a finite-element model.

## 1. Plate model

A rectangular slab is modelled as a simply-supported Kirchhoff plate with
dimensions `a × b × h`, Young's modulus `E`, density `ρ` and Poisson ratio `ν`.

Flexural rigidity:

```text
D = E h³ / (12 (1 - ν²))
```

Simply-supported bending modes:

```text
f_mn = (π/2) · sqrt(D / (ρ h)) · [ (m/a)² + (n/b)² ]
```

For the nominal granite slab (`600 × 400 × 20 mm`, `E = 50 GPa`,
`ρ = 2700 kg/m³`, `ν = 0.25`):

```text
D      ≈ 35.6 kN·m
sqrt(D/(ρh)) ≈ 25.7 m²/s
f_11   ≈ 364 Hz
```

Modes are generated up to the configured band (8 kHz in the default
scenarios), sorted by frequency, and weighted by the mode shape at the impact
and sensor points:

```text
coupling_mn = sin(mπx_i/a) sin(nπy_i/b) · sin(mπx_s/a) sin(nπy_s/b)
```

The relative velocity amplitude also includes the modal frequency and modal
mass:

```text
A_mn ∝ coupling_mn · ω_mn / M_mn · (f_mn/1 kHz)^(-0.08)
M_mn = ρ h a b / 4
```

High-order modes would otherwise dominate because velocity is `ω ×`
displacement.  A finite hammer contact time rolls them off:

```text
H_contact(f) = exp(-(f/f_contact)²)
```

with `f_contact ≈ 3 kHz`.  This is a stiffer roll-off than a Gaussian
`exp(-0.5 (f/f_c)²)` and produces a more realistic low-order modal spectrum.

Damping is a material loss factor that rises slowly with frequency:

```text
ζ(f) = ζ_0 · (1 + 0.08 · log2(max(f, 50 Hz)/1 kHz))
```

`ζ_0 = 0.0022` for granite, `0.003` for marble, `0.006` for concrete.  Values
are clamped to `[1e-5, 0.2]`.

### Mode selection

A real plate has a dense spectrum.  For a given impact/sensor pair, most
high-order modes couple weakly.  The twin:

1. applies the contact roll-off to each modal amplitude;
2. sorts by resulting amplitude;
3. keeps at most 22 modes;
4. rejects a mode if it is within `max(3 Hz, 0.4%·f)` of an already-kept mode.

This preserves close mode-splitting pairs (which are a defect signature) while
avoiding degenerate duplicates.

## 2. Impact response

Each tap is a start time, amplitude and contact corner.  The clean signal is
the sum of exponentially decaying sinusoids:

```text
v(t) = Σ_taps Σ_modes A_mn · H_contact(f_mn)
       · exp(-ζ_mn ω_mn (t - t_tap))
       · sin(ω_mn sqrt(1-ζ_mn²) (t - t_tap))
```

The oscillator is evaluated with a stable rotation recurrence (one complex
multiply per sample per mode), not four trigonometric calls per sample.  The
clean signal is normalized to a configurable peak (0.7 by default) before the
sensor chain.

Default taps: 0.20 s, 0.95 s and 1.70 s, with slightly different amplitudes and
contact corners to mimic a human hammer sequence.  The capture is 2.4 s.

## 3. Defect models

All defects modify the modal frequencies, damping ratios and, where relevant,
the mode count.  They are phenomenological signatures, not crack-tip stress
analysis.

| Defect | Stiffness factor | Frequency scale | Damping factor | Mode splitting | Extra mode |
|---|---|---|---|---|---|
| Crack | `1 - 0.055·s` | `sqrt(...)` | `1 + 4.0·s` | 3 lowest modes, `±2.5%·s` | — |
| Delamination | `1 - 0.020·s` | `sqrt(...)` | `1 + 2.5·s` | 2 lowest modes, `±2.0%·s` | layer mode at `0.34·f_min` |
| Void | `1 - 0.040·s` | `sqrt(...)` | `1 + 2.6·s` | — | — |
| Thickness variation | `1 + 0.100·s` | `sqrt(...)` | 1.0 | — | — |

`s` is severity in `[0, 1]`.  A split copy keeps 52% of the original amplitude
and gets 1.25× the damping; the original is reduced to 85% so the three lines
are comparable.  The delamination's low mode gets 35–70% of the strongest modal
amplitude and 2.5–4.5× the material damping.

The crack scenario uses `s = 0.55`: the fundamental moves from 363.9 Hz to
358.4 Hz (−1.5%) and damping rises from 0.0022 to about 0.006–0.012.  The split
lines around 353/358/363 Hz are visible in the spectrum and produce a
mode-splitting score of about 0.52.

The delamination scenario uses `s = 0.70` and adds a mode near 121 Hz, far
below the 364 Hz reference fundamental.

## 4. Sensor and ADC model

`SensorConfig` models a laser vibrometer and a 16-bit ADC:

- white Gaussian noise at a configured SNR relative to the clean signal RMS;
- pink-ish noise from a one-pole low-passed white sequence, renormalised to a
  fraction of the white RMS;
- mains sine at 50 Hz with a configurable dBFS level;
- DC offset;
- gain;
- hard clipping to `[-1, 1]`;
- quantization to `2^(bits-1)` levels.

Defaults: 38 dB SNR, 45% pink noise, −62 dBFS mains, 16-bit quantization, 0.7
signal peak.  The `healthy-noisy` scenario uses 22 dB SNR; the `overloaded`
scenario uses gain 2.4 and produces about 0.23% clipped samples, which triggers
a data-quality warning without a false defect.

The sensor model does **not** include laser speckle, target reflectivity,
photodiode non-linearity, ambient light, or mechanical mounting resonances.
Those are the main risks in the real system and must be measured, not simulated.

## 5. Network model

`NetworkConfig` encodes real `granite-protocol` datagrams, then applies:

| Impairment | Default | Scenario example |
|---|---|---|
| Packet loss | 0 | 3% in `lossy-link` |
| Duplication | 0 | 10% in a transport unit test |
| Corruption | 0 | 50% in a transport unit test |
| Jitter | 0 | 1.5–4 ms |
| Reordering | 0 | 10–15% |
| Packet size | 256 frames | 256 frames |
| Sample format | I24 packed | I24 packed |
| Base latency | 10 ms | 10 ms |

The impaired datagrams are sorted by arrival time and fed through the production
`StreamAssembler`.  The report's loss, gap, duplicate, reorder and jitter
figures therefore come from the same code that will process live UDP.

## 6. Scenario catalogue

| ID | Expected | What it exercises |
|---|---|---|
| `healthy` | HEALTHY | nominal baseline |
| `healthy-noisy` | HEALTHY | 22 dB SNR, pink noise, mains, light loss |
| `lossy-link` | HEALTHY | 3% loss, 4 ms jitter, reordering |
| `crack` | DEFECT | frequency shift, damping increase, mode splitting |
| `delamination` | DEFECT | new low-frequency layer mode |
| `void` | DEFECT | stiffness loss and damping increase |
| `thickness-plus` | SUSPECT | +3.9% frequency shift without damping change |
| `overloaded` | HEALTHY | clipping warning, no false defect |

The `thickness-plus` case is a reference mismatch, not a crack.  It is labelled
`SUSPECT` because a slab that is systematically thicker than the reference
should not be accepted as "healthy" without re-baselining.

## 7. Reference handling

For each scenario, `healthy_reference`:

1. clones the scenario;
2. removes the defect;
3. uses a clean link and a 45 dB SNR sensor;
4. synthesises three taps;
5. analyses the capture;
6. builds the reference profile from the stable modes and scalar features.

This mimics measuring a pristine slab of the same geometry and support.  The
defect scenario is then classified against that independent healthy baseline.
The reference is not built from the defect capture.

## 8. Validation results

Current `granite-twin suite` result:

| Scenario | Expected | Predicted | Score | f1 (Hz) | Mean damping |
|---|---|---|---:|---:|---:|
| healthy | HEALTHY | HEALTHY | 1.9 | 363.9 | 0.0023 |
| healthy-noisy | HEALTHY | HEALTHY | 4.0 | 363.9 | 0.0023 |
| lossy-link | HEALTHY | HEALTHY | 4.0 | 363.9 | 0.0022 |
| crack | DEFECT | DEFECT | 70.5 | 353.3 | 0.0118 |
| delamination | DEFECT | DEFECT | 71.4 | 121.1 | 0.0157 |
| void | DEFECT | DEFECT | 57.6 | 357.3 | 0.0075 |
| thickness-plus | SUSPECT | SUSPECT | 35.1 | 378.2 | 0.0023 |
| overloaded | HEALTHY | HEALTHY | 24.9 | 363.9 | 0.0022 |

**8/8 scenarios matched.**  The healthy cases are well below the healthy
threshold (30); the defect cases are above the defect threshold (55); the
thickness mismatch sits in the suspect band.  This is a validation of the
software chain, not proof that the method will work on real granite.

The crack report flags:

```text
unexpected mode at 353.3 Hz (persistence 100%)
damping increased 6.5x versus reference
closely spaced modes detected (splitting score 0.52)
```

The delamination report flags the new 121 Hz mode, an 8.3× damping increase and
the expected low-frequency mode.  The overloaded report shows a clipping
warning and reduced confidence while still calling the slab healthy.

## 9. Running and extending

```bash
# all scenarios
cargo run -p granite-sim --bin granite-twin -- suite --out out/suite

# one scenario
cargo run -p granite-sim --bin granite-twin -- run --scenario delamination

# a different deterministic seed
cargo run -p granite-sim --bin granite-twin -- suite --seed 123 --out out/suite-123

# a subset
cargo run -p granite-sim --bin granite-twin -- suite --only healthy,crack
```

Each scenario writes `capture.wav`, `report.html`, `scope.html`, `modes.csv`,
`features.csv`, `events.csv`, `reference.txt` and `summary.txt`.  The suite
index is `out/suite/index.html`.

### Animated scope

`report.html` is the static engineering report.  `scope.html` is the visual
demonstration: it embeds the reconstructed samples, the RMS envelope, the
averaged spectrum and the transport statistics, then animates them with plain
JavaScript in a canvas.  The wave scrolls under a playhead, impacts are marked,
packet gaps are shaded, the packet stream highlights the current block, and the
**🔇** button turns the capture into a WAV in the browser so the modal
ring-down can be heard.  It autoplays muted on open, supports click-to-seek,
keyboard control, a "next tap" jump and per-mode highlighting.  The file needs
no server and no internet connection.

The suite also writes `showcase.html`, a dashboard with all eight scenarios in
a sidebar and the selected animated scope in an iframe.  It is the most
convenient entry point for a live demonstration.

```bash
# one scenario
cargo run -p granite-sim --bin granite-twin -- scope --scenario crack \
  --out out/scope-crack.html

# all eight scopes and the interactive dashboard are written by the suite
xdg-open out/suite/showcase.html      # Linux
open out/suite/showcase.html          # macOS
```

The scope is generated from the same `ScenarioRun` as the report: the same
production reassembler, the same DSP, the same classifier.  It is not a mock-up
with hand-written numbers.

To add a scenario, edit `catalog()` in `crates/granite-sim/src/scenario.rs`:
choose geometry, material, sensor/impact positions, defect, taps, sensor and
network settings, and an expected label.  Then add a test if the scenario is
part of the acceptance criteria.

## 10. Why not Wokwi?

Wokwi is an excellent simulator for digital logic, GPIO and WiFi sketches, and
it can run an ESP32 that sends UDP.  It is not a substitute for this project's
visual demo, for three practical reasons:

1. **I2S coverage is partial.**  The ESP32 I2S peripheral is not fully modelled,
   and several ESP32 variants have no I2S part at all.  A PCM1808-style external
   ADC stream is therefore not something the simulator can reproduce faithfully.
2. **Rust firmware is not compiled online.**  Wokwi can run a pre-built
   `.bin`/`.elf`, but the Xtensa ESP-IDF build must happen locally.  Every
   firmware change becomes a manual rebuild-and-upload cycle, which is poor for
   a demo.
3. **The PC host cannot easily receive the packets.**  The public Wokwi gateway
   does not support incoming connections or access to the local network, so a
   `granite-host listen` process on the user's machine cannot subscribe to the
   simulated stream.  `host.wokwi.internal` only works with the paid private
   gateway.

Wokwi is still useful for a narrow experiment: a small firmware that sends
synthetic UDP packets over `Wokwi-GUEST`, with a receiver inside the same
simulated network.  That tests WiFi and packet framing, not the optical/analog
front-end, and it is not the end-to-end NDT demo.  The native scope exercises
the complete software path deterministically and can be opened by anyone with a
browser.

## 11. Limitations

- **Simply-supported boundary.** Real slabs rest on foam, are suspended, or are
  clamped.  Boundary conditions change the modes; the reference must use the
  same support.
- **Isotropic, homogeneous material.** Granite is anisotropic and
  heterogeneous at the grain scale.
- **Phenomenological defects.** The crack/delamination/void models reproduce
  modal signatures, not the actual damage mechanics.
- **No optical model.** Speckle, reflectivity, laser mode structure and
  photodiode non-linearity are absent.  Real SNR may be much worse.
- **No temperature or moisture.** Both affect stiffness and damping.
- **No force measurement.** Tap amplitude varies; the classifier uses
  normalized features, but very weak or double taps still hurt.
- **Tuned thresholds.** The thresholds were chosen against this simulation.
  They must be revalidated on real slabs before making any claim.
