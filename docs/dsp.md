# DSP and classification

This document describes what the host computes, why the parameters were
chosen, and where the method is fragile.  All code is in
`crates/granite-core/src`.

## 1. Preprocessing

The raw capture is mono `f32` in roughly `[-1, 1]`.

1. **Remove the mean.**
2. **High-pass at 20 Hz** (second-order Butterworth, zero-phase `filtfilt`) to
   remove DC drift and slow thermal movement.
3. **Notch 50 Hz, 100 Hz and 150 Hz** with Q = 10 to suppress mains pickup and
   its first two harmonics.  The notch is applied forward and backward, so it
   does not shift the modal frequencies.
4. **Optional low-pass** if the sensor/ADC band must be restricted.  The
   default is off; the ADC's anti-alias filter and the analysis band limit are
   expected to handle the top end.

`filtfilt` uses reflected padding and runs the biquad forward and backward,
which gives zero phase but doubles the effective order.  The transient at the
start/end of the capture is not used for modal estimates.

## 2. Impact detection

A 2 ms moving RMS envelope is computed from the filtered signal.  The noise
floor is estimated robustly:

```text
median = median(envelope)
sigma  = 1.4826 · MAD(envelope)
threshold = max(median + k·sigma, 3·median, 1e-12)
```

`k = 8` by default; the digital-twin scenarios use `k = 7` to remain sensitive
to weak taps.  Detection is by **threshold crossing**, not by finding every
envelope maximum:

- when the envelope crosses the threshold, search the next 50 ms for the
  envelope peak;
- accept the peak if it is at least the refractory distance (150 ms) from the
  previous impact;
- then skip forward until the envelope has decayed below half the threshold,
  so beat lobes of a decaying multi-mode response do not create extra events;
- keep at most 8 events (6 in the twin).

Each event is cut from `peak - 20 ms` to `peak + 750 ms` (700 ms in the twin),
clamped to the capture.  A minimum length of 50 ms is enforced.  The trigger SNR
is `20·log10(peak_envelope / noise_rms)`, where `noise_rms` is estimated from
the first 50 ms of the capture (which is silence before the first tap in the
recommended procedure).

**Why this matters.** A simple "local maximum above threshold" detector fires
several times on one impact because a multi-mode response beats.  The
threshold-crossing + decay-until-half-threshold rule produces exactly one event
per tap, which is required for mode persistence and averaging.

## 3. Per-impact spectrum

Each event segment is:

1. linearly detrended;
2. multiplied by a **half-Hann** window `w[n] = 0.5 + 0.5·cos(π n/(N-1))`,
   which is 1 at the impact and 0 at the end of the segment;
3. zero-padded to the next power of two at least twice the segment length
   (capped at 262144 samples);
4. transformed with a radix-2 real FFT;
5. converted to a single-sided amplitude spectrum, corrected for the window's
   coherent gain.

The half-Hann window is important: a symmetric Hann window would attenuate the
impact transient at the start of the segment.  The half-Hann taper is matched to
an exponentially decaying response that begins at the window start.

Spectra from all events are normalized to their own maximum and averaged
bin-by-bin.  The average is used for display and global spectral features; the
per-event spectra are used for peak picking and damping.

## 4. Modal peak picking

For each event spectrum:

- search from `f_min` (20 Hz default; the twin uses 40 Hz) to
  `min(0.45·fs, configured maximum)`;
- reject bins within a small tolerance of the 50/100/150 Hz mains notches;
- require the peak to be:
  - at least `max_db - 38 dB` (relative floor);
  - at least `median_db + 6 dB`;
  - a local maximum;
  - **prominent**: at least 4 dB above the higher of the local minima on its
    left and right within a ±max(20 Hz, 3%) window.
- enforce a minimum separation of `max(5 Hz, 0.8%·f)` between selected peaks;
- keep at most 10 modes, then sort by frequency.

The prominence test is what rejects a noise bump sitting on the flank of a
strong mode.  Without it, a weak shoulder can look like mode splitting; the
digital-twin `healthy-noisy` scenario exposed exactly this failure before the
test was added.  The threshold is deliberately only 4 dB because a real
mode-splitting pair can be closely spaced; the prominence check removes
shoulders without demanding a deep valley between every pair.

