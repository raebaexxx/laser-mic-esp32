# Calibration and measurement procedure

The classifier is **relative**: it compares a capture with a healthy reference
profile of the same slab, sensor position and support.  A bad reference makes
every later result meaningless.  This procedure is written for the first
hardware bring-up and for the course demo.

## 1. Before the first measurement

1. Verify the analog rails and the ADC clocks (see
   [`hardware.md`](hardware.md#9-bring-up-checklist)).
2. Confirm the host receives framed packets:
   ```bash
   cargo run -p granite-host --bin granite-host -- listen --duration 5 --out out/bringup
   ```
3. Tap the slab lightly and check the report:
   - events detected;
   - trigger SNR > 20 dB;
   - clipping < 0.1%;
   - at least three stable modes.
4. If no events appear, lower `event_threshold_sigma` or increase the analog
   gain.  If events are clipped, reduce the gain.

## 2. Support and marking

- Choose a repeatable support: foam pads at the four corners, soft suspension,
  or a rigid frame.  Write it down.
- Mark the impact point and the laser spot on the slab with a small dot or
  tape.  The mode shape depends on both positions.
- Keep the slab, sensor and laser at the same temperature for at least
  30 minutes before the reference capture.
- Remove loose dust from the laser spot; a dust particle moving in the beam
  adds noise.

## 3. Laser alignment

1. Start with the laser at low power and a beam dump.
2. Aim at the retroreflective marker or the painted spot.
3. Focus the spot to 0.5–2 mm.
4. Place the detector so the scattered/retroreflected light fills the active
   area without saturating the TIA.
5. Check the DC level at the TIA output:
   - near mid-supply, not railed;
   - no 50/100 Hz domination;
   - no visible clipping on a scope.
6. Block the laser and measure the noise floor; unblock and compare.  The
   laser-on noise should be clearly above the electronics floor, but the signal
   should not be dominated by ambient light.
7. Gently tap the slab and look at the host waveform.  A clean decaying
   multi-mode response is the goal.

## 4. Gain setting

Set the gain so the strongest tap peaks around **−6 dBFS** (about 0.5 in
normalized units), with no clipping.

- If the host reports clipping > 0.1%, reduce the gain.
- If the trigger SNR is < 20 dB, increase the laser return (marker, focus,
  aperture) before increasing electronic gain; gain also amplifies noise.
- Do not change the gain between the reference and the test capture.  If you
  must, rebuild the reference.

## 5. Tap protocol

Use a consistent hammer and technique:

1. Use a hard plastic or brass tip.
2. Tap once, firmly, at the marked point.
3. Wait at least 1 second before the next tap; the default capture expects
   separated events.
4. Do not double-tap.  A bounce within 150 ms is rejected by the refractory
   window, but it still contaminates the response.
5. Capture 3–5 taps per measurement.  The host averages modal estimates across
   events and requires a mode to appear in at least half of them.
6. Keep the tap energy similar between the reference and the test.  The
   classifier normalizes amplitude, but very weak taps have poor SNR.

A typical capture is 10 s with 5 taps spaced ~1.5 s apart.

## 6. Reference capture

The reference must be a **pristine** slab of the same geometry and material:

```bash
cargo run -p granite-host --bin granite-host -- listen \
  --duration 10 --out out/reference-capture
```

Then:

```bash
cargo run -p granite-host --bin granite-host -- reference \
  --wav out/reference-capture/capture.wav \
  --out reference.txt \
  --name "Granite slab A, foam corner support, sensor at (0.18,0.14)" \
  --length 0.600 --width 0.400 --thickness 0.020 --material granite
```

The command prints the reference modes.  Check:

- at least 3 modes;
- frequency standard deviation < 1% for each mode;
- damping ratio plausible for granite (roughly 0.001–0.01, depending on
  support and frequency);
- no clipping or packet-loss warnings;
- the fundamental is stable across taps.

If the reference looks noisy, capture again.  A reference built from a clipped
or gappy capture will make healthy slabs look defective.

### Averaging multiple reference captures

The current CLI builds a reference from one WAV.  For better repeatability,
capture the pristine slab several times and compare the mode tables.  If the
frequencies agree within ~1%, use the capture with the highest trigger SNR and
the lowest damping uncertainty.  A future version can average profiles
numerically; the profile format is a simple text file and can be merged by
hand if necessary.

## 7. Test capture

Use exactly the same:

- slab;
- support;
- impact point;
- sensor/laser position;
- gain;
- tap technique.

Then:

```bash
cargo run -p granite-host --bin granite-host -- listen \
  --duration 10 --reference reference.txt --out out/test
```

Read the report in this order:

1. **Data quality** — clipping, gaps, trigger SNR, mode count.
2. **Assessment flags** — what changed: frequency shift, damping, new mode,
   splitting.
3. **Modal table** — compare each mode with the reference.
4. **Feature comparison** — z-scores show which feature drove the score.
5. **Score/label** — only after the evidence.

## 8. Quality criteria

A capture is suitable for classification when:

| Check | Target |
|---|---|
| Trigger SNR | > 20 dB |
| Clipping | < 0.1% |
| Packet gaps | < 5% |
| Stable modes | ≥ 3 |
| Frequency std | < 1% of mode frequency |
| Damping fit r² | > 0.5 for the dominant modes (reported in events CSV) |
| Events | 3–5 separated taps |

The host's `usable` flag is more permissive, but the report warnings should be
taken seriously.

## 9. Re-baselining

Rebuild the reference when:

- the slab, support, sensor position or laser angle changes;
- the analog gain or ADC changes;
- the firmware sample rate or packet size changes;
- the slab is re-polished, painted or marked differently;
- the ambient temperature changes significantly;
- a different slab of the same nominal geometry is measured.

Do **not** use a reference from one slab to certify another unless you have
shown that their healthy modal parameters are interchangeable.  Granite is
heterogeneous; two slabs of the same dimensions can differ by several percent.

## 10. Controlled defect validation

Before trusting the classifier on a real defect:

1. Measure a pristine slab and save the reference.
2. Introduce a reversible, controlled change:
   - add a small mass at a known point (changes modes in a predictable way);
   - clamp a point to change boundary conditions;
   - make a small saw cut in a scrap slab (not a finished product).
3. Re-measure and check that the score moves in the expected direction.
4. Restore the slab and confirm the score returns to healthy.

A mass or clamp is safer than a crack and still validates the measurement
chain.  Do not damage a customer's slab for a course demo.

## 11. Common problems

| Symptom | Likely cause | Fix |
|---|---|---|
| No packets | wrong host IP/port, WiFi credentials, firewall | check serial log, `ping`, UDP port |
| Packets but no events | gain too low, laser off target, threshold too high | check waveform, raise gain, lower threshold |
| One event per tap but noisy spectrum | speckle, ambient light, loose mount | marker, aperture, shield, rigid mount |
| Many spurious modes | clipping, 50 Hz, mechanical resonance | reduce gain, notch/shield, isolate sensor |
| Damping estimates scattered | short decay, low SNR, neighbouring modes | more taps, better SNR, narrower damping band |
| Healthy slab flagged | reference mismatch or support changed | re-baseline; check positions |
| Defect missed | weak defect, poor SNR, wrong feature | improve SNR, compare modes manually, use more taps |

## 12. Reporting

The HTML report is self-contained and can be opened offline.  For a course
submission, include:

- the reference profile (`reference.txt`);
- the test capture (`capture.wav`);
- the report (`report.html`);
- the raw mode/feature CSVs;
- a note describing the slab, support, sensor position, gain and tap protocol.

Without that metadata, the numbers are not reproducible.
