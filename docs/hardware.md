# Hardware notes

> The original repository's `hardware/laser_mic_schematic.pdf` was a 38-byte
> placeholder containing the text `% placeholder schematic`.  There is no
> validated schematic in this repository.  This document is a design brief and
> checklist for the hardware partner, not a certified circuit.

## 1. The measurement problem

The instrument measures the **surface velocity** of a granite slab after a
hammer tap.  The slab response is a sum of decaying bending modes; a defect
changes the modal frequencies and damping.  The optical sensor must recover
vibration from roughly 50 Hz to several kHz with enough SNR to estimate damping
over a few hundred milliseconds.

Granite is a diffuse, low-reflectivity, speckled target.  A laser beam reflected
from a polished granite surface is not a clean specular return; it is a speckle
pattern.  A single photodiode placed in that pattern sees a noisy, position-
dependent signal.  This is the largest risk in the project.

Practical mitigations, in order of impact:

1. **Retroreflective marker.** Put a small piece of retroreflective tape or a
   glass-bead marker at the measurement point.  This turns a diffuse target into
   a bright return and dramatically improves SNR.  It is still non-contact if
   the marker is attached to the slab and the sensor is remote; for a truly
   contactless measurement, a small painted dot or a polished epoxy spot can
   help.
2. **Bicell / quadrant / PSD detector.** Use two or four photodiode segments
   and a difference/sum amplifier.  The difference signal is approximately
   linear for small spot motion and rejects common-mode ambient light.
3. **Knife-edge single photodiode.** A razor blade partially occludes the spot;
   vibration moves the spot across the edge and modulates the photocurrent.
   Simple and cheap, but nonlinear and very alignment-sensitive.
4. **Commercial laser vibrometer.** A Doppler vibrometer is the right tool, but
   it is far outside a course budget.

The digital twin assumes a clean velocity signal with configurable SNR.  It does
not model speckle; expect the real system to be worse.

## 2. Optical layout

```text
                   ┌──────────────┐
   laser module ──►│ focus lens   │──► target marker on slab
                   └──────────────┘          │
                                             │ scattered / retroreflected
                                             ▼
                   ┌──────────────┐   ┌──────────────┐
                   │ collection   │──►│ photodiode / │
                   │ lens / iris  │   │ bicell / PSD │
                   └──────────────┘   └──────┬───────┘
                                             │
                                      TIA + filter
                                             │
                                          PCM1808
                                             │ I2S
                                          ESP32
```

Recommendations:

- Mount the laser and detector on a rigid optical breadboard or a heavy
  tripod; the sensor must not pick up its own mount vibration.
- Focus the laser to a small spot (0.5–2 mm) on the marker.
- Use an aperture/iris in front of the detector to reject ambient light and
  stray reflections.
- Avoid a geometry that sends the specular reflection back into the laser.
- Use a laser line or a visible low-power module for alignment, then switch to
  the measurement laser if needed.
- Keep the beam path short and rigid; air currents and table vibration are
  common noise sources.
- Put a black cloth or enclosure around the optical path to reduce 100 Hz
  flicker from mains-powered lamps.

## 3. Analog front-end

### 3.1 Transimpedance amplifier (TIA)

A photodiode is a current source.  The first stage converts photocurrent to
voltage:

```text
V_out = -I_photo · R_f
```

with a feedback capacitor `C_f` for stability and bandwidth:

```text
f_-3dB ≈ 1 / (2π R_f C_f)
```

Starting values for a 48 kHz system:

| Part | Value | Note |
|---|---|---|
| Photodiode | BPW34 (or a bicell/PSD) | fast PIN, 3.3–5 V, low capacitance at zero bias |
| Op-amp | OPA2134 / OPA1642 | low-noise JFET input, 8 MHz, good for audio-band TIA |
| Feedback resistor `R_f` | 100 kΩ | lower noise gain than 1 MΩ, wider bandwidth |
| Feedback capacitor `C_f` | 4.7 pF | ~340 kHz pole with 100 kΩ; adjust for stability |
| Second stage | non-inverting ×10 | bring the signal up to the ADC range |
| Anti-alias filter | 4th-order low-pass, 20 kHz | Sallen-Key or multiple-feedback |
| AC coupling | 1 µF into the ADC bias network | block DC, preserve the vibration band |

