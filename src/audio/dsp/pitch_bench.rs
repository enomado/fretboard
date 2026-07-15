//! Raw Pitch Accuracy of the shipped SWIPE′ scorer, measured against a corpus with a
//! **perfect** f0 annotation (MDB-stem-synth).
//!
//! # Why this exists
//!
//! Marttila & Reiss (ISMIR 2025, arXiv:2507.11233) report SWIPE at **96.2 %** RPA on
//! MIR-1K where the self-supervised literature had been citing it as **86.6 %** — a
//! ten-point spread between implementations *of the same algorithm*, caused by nothing
//! more than search range and sampling scale. That number is the reason this file was
//! written before any neural work: our SWIPE′ sits somewhere on that same ten-point
//! spread, and **nobody knows where**. `testdata/`'s five violin takes answer "does the
//! phantom octave still win" (a histogram over one note); they cannot answer "how good is
//! this detector" (a number over many instruments, with ground truth).
//!
//! Without this baseline, training the paper's 647-parameter Toeplitz layer is
//! unfalsifiable: the paper's own tables put its gain over plain SWIPE at **+0.4 points**
//! on clean audio, which is far smaller than the implementation spread above. A gain that
//! size cannot be told apart from an accidentally-fixed frontend unless the frontend was
//! measured first.
//!
//! # What is measured
//!
//! The **production path**, exactly as `dsp::melody` consumes it: real bank → real
//! snapshot → `ResonatorSnapshot::fundamental`. Not a re-implementation. This is the same
//! discipline as `swipe::tests::real_violin_g_probe`, which is why this module drives the
//! analyzer the same way rather than sharing a helper: the probe answers a different
//! question (which *note* wins on one string) and its loop is its own oracle comparison.
//!
//! # Why a test and not a `bin/`
//!
//! `audio::dsp` is private in every build, on purpose — the hole that once let a probe
//! reach in from `app::` was closed deliberately (see `audio/mod.rs`). A benchmark is not
//! a reason to re-open it.
//!
//! # Running it
//!
//! ```text
//! cargo test --lib swipe_rpa_on_corpus -- --ignored --nocapture
//! ```
//!
//! Needs `datasets/MDB-stem-synth/` (~5 GB, git-ignored, fetched on demand) — see
//! `docs/pitch_benchmark.md`.

use std::collections::BTreeMap;
use std::path::{
    Path,
    PathBuf,
};

use crate::audio::dsp::resonator::ResonatorAnalyzer;
use crate::core_types::note::AccidentalStyle;

/// mir_eval's raw-pitch-accuracy threshold, and the one behind every number in the
/// paper's tables: a voiced frame is correct when the estimate lands within 50 cents of
/// the truth. Reproduced here rather than tuned — the whole point of the metric is that
/// it is the *same* metric the literature reports.
const RPA_TOLERANCE_CENTS: f32 = 50.0;

/// A semitone in cents. An octave slip is 1200 of them; `RPA_TOLERANCE_CENTS` is half of
/// this constant, which is why "correct" and "octave error" can never overlap.
const CENTS_PER_OCTAVE: f32 = 1200.0;

/// The bank's publish cadence — the rate `MelodyTracker` is contractually driven at, so
/// scoring at any other hop would measure a detector the app does not run.
///
/// Also, by coincidence, `lars76/pitch-benchmark`'s frame rate for every dataset
/// (256 samples at 16 kHz). That is what lets an external trajectory dumped by
/// `tools/swiftf0_oracle.py` be compared to the bank frame-for-frame — see
/// `external_rpa_on_corpus`.
const BANK_HOP_SECONDS: f32 = 0.016;

/// The band `lars76/pitch-benchmark` scores MDB-stem-synth in — `PitchDatasetMDBStemSynth`
/// declares `fmin = 65, fmax = 2093`, overriding its own 46.875–2093.75 Hz default.
///
/// Not SwiftF0's range, and not a number to re-derive from the model's spec: it is the
/// reference's arbitrary choice for this corpus, and reproducing their number requires
/// reproducing their band. Applied to the **annotation**, not to the estimate.
const REFERENCE_MDB_MASK_HZ: (f32, f32) = (65.0, 2093.0);