Peak frequency is refined by parabolic interpolation of the log magnitude
around the FFT maximum.

## 5. Damping estimation

For each peak at frequency `f`:

1. Band-pass the event with a zero-phase biquad centred at `f`.
   - fractional bandwidth `max(0.08·f, 10 Hz)`;
   - `Q = f / bandwidth`, clamped to `[0.5, 80]`.
2. Compute the analytic-signal envelope with a Hilbert transform.
3. Smooth the envelope with a 1 ms moving average.
4. Find the envelope maximum after the first 2 ms.
5. Fit `ln(envelope)` against time from `peak + 2 ms` until the envelope falls
   below 5% of its maximum (or the end of the segment).
6. The slope gives

   ```text
   ζ = -slope / (2π f)
   ```

   and the fit quality is `r²`.
7. Accept the fit if `r² >= 0.35` and `0 < ζ < 0.5`.

If the fit fails, use the **half-power bandwidth** of the band-passed spectrum:

```text
ζ = (f_high - f_low) / (2 f)
```

where `f_low` and `f_high` are the −3 dB points around the peak.  If that also
fails, use a low-confidence default `ζ = 0.005` and mark the method as
`Default`.

The estimate is repeated for every impact, then modes are clustered:

- cluster tolerance = `max(1%·mean_frequency, 4 Hz)`;
- a cluster must appear in at least half of the events (at least 1 event);
- damping is weighted by `amplitude · max(r², 0.1)`;
- frequency and damping standard deviations are recorded;
- persistence = fraction of events in which the cluster appears;
- `Q = 1/(2ζ)`;
- keep the 10 strongest clusters by `amplitude · persistence`, then sort by
  frequency.

**Fragility.** Closely spaced modes contaminate each other's band-passed
envelope.  High damping and short segments reduce the usable fit range.  A
frequency-domain modal fit (rational fraction polynomial or ERA) would be more
robust but much more code; the current method is deliberately transparent.

## 6. Scalar features

`FeatureSet` contains:

| Feature | Definition |
|---|---|
| `fundamental_hz` | lowest dominant stable mode (`amplitude · persistence ≥ 0.15`), falling back to the lowest mode |
| `modal_frequencies_hz`, `modal_damping` | all stable modes |
| `frequency_ratios` | higher mode / fundamental |
| `mean_damping` | persistence-weighted mean damping |
| `max_damping` | largest modal damping |
| `spectral_centroid_hz` | power-weighted mean frequency |
| `spectral_spread_hz` | power-weighted standard deviation around the centroid |
| `spectral_rolloff95_hz` | frequency below which 95% of power lies |
| `spectral_flatness` | geometric mean / arithmetic mean of bin power (0 = tonal, 1 = noise) |
| `spectral_tilt_db_per_octave` | slope of dB power versus log2(f), bins within 60 dB of the peak |
| `decay_t60_s` | `6.9078 / (2π f1 ζ_mean)` |
| `high_freq_energy_ratio` | power above `max(2 f1, 1 kHz)` divided by total power |
| `mode_splitting_score` | closeness/persistence of mode pairs within 3% |
| `peak_amplitude`, `rms`, `crest_factor` | time-domain amplitude statistics |
| `mode_count`, `event_count` | counts |
| `trigger_snr_db` | mean trigger SNR |

Scale-dependent features (`peak_amplitude`, `rms`) are reported but are **not**
used in the reference comparison, because laser alignment and gain vary between
sessions.  The classifier uses relative features and modal parameters.

### Mode splitting score

For every pair of modes within 3% in frequency, with persistence ≥ 0.75 and
`amplitude · persistence ≥ 0.12`:

```text
score = max(score, (1 - spacing/0.03) · min(persistence_a, persistence_b))
```

A crack or delamination that breaks symmetry can produce two close peaks where
the healthy slab had one.  The score is a simple, explainable proxy for that
pattern.

## 7. Reference profile and classifier

### Building a reference

`build_reference` keeps modes with `amplitude · persistence >= 0.12` (falling
back to all modes if fewer than two survive) and at most 8 modes.  For each:

- frequency tolerance = `max(2%·f, 5 Hz)`;
- damping tolerance = `max(25%·ζ, 0.001)`.

Scalar features have per-feature floors, for example:

