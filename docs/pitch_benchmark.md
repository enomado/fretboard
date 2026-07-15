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
came back like this on two tracks:

| lag (ms) | RPA | ±1-semitone share of misses |
|---|---|---|
| 0 | 83.81 % | 51.8 % |
| 8 | 88.42 % | 44.0 % |
| 16 | 90.97 % | 40.1 % |
| **24** | **91.86 %** | 43.7 % |
| 32 | 91.03 % | 50.5 % |

Two things fell out of that table:

1. **Half of our "errors" were timing, not scoring.** ±1-semitone misses were 52 % of all
   errors and near-symmetric (+1: 664, −1: 587) — the signature of a detector chasing a
   moving pitch, not of a kernel confusing harmonics (which would lean one way).
2. **The benchmark independently measured the bank's group delay.** The curve peaks at
   24 ms and falls off both sides, landing inside the 8–29 ms measured by an entirely
   different method (`resonator::bank_latency_probe`).

So there are **two honest numbers**, answering different questions:

- **lag 0** — the *pipeline*, latency included. What the player actually gets.
- **lag ≈ 24 ms** — the *scorer*. The number comparable to the literature, whose FFT-based
  estimators centre a symmetric window and are therefore delay-compensated by
  construction. Scoring our IIR at lag 0 against their centred FFT flatters *them*.

Neither is the "real" one; quoting only the flattering one would be the lie.

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