/// Per-track verdict tally. Counts, not rates: rates are derived at print time so a
/// per-track and a corpus-wide figure come from the same arithmetic.
#[derive(Default, Clone)]
struct Tally {
    /// Every miss, bucketed by its **rounded interval in semitones** (estimate − truth).
    ///
    /// The first run of this benchmark is the reason this field exists: it reported 1.5 %
    /// octave errors and **14.7 % "other"**, i.e. the failure this pipeline has spent its
    /// life fighting is not the failure it actually has. "Other" is not a diagnosis — an
    /// error at +7 (a fifth, i.e. the 3rd harmonic crowned) argues for a kernel fix, one
    /// at −0.7 (a smear) argues for the frontend, and one at +19 argues for something
    /// else entirely. So name the interval instead of lumping it.
    error_intervals: BTreeMap<i32, u32>,
    /// Frames the annotation calls voiced. The denominator of RPA.
    voiced:          u32,
    /// Voiced frames scored within `RPA_TOLERANCE_CENTS`.
    correct:         u32,
    /// Voiced frames off by (within tolerance of) a whole number of octaves.
    ///
    /// Broken out rather than lumped into "wrong" because on this app's instrument an
    /// octave is not automatically an error — `testdata/g_open_real_octave.wav` is the
    /// player actually *playing* the octave. Here, against a synthesised ground truth,
    /// an octave slip genuinely is a miss; the split exists so the failure *shape* is
    /// visible, since an octave miss and a rubbish miss argue for different fixes
    /// (a Viterbi can outvote the first and cannot invent a cure for the second).
    octave:          u32,
    /// Voiced frames where the scorer returned no pitch at all.
    silent_miss:     u32,
    /// Voiced frames that were neither correct, an octave, nor silent.
    other:           u32,
    /// Frames the annotation calls unvoiced where the scorer nonetheless crowned a pitch.
    /// Not part of RPA (which is voiced-only) — reported because a detector that hears
    /// notes in silence is a different problem than one that mis-hears them.
    false_voiced:    u32,
    /// Frames the annotation calls unvoiced and the scorer agreed. Denominator partner
    /// for `false_voiced`.
    unvoiced:        u32,
    /// Sum of |cents error| over `correct` frames — fine-pitch quality *given* the note
    /// was right. Averaged at print time.
    correct_cents:   f32,
}

impl Tally {
    fn merge(&mut self, other: &Tally) {
        self.voiced += other.voiced;
        self.correct += other.correct;
        self.octave += other.octave;
        self.silent_miss += other.silent_miss;
        self.other += other.other;
        self.false_voiced += other.false_voiced;
        self.unvoiced += other.unvoiced;
        self.correct_cents += other.correct_cents;
        for (interval, count) in &other.error_intervals {
            *self.error_intervals.entry(*interval).or_default() += count;
        }
    }

    /// RPA as mir_eval defines it, and as every number in the literature's tables means
    /// it: correct frames over **every** voiced frame of the annotation. A frame the
    /// detector went silent on is a miss, not an absentee.
    fn rpa(&self) -> f32 {
        100.0 * self.correct as f32 / self.voiced.max(1) as f32
    }

    /// RPA as `lars76/pitch-benchmark` computes it — **a different metric with the same
    /// name**, and the only one their published tables can be compared against.
    ///
    /// Their `run_single_evaluation` builds the denominator as `pred_voicing &
    /// true_voicing`, so a voiced frame the detector answered with silence is dropped
    /// from the denominator instead of counted wrong. That is accuracy *conditional on
    /// the detector having spoken*; their harmonic mean puts the missing penalty back via
    /// a separate voicing-recall term, so the composite is honest even though this
    /// component, read alone, flatters.
    ///
    /// Derived rather than tallied: `voiced - silent_miss` **is** their denominator, since
    /// `silent_miss` counts exactly the frames where truth is voiced and the estimate is
    /// absent. No second scoring path, so the two conventions can never disagree about
    /// anything except the division.
    fn rpa_conditional(&self) -> f32 {
        100.0 * self.correct as f32 / (self.voiced - self.silent_miss).max(1) as f32
    }
}

/// The band an annotation is scored in.
///
/// # Why truth is rewritten rather than dropped
///
/// `lars76/pitch-benchmark` handles a corpus whose annotation strays outside the band an
/// algorithm claims by keeping the pitch and zeroing its *periodicity*
/// (`PitchDataset._validate_pitch`, with their default `clip_pitch=False`) — i.e. the
/// frame stops being voiced but keeps participating, and a detector that crowns a pitch
/// there is still charged a false positive. Dropping the frame outright would be a third
/// convention, matching neither them nor us.
///
/// So this maps out-of-band truth onto `None`, which is already how this module spells
/// "unvoiced", and `score_frame` needs no new branch to honour it.
struct PitchMask {
    lo_midi: f32,
    hi_midi: f32,
}

impl PitchMask {
    /// Everything is in band — what the bank's own benchmark has always done implicitly.
    fn unbounded() -> Self {
        PitchMask {
            lo_midi: f32::NEG_INFINITY,
            hi_midi: f32::INFINITY,
        }
    }

