# Hardware

> **Status:** no validated schematic, PCB or assembly drawing exists in this
> repository.  The original `laser_mic_schematic.pdf` was a 38-byte placeholder
> containing the text `% placeholder schematic`; it has been removed.  This
> directory is a design brief for the hardware partner.

## Block diagram

```text
                        ┌──────────────────────────────┐
  650 nm laser ────────►│ target marker on granite slab│
                        └──────────────┬───────────────┘
                                       │ scattered / retroreflected light
                                       ▼
                        ┌──────────────────────────────┐
                        │ photodiode / bicell / PSD    │
                        └──────────────┬───────────────┘
                                       │ photocurrent
                                       ▼
                        ┌──────────────────────────────┐
                        │ TIA + gain + anti-alias      │
                        │ (OPA2134/OPA1642, 20 kHz LPF)│
                        └──────────────┬───────────────┘
                                       │ analog voltage
                                       ▼
                        ┌──────────────────────────────┐
                        │ PCM1808 24-bit I2S ADC       │
                        │ 48 kHz, 5 V analog, 3.3 V IO │
                        └──────────────┬───────────────┘
                                       │ I2S: BCLK, LRCK, DIN, MCLK
                                       ▼
                        ┌──────────────────────────────┐
                        │ ESP32-WROOM-32               │
                        │ WiFi + framed UDP            │
                        └──────────────┬───────────────┘
                                       │ 802.11
                                       ▼
                                  PC host
```

## Recommended baseline

| Block | Choice | Why |
|---|---|---|
| Detector | BPW34 single photodiode + knife edge, or a bicell/PSD | cheap, fast, available; bicell is more linear |
| TIA | OPA2134/OPA1642, `R_f = 100 kΩ`, `C_f = 4.7 pF` | ~340 kHz pole, low noise, stable |
| Gain | non-inverting ×10 second stage | uses the ADC range |
| Anti-alias | 4th-order low-pass at 20 kHz | prevents aliasing above 24 kHz |
| ADC | PCM1808 breakout | 24-bit, I2S, 48 kHz, easy to source |
| MCU | ESP32-DevKitC (WROOM-32) | matches the original board and firmware |
| Power | USB 5 V + quiet 3.3 V LDO for analog | avoids ESP32 rail noise |
| Target | retroreflective tape/dot | dramatically improves SNR on granite |

## Schematic checklist

The hardware partner should produce a KiCad schematic and a netlist that
includes:

### Photodiode and TIA

- [ ] Photodiode cathode/anode orientation and bias mode (photovoltaic vs
      reverse bias).
- [ ] Op-amp decoupling (100 nF close to each rail, 10 µF nearby).
- [ ] Feedback resistor and capacitor with a guard ring around the inverting
      input.
- [ ] Short, shielded connection from the photodiode to the TIA; no long
      unshielded wires.
- [ ] Output offset/bias compatible with the next stage and ADC.
- [ ] Optional test point for the TIA output.

### Gain and filter

- [ ] AC coupling with the correct polarity and a bias network for the ADC.
- [ ] Anti-alias filter corner and order; simulate in ngspice.
- [ ] Gain that can be changed with a jumper or a trim pot for bring-up.
- [ ] No clipping at the maximum expected laser return.
- [ ] Output impedance compatible with the PCM1808 input.

### ADC and digital

- [ ] PCM1808 analog supply (5 V) and digital supply (3.3 V) decoupling.
- [ ] MCLK, BCLK, LRCK, DOUT nets with series termination if needed.
- [ ] Logic levels compatible with the ESP32 (3.3 V).
- [ ] ADC input bias per datasheet; do not exceed absolute maximum ratings.
- [ ] Optional I2S test points.

### Power and protection

- [ ] Reverse-polarity protection and a fuse on the 5 V input.
- [ ] Separate analog/digital rails or a ferrite bead between them.
- [ ] Star ground near the ADC.
- [ ] ESD protection on external connectors.
- [ ] Laser driver with a current limit and a hardware enable/interlock.
- [ ] Clearly labelled laser warning label and beam dump.

### Mechanical

- [ ] Optical breadboard or rigid base.
- [ ] Laser mount with fine adjustment.
- [ ] Detector mount with an iris/aperture.
- [ ] Strain relief for the USB and I2S cables.
- [ ] Enclosure that does not block the ESP32 antenna.
- [ ] Access to the ADC and test points for bring-up.

## Suggested KiCad/ngspice workflow

1. Draw the TIA and filter in KiCad.
2. Export the analog section to ngspice (or redraw it in a SPICE netlist).
3. Simulate:
   - AC response from 10 Hz to 1 MHz;
   - transient response to a 1 kHz and 10 kHz input;
   - noise contribution of the TIA and second stage;
   - stability with the photodiode capacitance (phase margin > 45°).
4. Verify the anti-alias attenuation at 24 kHz and above.
5. Only then lay out the PCB.

A simple TIA transfer function for the first pass:

```text
V_out(s)/I_photo(s) = -R_f / (1 + s R_f C_f)
```

Add the photodiode capacitance `C_d` in parallel with the op-amp input; the
noise gain and stability depend on `C_d`, `C_f` and the op-amp gain-bandwidth.

## Connectors

Suggested connectors:

| Connector | Signal |
|---|---|
| J1 | 5 V input (USB-C or barrel jack) |
| J2 | laser module (switched 3.3/5 V, current-limited) |
| J3 | photodiode / detector (shielded) |
| J4 | I2S to ESP32 (BCLK, LRCK, DIN, MCLK, 3V3, GND) |
| J5 | optional accelerometer reference |

Keep the photodiode connector close to the TIA.  Do not route the I2S clocks
through the analog section.

## Bring-up measurements

| Measurement | Expected |
|---|---|
| 3.3 V rail | 3.3 V ± 5% |
| 5 V rail | 5 V ± 5% |
| TIA output with laser blocked | near mid-supply, low noise |
| TIA output with laser on a stationary target | stable DC, no oscillation |
| MCLK | 12.288 MHz |
| BCLK | 3.072 MHz |
| LRCK | 48 kHz |
| DOUT | toggling 24-bit data |
| 1 kHz sine input | clean tone in the host report |
| Tap on a metal plate | decaying multi-mode response |

## Risks

1. **Speckle and diffuse reflection.** A single photodiode may see a noisy
   speckle pattern.  Retroreflective marker or bicell/PSD is the mitigation.
2. **Optical misalignment.** The spot must stay on the detector for the whole
   capture; rigid mounts and a large active area help.
3. **Mechanical resonances.** The laser/detector mount can vibrate and add
   modes that are not in the slab.  Isolate the optical bench from the slab
   support.
4. **Ambient light.** 100 Hz lamp flicker and daylight can dominate.  Use an
   aperture, optical filter matched to the laser wavelength, and shielding.
5. **ADC/clock noise.** MCLK and BCLK can couple into the TIA.  Keep them away
   from the analog input and use ground guards.
6. **Power-supply noise.** WiFi current bursts can modulate the analog rail.
   Use a separate LDO and bulk capacitance.
7. **Boundary conditions.** The slab support is part of the measurement.  The
   hardware must provide a repeatable support.

## What not to do

- Do not connect a 5 V I2S output directly to the ESP32.
- Do not power the photodiode/TIA from the ESP32 3.3 V pin without filtering.
- Do not run the laser at high power "to get more signal" without checking
  saturation and eye safety.
- Do not use the ESP32 internal ADC for the final instrument.
- Do not claim a schematic exists until it is drawn, reviewed and tested.