`TL072` is cheap and common, but its input noise and bias current are not ideal
for a high-impedance TIA.  Use it only for a first breadboard.  `OPA657` or
`OPA355` are faster alternatives if the photodiode capacitance is large or the
bandwidth must exceed 100 kHz.

A bicell/PSD needs **two** TIAs and a differential amplifier:

```text
V_diff = k · (V_A - V_B)
V_sum  = V_A + V_B
```

Normalize the difference by the sum to reduce sensitivity to laser power and
target reflectivity.  An analog divider is expensive; a digital normalization
on the host is usually enough, or use an AGC/limiter.

### 3.2 Noise and interference

- Use a quiet linear regulator for the analog supply, not the ESP32's 3.3 V
  rail.
- Keep the photodiode and first op-amp in a small shielded enclosure; guard
  the input with a ground ring.
- Use short leads between the photodiode and TIA; stray capacitance at the
  inverting input causes instability.
- Separate analog and digital grounds; join at one point near the ADC.
- Add a ferrite bead and 10 µF/100 nF decoupling at the ADC and op-amp.
- Avoid ground loops between the PC, power supply and slab fixture.
- The host notches 50/100/150 Hz, but prevention is better than post-processing.

## 4. ADC: PCM1808 (recommended)

The ESP32's internal ADC is not suitable for this measurement: 12-bit
resolution, significant noise with WiFi active, non-linear response, and no
proper anti-alias filter.  The original ~8 kHz busy-loop sampler is the main
reason the original project cannot resolve granite plate modes.

Recommended external ADC: **PCM1808** (or PCM1802 / CS5343 / ES8388).

| Parameter | PCM1808 |
|---|---|
| Resolution | 24-bit |
| Sample rate | up to 96 kHz (48 kHz used here) |
| Interface | I2S, stereo |
| Analog supply | 5 V (check module) |
| Digital supply | 3.3 V |
| System clock | 256/384/512 × fs (12.288 MHz at 48 kHz) |
| Input | single-ended, AC-coupled, biased per datasheet |

**Important:** the PCM1808 is a 5 V analog part with 3.3 V digital I/O on many
breakouts, but not all modules are the same.  Check the breakout schematic and
use level shifting if its I2S outputs are 5 V.

The ADC input must be biased to the datasheet's recommended common-mode
voltage and kept within its full-scale range.  AC-couple the front-end output
and set the bias with a clean divider and a capacitor.  Do not exceed the
absolute maximum input voltage.

The PCM1808's digital filter rejects out-of-band noise, but the analog
anti-alias filter is still required to prevent high-frequency noise from
aliasing into the modal band.

## 5. ESP32 wiring

The firmware uses ESP32 classic I2S0.  These pins are a working default; they
can be changed in `firmware/granite-firmware/src/main.rs`.

| ESP32 pin | Signal | Connects to | Direction |
|---|---|---|---|
| GPIO26 | BCLK | PCM1808 BCK | output |
| GPIO25 | LRCK / WS | PCM1808 LRCK | output |
| GPIO27 | MCLK | PCM1808 SCKI | output (12.288 MHz) |
| GPIO33 | DIN | PCM1808 DOUT | input |
| 3V3 | digital supply | PCM1808 VDD (3.3 V) | power |
| 5V | analog supply | PCM1808 VCC / analog rail | power |
| GND | ground | common ground | power |

Notes:

- GPIO34–39 are input-only; GPIO33 is a valid input and is used for DIN.
- Avoid GPIO0, GPIO2, GPIO12 and GPIO15 for new functions; they are strapping
  pins.
- The ESP32 is not 5 V tolerant.  Confirm the ADC breakout's logic levels.
- Keep the I2S wires short and twisted with ground returns.
- If the board uses an ESP32-S3/C3/C6, the pin names and target triple change;
  the I2S/WiFi code is otherwise similar.  ESP32-S3 is the easiest migration
  path for Rust (`xtensa-esp32s3-espidf`), and C6 is RISC-V.