    fn from_hz(lo_hz: f32, hi_hz: f32) -> Self {
        assert!(lo_hz < hi_hz, "mask {lo_hz}..{hi_hz} Hz is empty");
        PitchMask {
            lo_midi: hz_to_midi(lo_hz),
            hi_midi: hz_to_midi(hi_hz),
        }
    }

    /// Out-of-band truth becomes unvoiced truth. Applied to the **annotation** only: the
    /// estimate is left exactly as its estimator emitted it, because clamping an estimate
    /// into the band would score a pitch nobody predicted.
    fn admit(&self, truth: Option<f32>) -> Option<f32> {
        truth.filter(|midi| *midi >= self.lo_midi && *midi <= self.hi_midi)
    }
}

/// The annotation, resampled to nothing — kept exactly as the corpus wrote it.
///
/// `midi[i]` is `None` where the corpus wrote frequency 0.0, which is its encoding for an
/// unvoiced frame. Storing `Option` rather than a 0.0 sentinel is deliberate: a 0.0 Hz
/// "pitch" silently converts to −inf MIDI and would poison any arithmetic it touched.
struct Annotation {
    /// Annotation timestamps in seconds, ascending (the corpus writes a uniform 2.9 ms
    /// grid, but nothing here assumes that — `at` binary-searches).
    times: Vec<f32>,
    midi:  Vec<Option<f32>>,
}

impl Annotation {
    /// The annotated pitch nearest in time to `seconds`.
    ///
    /// Nearest-neighbour, not interpolation: between two annotation frames the truth is a
    /// real trajectory we have no model for, and inventing one would put made-up numbers
    /// in the denominator of a benchmark. The corpus grid (2.9 ms) is ~5× finer than the
    /// bank's hop (16 ms), so the nearest sample is never more than 1.5 ms away.
    fn at(&self, seconds: f32) -> Option<f32> {
        if self.times.is_empty() {
            return None;
        }
        let after = self.times.partition_point(|&t| t < seconds);
        // Clamp to the ends, then pick whichever neighbour is closer.
        let index = if after == 0 {
            0
        } else if after >= self.times.len() {
            self.times.len() - 1
        } else {
            let before = after - 1;
            if (seconds - self.times[before]) <= (self.times[after] - seconds) {
                before
            } else {
                after
            }
        };
        self.midi[index]
    }
}

/// Hz → fractional MIDI. The corpus speaks Hz; the bank speaks MIDI.
fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Parse a `time,frequency` CSV. Frequency 0.0 means unvoiced.
///
/// No header in this corpus, so a line that does not parse as two floats is a corpus
/// problem, not a variant to tolerate — hence `unwrap`.
fn load_annotation(path: &Path) -> Annotation {
    let text = std::fs::read_to_string(path).unwrap();
    let mut times = Vec::new();
    let mut midi = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (t, f) = line.split_once(',').unwrap();
        let hz: f32 = f.trim().parse().unwrap();
        times.push(t.trim().parse().unwrap());
        // 0 Hz is the corpus's unvoiced marker. Anything else is a real f0.
        midi.push((hz > 0.0).then(|| hz_to_midi(hz)));
    }
    Annotation { times, midi }
}

/// Drive the real bank over one track and score every published frame at **every** lag in
/// `lags_seconds`, returning one `Tally` per lag.
///
/// # Why a sweep rather than a parameter
///
/// Running the bank is ~99 % of the cost here (a per-sample IIR over ~670 resonators);
/// consulting the annotation is a binary search. So N lags cost N× nothing, while N
/// separate runs would cost N× three hours. The sweep is the cheap shape.
///
/// # Why lag exists at all
///
/// The first run reported ±1-semitone misses as 52 % of all errors, near-symmetrically
/// (+1: 664, −1: 587) — the signature of a *timing* error, since a kernel confusing
/// harmonics would lean one way. It was: shifting the annotation 16 ms into the past
/// moved RPA 83.8 % → 91.0 %, and the curve plateaus by 32 ms, matching the bank's
/// independently-measured 8–29 ms group delay (`resonator::bank_latency_probe`).
///
/// Both ends of the sweep are honest, they just answer different questions:
///
/// - **lag = 0** — how good is the *pipeline*, latency and all. What the player gets.
/// - **lag ≈ group delay** — how good is the *scorer*. The number comparable to the
///   literature, whose FFT-based estimators centre a symmetric window and are therefore
///   delay-compensated by construction. Comparing our IIR at lag 0 to their centred FFT
///   flatters *them*.
fn bench_track(audio: &Path, annotation: &Annotation, mask: &PitchMask, lags_seconds: &[f32]) -> Vec<Tally> {
    let mut reader = hound::WavReader::open(audio).unwrap();
    let sample_rate = reader.spec().sample_rate as f32;
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();

    let mut analyzer = ResonatorAnalyzer::new(sample_rate);
    let hop = (sample_rate * BANK_HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); lags_seconds.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        analyzer.process_samples(&samples[fed..fed + hop], true);
        fed += hop;
        // The frame reflects the bank's state *after* `fed` samples, so that is its
        // timestamp. What the *truth* was at that moment depends on the lag — see the
        // function's docs; the bank is driven once and every lag reads off the same frame.
        let seconds = fed as f32 / sample_rate;
        let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);

        for (tally, lag) in tallies.iter_mut().zip(lags_seconds) {
            score_frame(
                tally,
                mask.admit(annotation.at(seconds - lag)),
                snapshot.fundamental,
            );
        }
    }
    tallies
}

