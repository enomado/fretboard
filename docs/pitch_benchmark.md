# Pitch benchmark — the number the detector is argued against

`audio::dsp::pitch_bench` measures **Raw Pitch Accuracy (RPA)** of the shipped SWIPE′
scorer against a corpus with a perfect f0 annotation. This file is the operator's manual;
the *why* lives in the module's own doc comment and is summarised below.

Companions: [`note_detection.md`](note_detection.md) (what the pipeline is),
[`pitch_detection_survey.md`](pitch_detection_survey.md) (what the alternatives are),
[`../testdata/README.md`](../testdata/README.md) (the five real violin takes).

## Why a corpus benchmark exists at all

We already have `testdata/` — five takes of a real violin, recorded under a protocol.
They answer **"does the phantom octave still win?"**: a histogram over one note, on the
instrument the app is for. They cannot answer **"how good is this detector?"** — for that
you need many instruments, many registers, and a ground truth that does not depend on
somebody remembering what they played.

The trigger was Table 1 of Marttila & Reiss, ISMIR 2025 ([arXiv:2507.11233](https://arxiv.org/abs/2507.11233)):

| SWIPE, as reported in the self-supervised literature | 86.6 % |
|---|---|
| **SWIPE, their careful implementation** | **96.2 %** |

Ten points of RPA between two implementations *of the same algorithm*, from nothing but
search range and frequency-sampling scale. Our SWIPE′ sits somewhere on that spread and
we do not know where. Every downstream decision — Viterbi, a neural reweighting, a
different frontend — is unfalsifiable until that number exists, because the effects being
chased (the paper's Toeplitz layer buys **+0.4 points** on clean audio) are an order of
magnitude smaller than the uncertainty about our own starting point.

## The corpus

**MDB-stem-synth** — [Zenodo 1481172](https://zenodo.org/records/1481172), 230 solo stems
from MedleyDB, 25 instruments, 15.56 hours. Each stem is **re-synthesised from its own f0
annotation**, which is what makes the annotation *perfect* rather than merely careful —
there is no estimator in the ground-truth path to inherit errors from.

It is the corpus the paper reports on (their Table 3), so our number lands on their axis.

```sh
mkdir -p datasets && cd datasets
curl -L -o MDB-stem-synth.tar.gz \
  "https://zenodo.org/records/1481172/files/MDB-stem-synth.tar.gz?download=1"
tar xzf MDB-stem-synth.tar.gz
```

~1.85 GB compressed, ~5 GB unpacked. `datasets/` is git-ignored — this is third-party
audio fetched on demand, never committed.

Layout: `audio_stems/<track>.RESYN.wav` ↔ `annotation_stems/<track>.RESYN.csv`, the CSV
being `time,frequency` with **frequency 0 meaning unvoiced**.

## Running it

```sh
# the whole corpus — ~3.5 h of CPU (per-sample IIR bank over 15.5 h of audio)
cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# scoped: a quick loop (~1 min/track)
PITCH_BENCH_LIMIT=5 cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# scoped by track id substring (note: ids are *pieces*, not instruments)
PITCH_BENCH_FILTER=NightOwl cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# custom lag sweep; 0 must stay first
PITCH_BENCH_LAGS_MS=0,20,24,28 cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture
```

`--release` is not optional in practice: the bank is a per-sample IIR over ~670 resonators,
and a debug build turns an overnight run into a weekend one. Budget ~5 GB of disk for the
corpus **on top of** `target/`, which for this crate runs to tens of GB — the first run of
this benchmark died on `No space left on device`.

## The lag sweep — the thing this benchmark found

Every frame is scored against the annotation at several lags in **one** pass (running the
bank is ~99 % of the cost; a binary search into the annotation is free). The first run
came back like this **on two tracks** — kept because the whole sweep exists because of it,
and because the corpus went on to move its peak:

| lag (ms) | RPA | ±1-semitone share of misses |
|---|---|---|
| 0 | 83.81 % | 51.8 % |
| 8 | 88.42 % | 44.0 % |
| 16 | 90.97 % | 40.1 % |
| **24** | **91.86 %** | 43.7 % |
| 32 | 91.03 % | 50.5 % |

### The corpus, all 230 tracks (2026-07-15) — these are the numbers

The first full run: 4.7 h, 1.57 M voiced frames, `datasets/bench_runs/`. It is the baseline
any detector change is argued against, and **the two-track probe above was not it** — the
peak moved from 24 ms to **16 ms**, and every figure is a point or two lower.

| lag (ms) | RPA, full band | RPA, 65–2093 Hz |
|---|---|---|
| 0 | 82.99 % | 83.93 % |
| 8 | 87.93 % | 89.07 % |
| **16** | **90.18 %** | **91.02 %** |
| 24 | 89.84 % | 90.11 % |
| 32 | 87.64 % | 87.44 % |

Two tracks were never going to settle a peak — they were 0.9 % of the corpus, and the
sweep's spacing is 8 ms. Quote the corpus; the probe is history, not a second opinion.

Two things fell out of the original table, and only one survived:

1. **Half of our "errors" were timing, not scoring.** ±1-semitone misses were 52 % of all
   errors and near-symmetric (+1: 664, −1: 587) — the signature of a detector chasing a
   moving pitch, not of a kernel confusing harmonics (which would lean one way).
2. ~~**The benchmark independently measured the bank's group delay.**~~ The curve peaked at
   24 ms and fell off both sides, landing inside the 8–29 ms measured by an entirely
   different method (`resonator::bank_latency_probe`). **This reading is wrong — see
   below.** It is struck through rather than deleted because the coincidence was the whole
   reason to believe it, and the next person to find a peak inside an expected range
   deserves to know that this one was not the confirmation it looked like. The corpus then
   put the peak at 16 ms, which is *also* inside 8–29: a 21 ms-wide range accepts almost
   anything, which is the point.

So there are **two honest numbers**, answering different questions:

- **lag 0** — the *pipeline*, latency included. What the player actually gets.
- **lag ≈ 16 ms** (the corpus's peak) — the *scorer*. The number comparable to the
  literature, whose FFT-based estimators centre a symmetric window and are therefore
  delay-compensated by construction. Scoring our IIR at lag 0 against their centred FFT
  flatters *them*.

Neither is the "real" one; quoting only the flattering one would be the lie.

### A lag-sweep peak is not a group delay (2026-07-15)

The SwiftF0 oracle broke claim 2. SwiftF0 peaks at **+9.4 ms** on this corpus — and
SwiftF0 has no group delay to have: it is a centred STFT plus a conv stack, and the
reference resamples its frames onto our grid zero-phase. A peak the *delay-free* estimator
also shows cannot be a measurement of anybody's delay.

`tools/lag_sweep_probe.py` (one command, ~4 min) chases it down. Two attractive answers are
wrong, and both are worth knowing about because each survived a plausible-sounding argument:

- **"SwiftF0 is late."** It is not. Its `CENTER_OFFSET = 127.5` samples is a claim the
  Python cannot enforce — the pad is computed *inside* the ONNX graph — but reading the
  graph out shows a true 384/384 split, so the claim holds. And a vibrato-phase probe
  through the oracle's own path measures the model at **+1.4 ms**, with self-checks that
  recover an imposed 8 ms shift to within 0.01 ms.
- **"The corpus is offset."** It is not. A centred-window YIN at 44.1 kHz — no model, no
  resampling, zero delay by construction — matches MDB-stem-synth's annotation to
  **~1 cent at ~0 ms**. (This one had me: SwiftF0's +9.4 ms looked exactly like a corpus
  offset until an instrument sharing nothing with the first one said otherwise.)

What is actually happening is the **envelope**. Inside a symmetric window straddling a
decaying note, the energy is weighted toward the note's earlier, louder part, so an
energy-weighted estimator reports what the signal was doing slightly in the *past*. It is
late, and the wider its window, the later — with no group delay in it anywhere. The probe
demonstrates it on synthetic audio where the truth is analytic and there is nothing left to
be offset:

| envelope | lag | note |
|---|---|---|
| flat amplitude | **+0.01 ms** | identical pitch trajectory |
| note attack + decay | **+5.11 ms** | *only* the envelope differs |

Real music is made of decaying notes, so **every** estimator on this corpus pays this,
including the bank. A peak is therefore `group_delay + envelope_bias`, and only the first
term belongs to the detector.

What this does and does not change:

- **Does not change the numbers.** Both columns are still the numbers they were, and
  comparing two detectors *each at its own peak* is still fair — the envelope term is
  common to both. The RT-SWIPE gate (`R2`) is unaffected.
- **Does change what the peak means.** The bank's own group delay is `peak − its envelope
  bias`, and that bias is not yet measured — the +5.11 ms above is a *symmetric* 64 ms
  window's, while the bank's window is a causal IIR decay of a different shape and width. So
  the peak (16 ms on the corpus) is an upper bound on the bank's delay, not a measurement of
  it, and "it landed inside 8–29" was never evidence: a 21 ms-wide range accepts almost
  anything.
- **The way to measure the bank's delay** is `resonator::bank_latency_probe`, which drives
  a step and watches the output. That one is a real latency measurement. The corpus sweep
  is not, and should stop being quoted as a second opinion on it.

## What it reports, and why each line is there

- **RPA** — voiced frames scored within 50 cents. *The* number; the same metric
  (`mir_eval`'s) the literature reports, reproduced rather than tuned.
- **octave errors** — errors whose residue on the octave circle is within tolerance.
  Split out because an octave miss and a rubbish miss argue for **different fixes**: a
  Viterbi can outvote a lone octave excursion, but it cannot invent a candidate the
  salience never proposed. (On this corpus an octave *is* an error. On `testdata/` it is
  not necessarily — `g_open_real_octave.wav` is the player actually playing the octave.
  See the memory note on that trap.)
- **no pitch on voiced** — the scorer declined a frame that has a pitch.
- **other errors** — neither correct, octave, nor silent. Where the next bug hides.
- **mean |cents| when correct** — fine-pitch quality *given* the note was right.
- **pitch in silence** — pitch crowned where the annotation says unvoiced. Not part of
  RPA (voiced-only), reported because hearing notes in silence is a different defect from
  mis-hearing them.

## The external oracle — the check on the checker

Everything above is our code measuring our detector. Until 2026-07-15 no number this
harness printed had ever been compared to anything outside this repository, which is a
problem no amount of internal care fixes: a benchmark that is wrong *in its own favour*
is worse than no benchmark.

So we run a reference implementation, feed its trajectory through our scorer, and require
the two harnesses to agree.

```sh
uv run tools/swiftf0_oracle.py       # ~10 min, writes datasets/oracle_swiftf0/
cargo test --release --lib external_rpa_on_corpus -- --ignored --nocapture
```

`tools/swiftf0_oracle.py` drives **SwiftF0** (MIT, 398 KB ONNX, ranked #1 in
[`lars76/pitch-benchmark`](https://github.com/lars76/pitch-benchmark)) through *that
repository's own harness*, pinned at commit `87982db2` and cloned on demand into the
git-ignored `datasets/reference/`. Their dataset loader, their algorithm wrapper, their
metric — imported, never reimplemented, because the entire point is to be checked by code
we did not write.

**Result: our scorer reproduces theirs exactly — 91.98 % vs 91.98 %, a delta of +0.00
points over 1.26 M frames.** Our RPA arithmetic is not the weak link in any number here.

Getting to that comparison meant establishing four things about their numbers first, each
of which independently breaks the naive "does our RPA match their published RPA?" check:

1. **Their `rpa` is not RPA.** `run_single_evaluation` builds the denominator as
   `pred_voicing & true_voicing`: a voiced frame the detector answered with *silence* is
   dropped from the denominator rather than counted wrong. That is accuracy conditional on
   the detector having spoken. mir_eval — and `Tally::rpa` — divide by every voiced frame.
   Their composite is still honest; it re-applies the penalty through a separate
   voicing-recall term. But the component, read alone, flatters. `Tally::rpa_conditional`
   exists solely to speak their dialect, and the summary prints both side by side.
2. **Their published 92.0 % for MDB-stem-synth is not RPA either** — it is a harmonic mean
   of six metrics (RPA, cents accuracy, voicing precision/recall, octave accuracy, gross
   error accuracy). Per-dataset RPA is never published; the only RPA they report (0.905)
   is aggregated across all eight datasets. That 90.2 % ≈ 0.905 coincidence is a trap.
3. **Their published numbers are measured on noise-augmented audio.** `pitch_benchmark.py`
   unconditionally wraps the corpus in `CHiMeNoiseDataset` — CHiME-Home plus Gaussian
   noise, SNR 10–30 dB, random voice gain ±6 dB. There is no clean path through their
   runner. Our oracle script skips the wrapper (its single deliberate deviation), so our
   93.96 % harmonic mean on clean audio sits about two points above their noisy 92.0 %,
   in the direction and of the size one would expect.
4. **Their threshold is oracle-tuned per dataset** — swept 0…1 in steps of 0.1, best
   kept. A ceiling, not a production setting. (0.9 wins on clean MDB, consistent with the
   0.887 baked into their wrapper.)

### The reference's annotation drifts — and it costs them 4.27 points

The gate scores against **their** resampled annotation (`datasets/oracle_swiftf0/truth/`),
not the corpus's. That is not a detail; it is what makes the comparison decidable.

`PitchDataset.process_sample` resamples the 2.9 ms annotation onto the frame grid with
`F.interpolate(mode="linear", align_corners=True)`. `align_corners` maps annotation index
`i` onto frame `i·(L−1)/(N−1)`, so on a 171-second track frame `j` reads its truth from
`0.0160013·j` seconds while the harness labels that frame `0.016·j`. The error is tiny per
frame and accumulates linearly: **~14 ms by the end of a long track, most of a whole
frame.** On 5.79 % of voiced frames their truth ends up more than 50 cents from the
corpus's own (worst case 946 cents), and every one of the worst frames sits in the tail of
its track.

Scored against the corpus annotation, SwiftF0 gets **96.25 %** by their own metric; against
their drifted annotation, **91.98 %**. The 4.27-point gap is not a scorer disagreement —
it is the reference marking a correct answer wrong because it is asking about the wrong
moment. (The first cut of this gate scored against the corpus annotation and reported our
number 3.12 points "above" theirs, which is how the drift was found. The hypothesis that
linear interpolation was blending the 0 Hz unvoiced markers into real pitches was tested
and rejected: only 18 of 389 corrupted frames sit on a voiced/unvoiced boundary.)

So the harness holds the annotation fixed at theirs and lets the scorers be the only
variable. Our own numbers keep using the corpus annotation, which is the accurate one.

### What the oracle says about SwiftF0 itself

Incidentally — not the point of the exercise, but the map for [the RT-SWIPE
track](pitch_detection_survey.md):

| | RPA (mir_eval convention, corpus annotation) |
|---|---|
| SwiftF0, lag 0 | 87.87 % |
| SwiftF0, lag +8 ms | **89.54 %** |

The sweep peaking at +8 ms — half a hop — rather than 0 is worth a look before SwiftF0 is
used to arbitrate anything about timing.

The gap between that and its 96.25 % under their conditional metric is one number:
**SwiftF0 declines 8.70 % of voiced frames outright**. Under mir_eval those are misses;
under theirs they vanish from the denominator. Same trajectory, nine points of headline.

## What it deliberately does not do

- **No annotation interpolation.** Nearest neighbour only. The corpus grid (2.9 ms) is
  ~5× finer than the bank's hop (16 ms), so the nearest sample is within 1.5 ms; inventing
  a trajectory between annotations would put made-up numbers in a benchmark's denominator.
- **No re-implementation.** It drives the production path — real bank, real snapshot,
  `ResonatorSnapshot::fundamental` — the same discipline as `real_violin_g_probe`.
- **No silent lag correction.** The sweep reports lag 0 first and leads the summary with
  it. A single delay-compensated figure would be a better-looking number for a detector
  nobody runs.

## For scale (Marttila & Reiss 2025, Tables 1/3/5, MDB-stem-synth unless noted)

| Method | RPA | at 0 dB SNR (MIR-1K) | at −10 dB (MIR-1K) |
|---|---|---|---|
| SWIPE (their impl) | 96.1 % | 91.2 % | 75.2 % |
| SWIPE-tiny (647 params) | 96.5 % | 95.3 % | 88.5 % |
| PESTO (28.9k) | 94.6 % | 92.9 % | 81.7 % |
| pYIN | 91.6 % | 95.1 % | 85.8 % |

Read the noise columns before reaching for the network: on clean audio the 647-parameter
Toeplitz layer is worth +0.4 points over plain SWIPE. Its real purchase is robustness.