## 6. Power and grounding

```text
USB 5 V ──► ferrite ──► 3.3 V LDO ──► ESP32 / digital
                    └──► 5 V analog ──► op-amp / PCM1808 analog
```

- Power the analog rail from a low-noise LDO or a linear regulator, not a
  switching converter near the photodiode.
- Keep the laser module on a separate switched rail if possible; laser driver
  noise can couple into the TIA.
- Star-ground the analog and digital sections at the ADC.
- Use a shielded enclosure for the TIA; the ESP32 antenna should be outside it.
- Add a bulk capacitor (100–470 µF) at the 5 V input and 10 µF + 100 nF at each
  IC.

## 7. Mechanical setup

- Support the slab consistently: foam pads at the corners, suspension on soft
  cords, or a rigid frame.  The same support must be used for the reference and
  the test.
- Mark the impact point and the sensor point on the slab.  Moving either point
  changes the modal coupling and invalidates the comparison.
- Use a hammer with a hard plastic or brass tip; a soft tip excites fewer high
  modes.  Keep the tap energy similar.
- Avoid double taps.  The host's refractory window rejects closely spaced
  events, but a double tap still contaminates the response.
- Let the slab and electronics reach thermal equilibrium; granite stiffness
  drifts with temperature.
- For a truly non-contact measurement, do not touch the slab during the
  capture.  The tap itself is the only intentional contact.

## 8. Safety

- Use the lowest laser power that gives adequate SNR.  A 1–5 mW 650 nm module
  is already hazardous if viewed directly or focused.
- Wear laser safety goggles matched to the wavelength.
- Never aim the beam at people, animals, vehicles, aircraft or unknown
  reflective surfaces.
- Use a beam dump behind the target.
- Keep the laser off when aligning the detector by eye; use a camera or a
  fluorescent card.
- Do not rely on the software to make a structural-safety decision.  This is an
  educational/research instrument.

## 9. Bring-up checklist

1. Verify the 3.3 V and 5 V rails with a multimeter before inserting the ADC.
2. Check the PCM1808 MCLK, BCLK and LRCK with an oscilloscope:
   - MCLK ≈ 12.288 MHz;
   - BCLK = 64 × fs = 3.072 MHz;
   - LRCK = 48 kHz.
3. Inject a 1 kHz sine into the analog front-end from a signal generator;
   confirm a clean tone in the host WAV/report.
4. Measure the noise floor with the laser blocked and with the laser on a
   stationary target.  Compare the two; ambient light and laser noise should
   not dominate.
5. Tap a metal plate and confirm the host detects events and finds modes.
6. Tap the granite slab with a retroreflective marker; compare the spectrum
   with an accelerometer or piezo reference if available.
7. Measure a pristine slab 10 times; check frequency and damping repeatability.
   Build the reference from the best capture or average multiple captures.
8. Introduce a controlled defect (a small saw cut or a clamped crack) and check
   that the classifier moves away from healthy.
9. Only then attempt field measurements.

## 10. What the original design did

The original Arduino sketch:

- used `analogRead(GPIO34)` in a busy loop at about 8 kHz;
- had no anti-alias filter;
- transmitted raw 16-bit samples over UDP with no sequence, timestamp, sample
  rate, channel count or CRC;
- had no packet-loss/reorder handling;
- wrote a WAV on the PC with a 20-line Python script;
- claimed a schematic and several files that were not present.

The rewrite replaces the acquisition and transport completely.  The original
8 kHz path cannot resolve a 364 Hz fundamental plus higher modes with useful
damping estimates, and its 12-bit noisy ADC limits dynamic range.  The new
PCM1808 + I2S path is the minimum practical configuration for this project.

## 11. Bill of materials

See [`bom/bom.csv`](../bom/bom.csv).  Prices are approximate and depend on
supplier and country.  The BOM includes a recommended baseline and optional
calibration items.