/// Score a trajectory this pipeline did not produce — a CSV from some other estimator —
/// through the **same** `score_frame`, the same lag sweep, and the same `Tally` the bank's
/// own frames go through.
///
/// # Why the harness needs this at all
///
/// Every number this module has ever printed was produced by code that has never been
/// checked against anything outside this repository. `testdata/`'s violin takes and
/// MDB-stem-synth both ask "is the detector right"; neither asks "is the *scorer* right",
/// and a benchmark that errs in its own favour is worse than no benchmark. Feed it a
/// trajectory whose verdict is already known from a reference implementation, and the
/// scorer is the only unknown left in the equation.
///
/// `tools/swiftf0_oracle.py` produces the trajectory (SwiftF0, driven by
/// `lars76/pitch-benchmark` at a pinned commit) and records that harness's verdict on it
/// in `run.json`. See `external_rpa_on_corpus` for what agreement is required.
///
/// # Why the estimate's own timestamps are the grid
///
/// The external file *is* the frame grid: its rows are scored where they sit, not
/// resampled onto the bank's hop. Resampling an estimate invents pitches its estimator
/// never emitted — and at a 50-cent tolerance an invented pitch is not a rounding
/// difference, it is a different answer. That the two grids coincide anyway
/// (`BANK_HOP_SECONDS` == their 256/16000 s) is luck worth keeping, not a licence to
/// interpolate.
fn bench_external(
    estimates: &Annotation,
    annotation: &Annotation,
    mask: &PitchMask,
    lags_seconds: &[f32],
) -> Vec<Tally> {
    let mut tallies = vec![Tally::default(); lags_seconds.len()];
    for (index, seconds) in estimates.times.iter().enumerate() {
        // Strength is what the bank reports alongside a pitch and what `score_frame`
        // ignores (`_strength`); an external CSV carries no such thing, so 1.0 is a
        // placeholder for a field with no reader — not a confidence we are asserting.
        let estimate = estimates.midi[index].map(|midi| (midi, 1.0));
        for (tally, lag) in tallies.iter_mut().zip(lags_seconds) {
            score_frame(tally, mask.admit(annotation.at(seconds - lag)), estimate);
        }
    }
    tallies
}

/// Classify one frame's verdict into `tally`. Split out so the lag sweep cannot drift
/// into N slightly-different copies of the same arithmetic.
fn score_frame(tally: &mut Tally, truth: Option<f32>, estimate: Option<(f32, f32)>) {
    match (truth, estimate) {
        (None, None) => tally.unvoiced += 1,
        (None, Some(_)) => {
            tally.unvoiced += 1;
            tally.false_voiced += 1;
        }
        (Some(_), None) => {
            tally.voiced += 1;
            tally.silent_miss += 1;
        }
        (Some(truth), Some((estimate, _strength))) => {
            tally.voiced += 1;
            let cents = (estimate - truth) * 100.0;
            if cents.abs() <= RPA_TOLERANCE_CENTS {
                tally.correct += 1;
                tally.correct_cents += cents.abs();
            } else {
                // Name every miss by its interval before classifying it — a fifth and
                // a smear are both "not an octave" and want opposite fixes.
                *tally
                    .error_intervals
                    .entry((estimate - truth).round() as i32)
                    .or_default() += 1;
                // An octave slip is an error whose *residue* is small: fold the error onto
                // the octave circle and see whether what is left is within tolerance.
                let octaves = (cents / CENTS_PER_OCTAVE).round();
                let residue = cents - octaves * CENTS_PER_OCTAVE;
                if octaves != 0.0 && residue.abs() <= RPA_TOLERANCE_CENTS {
                    tally.octave += 1;
                } else {
                    tally.other += 1;
                }
            }
        }
    }
}