| Feature | Reference standard deviation floor |
|---|---|
| `fundamental_hz` | `max(2%·value, 5 Hz)` |
| `mean_damping`, `max_damping` | `max(25%·value, 0.001)` |
| `spectral_centroid_hz`, `rolloff95` | `max(5%·value, 10 Hz)` |
| `spectral_flatness` | `max(25%·value, 0.01)` |
| `spectral_tilt_db_per_octave` | 2 dB |
| `decay_t60_s` | `max(20%·value, 0.01 s)` |
| `high_freq_energy_ratio` | `max(20%·value, 0.01)` |
| `mode_splitting_score` | 0.08 |
| `crest_factor` | `max(10%·value, 0.1)` |

The floors prevent a single pristine measurement from producing unrealistically
tight tolerances.

### Matching

For each reference mode, the closest unmatched measured mode within

```text
max(frequency_tolerance, 10%·f_ref, 30 Hz)
```

is accepted.  The frequency z-score is

```text
z_f = |f_measured - f_ref| / frequency_tolerance
```

and the damping z-score is one-sided:

```text
z_ζ = max(0, (ζ_measured - ζ_ref) / damping_tolerance)
```

because a defect is expected to *increase* damping, not decrease it.  A missing
reference mode contributes a fixed penalty of 2.5.  An unmatched measured mode
with `amplitude · persistence >= 0.15` contributes an "unexpected mode" penalty
of 1.5.

Scalar features are compared with one-sided or absolute z-scores:

- damping, high-frequency ratio, splitting: only positive deviations count;
- T60: only *shorter* decay counts (negative z);
- other features: absolute z.

Each mode/metric has a weight; mode frequency has weight 1.0, damping 0.9,
scalar features 0.55, with a mild `1/(1 + 0.35·index)` down-weighting for
higher modes.  The weighted RMS z-score is mapped to an anomaly score:

```text
score = 100 · z_rms / (z_rms + 2.5)
```

This mapping is monotonic and bounded.  It is not a probability.

### Labels and explanations

| Score | Label |
|---:|---|
| `< 30` | `HEALTHY` |
| `30..55` | `SUSPECT` |
| `>= 55` | `DEFECT` |
| no modes | `UNKNOWN` |

The classifier also emits flags such as:

- "modal frequencies shifted X% downward/upward";
- "damping increased N× versus reference";
- "closely spaced modes detected";
- "low-frequency mode at X Hz";
- "reference mode at X Hz was not found";
- data-quality warnings (clipping, gaps, low SNR, fewer than two modes).

Each flag has a short physical interpretation.  The goal is that a user can see
*why* the label was produced, not just the label.

## 8. Data quality

`QualityMetrics` records:

- gap fraction (from reassembly);
- clipping fraction (`|sample| >= 0.999`);
- trigger SNR;
- warnings;
- `usable`.

Warnings are emitted when:

- clipping > 0.1% of samples;
- gaps > 5% of the capture;
- trigger SNR < 10 dB;
- fewer than two stable modes;
- the averaged spectrum is empty;
- no impacts were detected.

`usable` is slightly more permissive than the warning thresholds (clipping
≤ 1%, gaps ≤ 25%) so a capture with a small dropout can still be analysed, but
the warnings remain visible in the report.

## 9. Tuning guide

| Symptom | Likely cause | Adjustment |
|---|---|---|
| No events | threshold too high, gain too low, tap too weak | lower `event_threshold_sigma`, raise analog gain, use a harder tap |
| Many events per tap | beat lobes, threshold too low | raise threshold, increase refractory, shorten post-trigger |
| Missing weak modes | peak floor too high | lower `peak_threshold_db` (e.g. −45), reduce prominence threshold |
| Too many spurious modes | noise, shoulder peaks | raise prominence/floor, increase minimum separation |
| Damping too high | neighbouring modes in band-pass | reduce `damping_bandwidth_fraction`, raise `damping_min_r2` |
| Damping unstable | short decay, low SNR | longer post-trigger, average more taps, improve laser SNR |
| Healthy slab flagged | reference mismatch, support changed | re-measure reference, check sensor position, check boundary conditions |
| Defect missed | severity too low, noise too high | improve SNR, use more taps, compare mode-by-mode in the report |

All thresholds are in `AnalysisConfig` and `ClassifierConfig`, not scattered
through the code.