/// The corpus layout, as Zenodo record 1481172 unpacks it.
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasets/MDB-stem-synth")
}

/// Pair every annotation with its audio. Returns `(track_id, wav, csv)`, sorted, so a run
/// is reproducible and two runs are diffable.
fn tracks() -> Vec<(String, PathBuf, PathBuf)> {
    let root = corpus_root();
    let audio_dir = root.join("audio_stems");
    let annotation_dir = root.join("annotation_stems");
    assert!(
        audio_dir.is_dir(),
        "corpus missing: {} — see docs/pitch_benchmark.md",
        audio_dir.display()
    );

    let mut found = Vec::new();
    for entry in std::fs::read_dir(&audio_dir).unwrap() {
        let wav = entry.unwrap().path();
        if wav.extension().is_none_or(|e| e != "wav") {
            continue;
        }
        // `<track>.RESYN.wav` ↔ `<track>.RESYN.csv`
        let stem = wav.file_stem().unwrap().to_str().unwrap().to_string();
        let csv = annotation_dir.join(format!("{stem}.csv"));
        if csv.is_file() {
            found.push((stem, wav, csv));
        }
    }
    found.sort();
    found
}

/// The tracks a run covers, after `PITCH_BENCH_FILTER` (substring of the track id, e.g.
/// `violin`) and `PITCH_BENCH_LIMIT` (max tracks).
///
/// Both exist because the full corpus is 15.5 hours through a per-sample IIR bank — hours
/// of CPU, which is a fine overnight run and a terrible edit-test loop. Default: every
/// track, because the default should be the honest number.
fn selected_tracks() -> Vec<(String, PathBuf, PathBuf)> {
    let filter = std::env::var("PITCH_BENCH_FILTER").unwrap_or_default();
    let limit: usize = std::env::var("PITCH_BENCH_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let selected: Vec<_> = tracks()
        .into_iter()
        .filter(|(id, _, _)| filter.is_empty() || id.to_lowercase().contains(&filter.to_lowercase()))
        .take(limit)
        .collect();
    assert!(!selected.is_empty(), "no tracks matched filter {filter:?}");
    if !filter.is_empty() {
        println!("    filter: {filter:?}");
    }
    selected
}

/// The lag sweep in milliseconds, from `PITCH_BENCH_LAGS_MS` or `default`.
///
/// `0` must stay first and stay present in any default: it is the pipeline's real number
/// and the one `report` leads with.
fn lags_ms_from_env(default: &[f32]) -> Vec<f32> {
    std::env::var("PITCH_BENCH_LAGS_MS")
        .map(|v| v.split(',').map(|p| p.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| default.to_vec())
}

/// The scoring band, from `PITCH_BENCH_MASK_HZ` (`"65,2093"`) or `default`.
fn mask_from_env(default: PitchMask) -> PitchMask {
    match std::env::var("PITCH_BENCH_MASK_HZ") {
        Err(_) => default,
        Ok(spec) => {
            let (lo, hi) = spec.split_once(',').unwrap();
            PitchMask::from_hz(lo.trim().parse().unwrap(), hi.trim().parse().unwrap())
        }
    }
}

/// **The baseline.** RPA of the shipped SWIPE′ over the corpus.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn swipe_rpa_on_corpus() {
    // The default brackets the bank's measured 8–29 ms group delay so the plateau is
    // visible rather than assumed.
    let lags_ms = lags_ms_from_env(&[0.0, 8.0, 16.0, 24.0, 32.0]);
    let lags_seconds: Vec<f32> = lags_ms.iter().map(|ms| ms / 1000.0).collect();
    // Unbounded by default: this measures the detector the app ships, and the app does not
    // stop hearing at 2093 Hz. A band is opt-in, for when a run has to be made comparable
    // to something that has one.
    let mask = mask_from_env(PitchMask::unbounded());

    println!("\n=== SWIPE′ RPA · MDB-stem-synth ===");
    let selected = selected_tracks();
    println!("    {} tracks", selected.len());
    let mut per_lag = vec![Tally::default(); lags_seconds.len()];

    for (id, wav, csv) in &selected {
        let annotation = load_annotation(csv);
        let tallies = bench_track(wav, &annotation, &mask, &lags_seconds);
        for (corpus, tally) in per_lag.iter_mut().zip(&tallies) {
            corpus.merge(tally);
        }
        // Per track, report the sweep as bare RPAs — the shape (does it climb? where does
        // it flatten?) is the per-track story; the breakdown belongs to the corpus.
        let sweep: Vec<String> = tallies.iter().map(|t| format!("{:>5.1}", t.rpa())).collect();
        println!("  {:<48} RPA {}", id, sweep.join(" "));
    }

    report("the pipeline as the player hears it", &lags_ms, &per_lag);
    println!(
        "\n  Reading it: lag 0 is the pipeline (what the player gets). A lag near the\n  \
         bank's 8–29 ms group delay is the *scorer*, and is what the literature's\n  \
         symmetric-window FFT estimators are already compensated for by construction."
    );
    println!(
        "\n  for scale — Marttila & Reiss 2025, Table 1/3 (MDB-stem-synth):\n    \
         SWIPE (their impl, mel) 96.1%   SWIPE-tiny 96.5%   PESTO 94.6%   pYIN 91.6%"
    );
}

/// **The check on the checker.** Score SwiftF0's trajectory with our scorer and require
/// our arithmetic to reproduce the reference harness's verdict on the same trajectory.
///
/// # What this proves, and what it does not
///
/// Not "is SwiftF0 good" — that is their business and already published. This asks the
/// one question `swipe_rpa_on_corpus` structurally cannot ask about itself: **is our
/// scoring code right?** Both harnesses see identical inputs (the same annotation, the
/// same trajectory, the same band), so any disagreement in the verdict is a disagreement
/// about arithmetic, alignment, or the voicing convention — and one of the two is wrong.
///
/// The comparison is against `rpa_conditional`, not `rpa`: theirs is the conditional
/// metric, and holding our own definition up against their number would be comparing two
/// different quantities and calling the difference a bug.
///
/// # Why the gate scores against *their* annotation
///
/// The reference does not score against the corpus's annotation — it scores against its
/// own resampling of it, and that resampling drifts. `PitchDataset.process_sample` maps
/// annotation index `i` onto frame `i·(L−1)/(N−1)` (`align_corners=True`), so for a 171 s
/// track frame `j` reads the truth from `0.0160013·j` seconds while the harness labels it
/// `0.016·j`. The error accumulates to roughly 14 ms — most of a frame — by the end of a
/// long track, and leaves ~5.8 % of voiced frames with a truth more than 50 cents from the
/// corpus's own.
///
/// Scored against the corpus annotation, this test found our figure 3.12 points *above*
/// theirs — not because either scorer is wrong, but because the two were being asked about
/// different truths. So the gate holds the annotation fixed at theirs (`truth/`, dumped by
/// the same script that produced the trajectory) and lets the scorers be the only variable.
/// The corpus-annotation number is reported too, and it is the more accurate of the two —
/// but it cannot arbitrate arithmetic, which is this test's only job.
///
/// # Prerequisite
///
/// ```text
/// uv run tools/swiftf0_oracle.py                    # writes datasets/oracle_swiftf0/
/// cargo test --lib external_rpa_on_corpus -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs datasets/oracle_swiftf0 — run tools/swiftf0_oracle.py first"]
fn external_rpa_on_corpus() {
    let oracle_dir = std::env::var("PITCH_BENCH_EXTERNAL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasets/oracle_swiftf0"));
    let manifest_path = oracle_dir.join("run.json");
    assert!(
        manifest_path.is_file(),
        "no oracle at {} — run `uv run tools/swiftf0_oracle.py` first",
        manifest_path.display()
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();

    // Their band, read from the run that produced these CSVs rather than assumed. If the
    // oracle was generated under a different mask than we score under, the two numbers
    // describe different frame populations and comparing them is meaningless — so this is
    // an assert, not a default.
    let mask_hz = (
        manifest["mask_hz"][0].as_f64().unwrap() as f32,
        manifest["mask_hz"][1].as_f64().unwrap() as f32,
    );
    assert_eq!(
        mask_hz, REFERENCE_MDB_MASK_HZ,
        "oracle was produced under a band this test does not know how to compare in"
    );
    let mask = mask_from_env(PitchMask::from_hz(mask_hz.0, mask_hz.1));

    // Lag 0 is the comparison point: their harness has no lag concept — it scores frame k
    // against the annotation resampled onto frame k — so lag 0 is the only setting that
    // asks them and us the same question. The rest of the sweep is diagnosis: SwiftF0
    // centres its window and is delay-compensated by construction, so a peak *away* from
    // 0 would mean the two grids are offset, not that the model is late.
    let lags_ms = lags_ms_from_env(&[0.0, -16.0, -8.0, 8.0, 16.0]);
    let lags_seconds: Vec<f32> = lags_ms.iter().map(|ms| ms / 1000.0).collect();

    println!("\n=== SwiftF0 (external trajectory) · MDB-stem-synth ===");
    println!(
        "    oracle : {} @ threshold {}",
        manifest["reference_commit"].as_str().unwrap(),
        manifest["optimal_threshold"]
    );
    println!("    audio  : {}", manifest["audio"].as_str().unwrap());
    println!("    mask   : {}-{} Hz (theirs)", mask_hz.0, mask_hz.1);

    let truth_dir = oracle_dir.join(manifest["truth_subdir"].as_str().unwrap());
    assert!(
        truth_dir.is_dir(),
        "oracle at {} predates the truth dump — re-run tools/swiftf0_oracle.py",
        oracle_dir.display()
    );

    let selected = selected_tracks();
    let mut per_lag = vec![Tally::default(); lags_seconds.len()];
    // The gate's tally: their trajectory against their annotation, at lag 0 only. Lag has
    // no meaning here — both sit on the same grid by construction, so `Annotation::at`
    // lands on the exact frame and interpolates nothing.
    let mut gate_tally = Tally::default();
    let mut covered = 0usize;

    for (id, _wav, csv) in &selected {
        let estimates_path = oracle_dir.join(format!("{id}.csv"));
        // A missing CSV is not a failure: the reference skips tracks its annotation calls
        // silent throughout, so those legitimately have no trajectory.
        if !estimates_path.is_file() {
            continue;
        }
        covered += 1;
        // The oracle CSV and the corpus annotation are the same format — `time,frequency`
        // with 0 Hz for unvoiced — which is exactly why `tools/swiftf0_oracle.py` writes
        // that format and these lines need no second parser.
        let estimates = load_annotation(&estimates_path);

        // Pass 1 — the gate: their truth, so the scorers are the only difference left.
        // Unbounded mask, because their `_validate_pitch` already zeroed the periodicity
        // of out-of-band frames before this file was written; masking again would be
        // masking twice.
        let their_truth = load_annotation(&truth_dir.join(format!("{id}.csv")));
        let gate_pass = bench_external(&estimates, &their_truth, &PitchMask::unbounded(), &[0.0]);
        gate_tally.merge(&gate_pass[0]);

        // Pass 2 — the report: the corpus's own annotation, which is the accurate one.
        let annotation = load_annotation(csv);
        let tallies = bench_external(&estimates, &annotation, &mask, &lags_seconds);
        for (corpus, tally) in per_lag.iter_mut().zip(&tallies) {
            corpus.merge(tally);
        }
        let sweep: Vec<String> = tallies.iter().map(|t| format!("{:>5.1}", t.rpa())).collect();
        println!("  {:<48} RPA {}", id, sweep.join(" "));
    }
    assert!(covered > 0, "no oracle CSV matched any selected track");

    report(
        "SwiftF0, scored by our harness on the corpus annotation",
        &lags_ms,
        &per_lag,
    );

    let their_tracks = manifest["tracks"].as_array().unwrap().len();
    let their_rpa = 100.0 * manifest["reference_metrics"]["pitch"]["rpa"].as_f64().unwrap() as f32;
    let gated = gate_tally.rpa_conditional();
    println!("\n=== gate — does our scorer agree with the reference? ===");
    println!("  (both sides scoring the same trajectory against the same annotation)");
    println!("  reference harness  : {their_rpa:.2}%   (their code)");
    println!("  our harness        : {gated:.2}%   (our code)");
    println!("  delta              : {:+.2} points", gated - their_rpa);
    println!(
        "\n  for contrast, our harness on the *corpus* annotation: {:.2}%\n  \
         ({:+.2} points, which is what their annotation's ~14 ms end-of-track drift costs\n  \
         them — not a scorer disagreement)",
        per_lag[0].rpa_conditional(),
        per_lag[0].rpa_conditional() - their_rpa
    );

    // Their aggregate is one number over every track concatenated, so a filtered or
    // truncated run scores a different population and cannot be held to it — say so rather
    // than comparing anyway.
    if covered != their_tracks {
        println!(
            "\n  SUBSET RUN ({covered} of {their_tracks} tracks) — gate skipped. The reference\n  \
             figure is an aggregate over every track; rerun without PITCH_BENCH_FILTER/LIMIT\n  \
             to hold our number to it."
        );
        return;
    }
    // Tight on purpose. With the annotation, the trajectory, the grid and the band all
    // held identical, there is no modelled reason for *any* daylight between the two
    // scorers: the remaining freedom is float accumulation order over ~1.5 M frames. A
    // drift past this is a real disagreement about the metric and is worth stopping for.
    let tolerance = 0.05;
    assert!(
        (gated - their_rpa).abs() <= tolerance,
        "our scorer and the reference disagree by {:+.2} points on the SAME trajectory \
         against the SAME annotation (ours {gated:.2}% vs theirs {their_rpa:.2}%). One of \
         the two is wrong, and until it is found every number this module prints is \
         unattributable.",
        gated - their_rpa
    );
    println!("\n  AGREED within {tolerance} points — our RPA arithmetic reproduces the reference's.");
}

/// Print a corpus verdict and its lag sweep.
///
/// Shared by the bank's benchmark and the external-trajectory one so the two can never
/// drift into reporting slightly different arithmetic under the same column headings —
/// which would defeat the entire point of running an external trajectory through this
/// harness.
fn report(headline: &str, lags_ms: &[f32], per_lag: &[Tally]) {
    // The corpus lines are reported at lag 0 — the pipeline's real number. The sweep
    // below it is what says how much of the gap is timing rather than scoring.
    let corpus = per_lag[0].clone();
    let pct = |n: u32, d: u32| 100.0 * n as f32 / d.max(1) as f32;
    println!("\n=== corpus · lag 0 — {headline} ===");
    println!("  voiced frames      : {}", corpus.voiced);
    println!("  RPA                : {:.2}%   <- the number", corpus.rpa());
    // Printed on every run, next to our own number, so that the one figure the reference
    // publishes is never silently compared against the one we mean. They are different
    // metrics; keeping them adjacent is what stops the confusion from recurring.
    println!(
        "  RPA, their denom.  : {:.2}%   <- lars76/pitch-benchmark's convention (pred ∧ \
         true), NOT mir_eval's",
        corpus.rpa_conditional()
    );
    println!(
        "  octave errors      : {:>5.2}%  ({} frames)",
        pct(corpus.octave, corpus.voiced),
        corpus.octave
    );
    println!(
        "  no pitch on voiced : {:>5.2}%  ({} frames)",
        pct(corpus.silent_miss, corpus.voiced),
        corpus.silent_miss
    );
    println!(
        "  other errors       : {:>5.2}%  ({} frames)",
        pct(corpus.other, corpus.voiced),
        corpus.other
    );
    println!(
        "  mean |cents| when correct: {:.1}",
        corpus.correct_cents / corpus.correct.max(1) as f32
    );
    println!(
        "  pitch in silence   : {:>5.2}%  ({} of {} unvoiced frames)",
        pct(corpus.false_voiced, corpus.unvoiced),
        corpus.false_voiced,
        corpus.unvoiced
    );
    // Where the misses actually are. Sorted by frequency, not by interval: the question
    // this answers is "what is the dominant failure", and the answer should be readable
    // off the first line rather than hunted for in a sorted-by-key table.
    let mut intervals: Vec<(i32, u32)> = corpus.error_intervals.iter().map(|(i, c)| (*i, *c)).collect();
    intervals.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let missed: u32 = intervals.iter().map(|(_, c)| c).sum();
    println!("\n  misses by interval (estimate − truth, semitones):");
    for (interval, count) in intervals.iter().take(12) {
        let label = match interval {
            0 => "  in-bin smear (>50 ¢, <1 semitone)".to_string(),
            12 | -12 => format!("{interval:+3}  OCTAVE"),
            24 | -24 => format!("{interval:+3}  two octaves"),
            7 | -7 => format!("{interval:+3}  fifth (3rd harmonic?)"),
            19 | -19 => format!("{interval:+3}  octave+fifth (3rd harmonic?)"),
            i => format!("{i:+3}"),
        };
        println!(
            "    {label:<38} {count:>7}  ({:>5.2}% of misses, {:>5.2}% of voiced)",
            pct(*count, missed),
            pct(*count, corpus.voiced)
        );
    }
    // The sweep. Reported last because it is the interpretation, not the measurement:
    // how much of the gap to the literature is our scorer being worse, versus our bank
    // answering about a moment that has passed.
    println!("\n=== lag sweep — timing vs scoring ===");
    println!("  lag(ms)    RPA     octave%   other%   ±1 semitone (share of misses)");
    for (lag_ms, tally) in lags_ms.iter().zip(per_lag) {
        let adjacent: u32 = tally.error_intervals.get(&1).copied().unwrap_or(0)
            + tally.error_intervals.get(&-1).copied().unwrap_or(0);
        let missed: u32 = tally.error_intervals.values().sum();
        println!(
            "  {:>5.0}    {:>6.2}%   {:>5.2}%   {:>5.2}%   {:>5.1}%",
            lag_ms,
            tally.rpa(),
            pct(tally.octave, tally.voiced),
            pct(tally.other, tally.voiced),
            pct(adjacent, missed)
        );
    }
}
