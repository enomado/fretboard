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
mod transitions;
use std::path::{
    Path,
    PathBuf,
};

use crate::audio::dsp::analysis_math::slide_in;
use crate::audio::dsp::peak_comb::{
    self,
    PeakComb,
};
use crate::audio::dsp::pyin::PitchTracker;
use crate::audio::dsp::resonator::ResonatorAnalyzer;
use crate::audio::dsp::rtswipe::RtSwipe;
use crate::audio::dsp::sieve::{
    HarmonicSieve,
    ScoringVariant,
};
use crate::audio::dsp::swipe::tracked_bins;
use crate::audio::dsp::yinfft::{
    self,
    YinFft,
};
use crate::audio::sample_rate::SampleRate;
use crate::audio::test_rig::Rig;
use crate::audio::types::{
    PitchFrontend,
    SalienceHeat,
};
use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::Hz;

/// A4, for every frequency↔MIDI conversion in this module.
///
/// Shared rather than spelled out per call site because it has to match in three places at
/// once — [`load_annotation`] (which reads the annotation), the bank's default grid
/// (`resonator`'s `reference_hz`) and [`RtSwipe::new`]'s. A frontend put on a grid the
/// annotation is not on would lose cents to arithmetic and blame the detector.
const CONCERT_PITCH_HZ: Hz = Hz::A4_STANDARD;

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

/// pYIN's analysis window as `audio::core` feeds it: 6144 samples, whatever the rate.
const PYIN_WINDOW: usize = 6144;

/// pYIN's cadence, `core::ANALYSIS_INTERVAL` — the frame its HMM is calibrated per.
const PYIN_HOP_SECONDS: f32 = 0.040;

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
    /// The «calm» metrics — what the user sees on the roll and RPA does not: see [`Calm`].
    calm:            Calm,
    transitions:     transitions::Diagnostics,
}

/// What one frame was scored as. RPA only needs the counts; [`Calm`] needs the sequence.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Unvoiced,
    FalseVoiced,
    SilentMiss,
    Correct,
    Octave,
    Other,
}

/// The previous frame of the same track under the same scoring — the pairs and runs
/// [`Calm`] counts are made of.
#[derive(Clone, Copy)]
struct PrevFrame {
    seconds:  f32,
    truth:    Option<f32>,
    estimate: Option<f32>,
    verdict:  Verdict,
}

/// H1's standing-note threshold, applied to every adjacent pair of annotation frames
/// between the two estimates, on the annotation's native grid (2.9 ms for MDB).
/// The threshold must not silently become stricter for detectors with a longer hop.
const NOTE_STANDS_CENTS: f32 = 5.0;

/// Resolution of the jitter histogram, cents. Fine enough for a median of a few cents.
const JITTER_BIN_CENTS: f32 = 0.1;

/// «Calm» metrics of H1 (`docs/detector_hypotheses_plan.md`): three things a user sees on a
/// pianoroll that RPA does not distinguish.
///
/// Counted in the same single pass as RPA, from consecutive frames, so a `Tally` is a
/// per-track object while it is being filled (`prev` belongs to one track) and a corpus
/// total after [`Tally::merge`], which sums everything but `prev`.
#[derive(Default, Clone)]
struct Calm {
    prev:           Option<PrevFrame>,
    /// Maximal runs of consecutive octave-error frames. Ten frames in one run read as one
    /// outlier on the roll, ten scattered frames as ten — RPA counts both as ten.
    octave_events:  u32,
    /// Total length of those runs, for the mean run length.
    octave_seconds: f32,
    /// Audio the annotation calls voiced — the «per minute» of `octave_events`.
    voiced_seconds: f32,
    /// |Δ estimate| between consecutive frames where the note stands
    /// ([`NOTE_STANDS_CENTS`]) and both frames are correct, binned by [`JITTER_BIN_CENTS`].
    jitter:         BTreeMap<u32, u32>,
    /// Independent voiced runs of the truth and estimate, each restricted to the scoring
    /// band. False voicing can join two true runs or add a spurious run; neither is hidden.
    truth_runs:     u32,
    estimate_runs:  u32,
}

impl Calm {
    fn follow(&mut self, frame: PrevFrame, mask: &PitchMask, truth_stands: bool) {
        let prev = self.prev.replace(frame);

        let in_truth = |f: &PrevFrame| f.truth.is_some();
        let in_estimate = |f: &PrevFrame| mask.admit(f.estimate).is_some();
        if in_truth(&frame) && !prev.as_ref().is_some_and(in_truth) {
            self.truth_runs += 1;
        }
        if in_estimate(&frame) && !prev.as_ref().is_some_and(in_estimate) {
            self.estimate_runs += 1;
        }
        if frame.verdict == Verdict::Octave && prev.is_none_or(|p| p.verdict != Verdict::Octave) {
            self.octave_events += 1;
        }

        // A frame owns the audio interval ending at its timestamp. Benches publish their
        // first frame after one hop; an external trajectory starting at t=0 owns zero then.
        let dt = frame.seconds - prev.map_or(0.0, |p| p.seconds);
        if in_truth(&frame) {
            self.voiced_seconds += dt;
        }
        if frame.verdict == Verdict::Octave {
            self.octave_seconds += dt;
        }
        let Some(prev) = prev else { return };
        if truth_stands && frame.verdict == Verdict::Correct && prev.verdict == Verdict::Correct {
            // Both correct ⇒ both estimates exist. Selection is on the annotation grid;
            // the reported movement remains between the detector's own neighbours.
            let moved = (frame.estimate.unwrap() - prev.estimate.unwrap()).abs() * 100.0;
            *self.jitter.entry((moved / JITTER_BIN_CENTS) as u32).or_default() += 1;
        }
    }

    fn merge(&mut self, other: &Calm) {
        self.octave_events += other.octave_events;
        self.octave_seconds += other.octave_seconds;
        self.voiced_seconds += other.voiced_seconds;
        self.truth_runs += other.truth_runs;
        self.estimate_runs += other.estimate_runs;
        for (bin, count) in &other.jitter {
            *self.jitter.entry(*bin).or_default() += count;
        }
    }

    fn octave_events_per_minute(&self) -> f32 {
        self.octave_events as f32 / (self.voiced_seconds / 60.0).max(f32::EPSILON)
    }

    fn mean_octave_run_ms(&self) -> f32 {
        1000.0 * self.octave_seconds / self.octave_events.max(1) as f32
    }

    /// The `q`-quantile of the jitter histogram, in cents (upper edge of its bin), or
    /// `None` when no pair qualified.
    fn jitter_quantile(&self, q: f32) -> Option<f32> {
        let total: u32 = self.jitter.values().sum();
        if total == 0 {
            return None;
        }
        let rank = (q * total as f32).ceil().max(1.0) as u32;
        let mut seen = 0;
        for (bin, count) in &self.jitter {
            seen += count;
            if seen >= rank {
                return Some((*bin + 1) as f32 * JITTER_BIN_CENTS);
            }
        }
        unreachable!("rank {rank} is within the total {total}")
    }

    fn jitter_pairs(&self) -> u32 {
        self.jitter.values().sum()
    }

    fn fragmentation(&self) -> Option<f32> {
        (self.truth_runs > 0).then(|| self.estimate_runs as f32 / self.truth_runs as f32)
    }
}

impl Tally {
    fn merge(&mut self, other: &Tally) {
        self.transitions.merge(&other.transitions);
        self.calm.merge(&other.calm);
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
#[derive(Clone)]
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
            lo_midi: Hz(lo_hz).to_midi(CONCERT_PITCH_HZ).0,
            hi_midi: Hz(hi_hz).to_midi(CONCERT_PITCH_HZ).0,
        }
    }

    /// Out-of-band truth becomes unvoiced truth. Applied to the **annotation** only: the
    /// estimate is left exactly as its estimator emitted it, because clamping an estimate
    /// into the band would score a pitch nobody predicted.
    fn admit(&self, truth: Option<f32>) -> Option<f32> {
        truth.filter(|midi| *midi >= self.lo_midi && *midi <= self.hi_midi)
    }
}

/// One way of scoring a frame: which band the annotation is read in, and how far into the
/// past to read it.
///
/// # Why these are one axis and not two loops
///
/// Driving the bank is ~99 % of this benchmark's cost — a per-sample IIR over ~670
/// resonators, ~3.5 hours for the corpus — while consulting the annotation is a binary
/// search. So every extra (band, lag) combination is free, and re-running the bank to ask
/// a second question about the same audio would cost another 3.5 hours to learn something
/// already sitting in the frame that was just computed.
///
/// The bands matter as much as the lags on this corpus: the reference's 65 Hz floor cuts
/// **11.7 %** of MDB's voiced frames (its 2093 Hz ceiling cuts none — nothing here is that
/// high). Full-band is the honest number for a detector that hears down to MIDI 24; the
/// reference's band is the only one comparable to SwiftF0. Neither answers the other's
/// question, so the sweep reports both.
struct Scoring {
    mask:        PitchMask,
    lag_seconds: f32,
}

/// The annotation, resampled to nothing — kept exactly as the corpus wrote it.
///
/// `midi[i]` is `None` where the corpus wrote frequency 0.0, which is its encoding for an
/// unvoiced frame. Storing `Option` rather than a 0.0 sentinel is deliberate: a 0.0 Hz
/// "pitch" silently converts to −inf MIDI and would poison any arithmetic it touched.
pub(super) struct Annotation {
    /// Annotation timestamps in seconds, ascending (the corpus writes a uniform 2.9 ms
    /// grid, but nothing here assumes that — `at` binary-searches).
    times: Vec<f32>,
    midi:  Vec<Option<f32>>,
}

#[derive(Clone, Copy)]
struct AnnotationFrame(usize);

impl Annotation {
    /// The annotated pitch nearest in time to `seconds`.
    ///
    /// Nearest-neighbour, not interpolation: between two annotation frames the truth is a
    /// real trajectory we have no model for, and inventing one would put made-up numbers
    /// in the denominator of a benchmark. The corpus grid (2.9 ms) is ~5× finer than the
    /// bank's hop (16 ms), so the nearest sample is never more than 1.5 ms away.
    pub(super) fn at(&self, seconds: f32) -> Option<f32> {
        self.nearest_frame(seconds).and_then(|index| self.midi[index.0])
    }

    fn nearest_frame(&self, seconds: f32) -> Option<AnnotationFrame> {
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
        Some(AnnotationFrame(index))
    }

    /// Every native annotation step from the previous scored truth to the current one
    /// must be voiced, in band and smaller than 5 ¢. Checking only the endpoints would
    /// miss a jump and return, or a short rest, between two coarser detector frames.
    fn stands_between(&self, start: f32, end: f32, mask: &PitchMask) -> bool {
        let Some(first) = self.nearest_frame(start) else {
            return false;
        };
        let last = self.nearest_frame(end).unwrap();
        let truth = &self.midi[first.0..=last.0];
        truth.iter().all(|midi| mask.admit(*midi).is_some())
            && truth
                .windows(2)
                .all(|pair| (pair[1].unwrap() - pair[0].unwrap()).abs() * 100.0 < NOTE_STANDS_CENTS)
    }
}

/// Parse a `time,frequency` CSV. Frequency 0.0 means unvoiced.
///
/// No header in this corpus, so a line that does not parse as two floats is a corpus
/// problem, not a variant to tolerate — hence `unwrap`.
pub(super) fn load_annotation(path: &Path) -> Annotation {
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
        // 0 Hz is the corpus's unvoiced marker. Anything else is a real f0. The corpus
        // speaks Hz; the bank speaks MIDI.
        midi.push((hz > 0.0).then(|| Hz(hz).to_midi(CONCERT_PITCH_HZ).0));
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
fn bench_track(audio: &Path, annotation: &Annotation, scorings: &[Scoring]) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut analyzer = ResonatorAnalyzer::new(sample_rate);
    let hop = (sample_rate.hz() * BANK_HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); scorings.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        analyzer.process_samples(&samples[fed..fed + hop], true);
        fed += hop;
        // The frame reflects the bank's state *after* `fed` samples, so that is its
        // timestamp. What the *truth* was at that moment depends on the lag — see the
        // function's docs; the bank is driven once and every lag reads off the same frame.
        let seconds = fed as f32 / sample_rate.hz();
        let fundamental = analyzer.snapshot(true, AccidentalStyle::Sharps).fundamental();

        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, fundamental);
        }
    }
    tallies
}

/// Drive **RT-SWIPE** over a track — [`bench_track`]'s twin for the other frontend.
///
/// Deliberately the same shape as `bench_track`, line for line: same hop, same timestamp
/// convention, same `score_frame`, same sweep. The only difference is which frontend builds
/// the [`SalienceFrame`], which is exactly the variable this comparison is about — the
/// kernel, the decode and the scoring are shared code (`dsp::rtswipe` is a frontend to
/// `dsp::swipe`, not a second detector).
///
/// # ⚠ A global lag handicaps this frontend, and the handicap is not a defect
///
/// The sweep shifts the annotation by **one** lag for a whole track, which suits the bank:
/// its group delay is 8–29 ms across the range, so one number is nearly right everywhere.
/// RT-SWIPE's delay is 4 periods **of the pitch** — 1.6 ms at E7, 20.7 at G3, 130 at C1
/// (`rtswipe::tests::what_does_the_ladder_cost_in_delay`) — an 80-fold spread, and no single
/// lag fits it. Expect a *flatter, lower* sweep than the bank's for that reason alone.
///
/// So the sweep is the apples-to-apples comparison (each frontend at its own best lag, as
/// the kickstart's gate asks) and simultaneously an under-count of what this frontend could
/// deliver: RT-SWIPE knows its own delay analytically per candidate
/// ([`RtSwipe::delay_seconds`]), so a real integration would place each estimate in time
/// itself. Scoring *that* needs a per-frame lag, which `Scoring` cannot express — the
/// follow-up if and only if the sweep says the frontend is worth it.
fn bench_rtswipe(audio: &Path, annotation: &Annotation, scorings: &[Scoring]) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut rtswipe = RtSwipe::new(sample_rate, CONCERT_PITCH_HZ);
    let hop = (sample_rate.hz() * BANK_HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); scorings.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        rtswipe.process_samples(&samples[fed..fed + hop]);
        fed += hop;
        let seconds = fed as f32 / sample_rate.hz();
        let estimate = rtswipe.frame().and_then(|frame| frame.argmax());

        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
        }
    }
    tallies
}

/// Drive the **harmonic sieve** (`dsp::sieve`) over a track — [`bench_rtswipe`]'s twin.
///
/// Same hop, same timestamp convention, same `score_frame`, same sweep. The sieve searches
/// only `range`: unlike the bank it is built for an instrument's range, and its window length
/// follows from the bottom of it. Its delay is half that window — one number for the whole
/// track, so unlike RT-SWIPE a single lag *does* fit it, and the sweep should find it.
fn bench_sieve(audio: &Path, annotation: &Annotation, scorings: &[Scoring], range: (Hz, Hz)) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut sieve = HarmonicSieve::with_variant(sample_rate, range.0, range.1, ScoringVariant::from_env());
    let changes = std::env::var_os("PITCH_BENCH_SIEVE_DIAGNOSTICS")
        .map(|_| transitions::ReferenceChanges::new(annotation));
    let hop = (sample_rate.hz() * BANK_HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); scorings.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        sieve.process_samples(&samples[fed..fed + hop]);
        fed += hop;
        let seconds = fed as f32 / sample_rate.hz();
        let estimate = sieve
            .estimate()
            .map(|found| (found.hz.to_midi(CONCERT_PITCH_HZ).0, found.score));

        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
            if let Some(changes) = &changes {
                let frame = tally.calm.prev.unwrap();
                tally.transitions.follow(
                    changes,
                    seconds - scoring.lag_seconds,
                    frame.verdict,
                    frame.truth,
                    frame.estimate,
                );
            }
        }
    }
    tallies
}

/// R3: complete windows at exactly R2's phase, with the same audio/noise as all
/// other internal benchmarks. Optional oracle CSVs are compared by row, never
/// interpolated; a timestamp mismatch is a different experiment and must fail.
fn bench_yinfft(audio: &Path, annotation: &Annotation, scorings: &[Scoring]) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut detector = YinFft::new(sample_rate);
    let hop = sample_rate.samples_in(yinfft::HOP);
    let mut tallies = vec![Tally::default(); scorings.len()];
    let oracle = std::env::var_os("PITCH_BENCH_YINFFT_ORACLE_DIR").map(|directory| {
        load_annotation(&PathBuf::from(directory).join(audio.with_extension("csv").file_name().unwrap()))
    });
    let mut comparison = YinFftComparison::default();
    let mut rows = 0;
    for start in (0..samples.len().saturating_sub(yinfft::WINDOW_SAMPLES - 1)).step_by(hop) {
        let end = start + yinfft::WINDOW_SAMPLES;
        // R2 divides integer sample counts in JS f64, then the CSV loader
        // rounds seconds to f32. Casting end first loses a sample past 2^24.
        let seconds = (end as f64 / sample_rate.0 as f64) as f32;
        let estimate = detector
            .estimate(&samples[start..end])
            .map(|found| (found.hz.to_midi(CONCERT_PITCH_HZ).0, found.periodicity));
        if let Some(oracle) = &oracle {
            assert_eq!(
                oracle.times[rows],
                seconds,
                "oracle timestamp, {} row {rows}",
                audio.display()
            );
            comparison.follow(estimate.map(|(midi, _)| midi), oracle.midi[rows]);
        }
        rows += 1;
        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
        }
    }
    if let Some(oracle) = oracle {
        assert_eq!(
            rows,
            oracle.times.len(),
            "oracle frame count, {}",
            audio.display()
        );
        println!(
            "YINFFT oracle {}: {comparison:?}",
            audio.file_stem().unwrap().to_str().unwrap()
        );
        // These limits express numerical equivalence, not a quality threshold.
        // Report the maximum and counts before applying the numerical limits.
        assert_eq!(comparison.voicing_mismatches, 0);
        assert_eq!(comparison.over_one_cent, 0);
    }
    tallies
}

#[derive(Default, Debug)]
struct YinFftComparison {
    frames:             usize,
    both_voiced:        usize,
    voicing_mismatches: usize,
    max_cents:          f32,
    max_frame:          usize,
    over_one_cent:      usize,
}

impl YinFftComparison {
    fn follow(&mut self, actual: Option<f32>, expected: Option<f32>) {
        self.frames += 1;
        match (actual, expected) {
            (Some(actual), Some(expected)) => {
                self.both_voiced += 1;
                let cents = 100.0 * (actual - expected).abs();
                if cents > self.max_cents {
                    self.max_cents = cents;
                    self.max_frame = self.frames - 1;
                }
                self.over_one_cent += usize::from(cents > 1.0);
            }
            (None, None) => {}
            _ => self.voicing_mismatches += 1,
        }
    }
}

/// Drive the **peak comb** (`dsp::peak_comb`) over a track at its own cadence
/// ([`peak_comb::HOP_SECONDS`]) and level gate — [`peak_comb::DEFAULT_GATE_DBFS`], or
/// `PITCH_BENCH_COMB_GATE_DBFS`. The default suits a phone microphone and silences a third of
/// MDB's quieter stems, so a fair comparison lowers it (−55 is what `docs/peak_comb.md` uses).
///
/// `smoothed` picks what is scored: the 6-frame semitone average with its jump rejection, or
/// the raw per-frame estimate under it. Both run the same frames; the difference between the
/// two logs is the smoothing and nothing else. The smoothing state advances only through
/// `pitch`, so a raw run never touches it.
fn bench_peak_comb(
    audio: &Path,
    annotation: &Annotation,
    scorings: &[Scoring],
    smoothed: bool,
) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let gate_dbfs = std::env::var("PITCH_BENCH_COMB_GATE_DBFS")
        .map(|v| v.parse().unwrap())
        .unwrap_or(peak_comb::DEFAULT_GATE_DBFS);
    let mut detector = PeakComb::new(sample_rate, gate_dbfs);
    let hop = (sample_rate.hz() * peak_comb::HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); scorings.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        detector.process_samples(&samples[fed..fed + hop]);
        fed += hop;
        let seconds = fed as f32 / sample_rate.hz();
        // The second field is a strength `score_frame` does not read; 1.0 as in
        // `bench_external` — a placeholder, not a confidence.
        let estimate = if smoothed {
            detector.pitch().map(|midi| (midi.0, 1.0))
        } else {
            detector
                .raw_pitch()
                .map(|hz| (hz.to_midi(CONCERT_PITCH_HZ).0, 1.0))
        };

        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
        }
    }
    tallies
}

/// Drive the shipped **pYIN** tracker over a track, the way `audio::core` drives it: a
/// [`PYIN_WINDOW`]-sample window, right-aligned, every [`PYIN_HOP_SECONDS`].
///
/// Its own cadence, not the bank's: pYIN's transition probabilities are calibrated per
/// 40 ms frame (`dsp::trellis`), so driving it at 16 ms would measure a tracker the app does
/// not run. Each frame is scored where it sits in time, as `bench_external` does.
fn bench_pyin(audio: &Path, annotation: &Annotation, scorings: &[Scoring]) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut tracker = PitchTracker::new();
    // Starts as silence, as every other frontend here does.
    let mut window = vec![0.0f32; PYIN_WINDOW];
    let hop = (sample_rate.hz() * PYIN_HOP_SECONDS) as usize;
    let mut tallies = vec![Tally::default(); scorings.len()];

    let mut fed = 0usize;
    while fed + hop <= samples.len() {
        slide_in(&mut window, &samples[fed..fed + hop]);
        fed += hop;
        let seconds = fed as f32 / sample_rate.hz();
        let estimate = tracker
            .process(&window, sample_rate)
            .map(|(hz, voiced)| (Hz(hz).to_midi(CONCERT_PITCH_HZ).0, voiced));

        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
        }
    }
    tallies
}

/// Drive the **whole engine** over a track — the melody line the pianoroll draws, not a
/// frontend's argmax.
///
/// Every other `bench_*` here scores a frontend in isolation. What the user sees is
/// `MelodyFrame::pitch`: the frontend's salience decoded by `SalienceDecoder`'s trellis, run
/// through the repair layer (`OctaveGate`, snap, leap confirmation) against the pYIN anchor,
/// and gated by the input level. This feeds the WAV to [`Rig`] — both planes, shared state and
/// the sample clock, exactly as a device stream would — and scores every published frame.
///
/// # Grid and time
///
/// The engine's own frames, each at its `MelodyFrame::t`, as [`bench_external`] scores an
/// external trajectory where it sits: no resampling onto [`BANK_HOP_SECONDS`]. With the rig's
/// 10 ms chunks the default 16 ms publish gate fires every **20 ms**, so this grid is coarser
/// than the frontend benches' — a cadence of the rig, documented on [`Rig::feed`].
///
/// # Reading the history
///
/// The engine keeps only the last `MELODY_HISTORY_SECONDS` of frames, so the track is fed in
/// pieces shorter than that and the history is read with a cursor after each — the same
/// cursor a panel holds. Pieces are whole rig chunks, so piecing does not change the cadence.
fn bench_melody(
    audio: &Path,
    annotation: &Annotation,
    scorings: &[Scoring],
    frontend: PitchFrontend,
    stage: MelodyStage,
) -> Vec<Tally> {
    let (sample_rate, samples) = corpus_audio(audio);
    let mut rig = Rig::new(sample_rate);
    {
        let mut settings = rig.settings.lock().unwrap();
        // The line comes out in MIDI against the engine's concert pitch; the annotation is
        // read against ours. A mismatch would cost cents to arithmetic and blame the chain.
        assert_eq!(settings.concert_pitch_hz, CONCERT_PITCH_HZ);
        settings.resonator.frontend = frontend;
    }
    let mut tallies = vec![Tally::default(); scorings.len()];

    // Half a second: well inside the 2 s history, and a whole number of 10 ms chunks.
    let piece = Rig::chunk(sample_rate) * 50;
    let mut cursor = None;
    for part in samples.chunks(piece) {
        rig.feed(part, sample_rate);
        for frame in rig.melody_since(cursor) {
            cursor = Some(frame.seq);
            let seconds = frame.t as f32;
            // Strength is unread by `score_frame`; 1.0 is a placeholder, as in `bench_external`.
            let estimate = match stage {
                MelodyStage::Line => frame.pitch.map(|midi| (midi.0, 1.0)),
                MelodyStage::Salience => frame.salience.as_ref().and_then(salience_argmax),
                MelodyStage::Raw => frame.raw_pitch.map(|m| (m.0, 1.0)),
                MelodyStage::Reference => frame.reference_pitch.map(|m| (m.0, 1.0)),
            };
            for (tally, scoring) in tallies.iter_mut().zip(scorings) {
                score_frame(tally, seconds, annotation, scoring, estimate);
            }
        }
    }
    tallies
}

/// A track as every `bench_*` hears it: [`read_wav`], plus white noise at
/// `PITCH_BENCH_SNR_DB` when that is set (H2 of `docs/detector_hypotheses_plan.md`).
///
/// MDB is clean; a voice into a phone is not. The noise is Gaussian, with RMS = the
/// **whole track's** RMS / 10^(SNR/20). Stems have long rests, so over the sounding part the
/// SNR is higher than the nominal by 10·log10(1 / sounding fraction) — the nominal is a
/// property of the file, the same for every detector, which is what a comparison needs.
///
/// The generator is seeded from the file name, so every detector hears the same noise on a
/// track and two runs agree byte for byte.
fn corpus_audio(audio: &Path) -> (SampleRate, Vec<f32>) {
    let (sample_rate, mut samples) = read_wav(audio);
    if let Ok(snr) = std::env::var("PITCH_BENCH_SNR_DB") {
        let snr_db: f32 = snr.parse().unwrap();
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let noise_rms = rms / 10f32.powf(snr_db / 20.0);
        let mut noise = GaussianNoise::seeded(audio.file_name().unwrap().to_str().unwrap());
        for sample in &mut samples {
            *sample += noise_rms * noise.next();
        }
    }
    (sample_rate, samples)
}

/// Unit Gaussian samples from a 64-bit LCG (Knuth's MMIX constants) through Box–Muller.
///
/// Not `rand`: the stream has to be fixed by this file alone, so a log from today and one
/// from next year are about the same noise.
struct GaussianNoise {
    state: u64,
    /// Box–Muller makes pairs; the second waits here.
    spare: Option<f32>,
}

impl GaussianNoise {
    /// FNV-1a of `key` as the seed — stable across platforms and compiler versions,
    /// unlike `std`'s `DefaultHasher`.
    fn seeded(key: &str) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in key.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        GaussianNoise {
            state: hash,
            spare: None,
        }
    }

    /// Uniform in (0, 1]: the top 24 bits, so every value is exact in `f32`, and never 0,
    /// which `ln` below could not take.
    fn uniform(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.state >> 40) as f32 + 1.0) / (1u32 << 24) as f32
    }

    fn next(&mut self) -> f32 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }
        let radius = (-2.0 * self.uniform().ln()).sqrt();
        let angle = std::f32::consts::TAU * self.uniform();
        self.spare = Some(radius * angle.sin());
        radius * angle.cos()
    }
}

/// Which point of the engine's chain [`bench_melody`] scores.
#[derive(Clone, Copy, Debug)]
enum MelodyStage {
    /// `MelodyFrame::pitch` — the line the roll draws.
    Line,
    /// The argmax of `MelodyFrame::salience` — the frontend's curve the trellis decoded the
    /// line from, at the engine's cadence and before the trellis, the repair layer and the
    /// level gate. Line minus this is what that chain adds and takes away; this minus the
    /// frontend's own `*_rpa_on_corpus` is the cadence (20 ms here, 16 there) and the
    /// coarse read: bin centres, no partial refinement — fine for a 50 ¢ verdict, not for
    /// jitter.
    Salience,
    Raw,
    Reference,
}

/// The highest bin of a salience layer inside the tracked domain (C1..C8, as the decoder
/// reads it), as `(midi, strength)`; `None` when nothing there is positive.
fn salience_argmax(heat: &SalienceHeat) -> Option<(f32, f32)> {
    let bins = tracked_bins(heat.data.len(), heat.min_midi, heat.bins_per_semitone)?;
    let (bin, value) = bins
        .map(|b| (b, heat.data[b]))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    (value > 0.0).then(|| (heat.min_midi + bin as f32 / heat.bins_per_semitone, value))
}

/// A corpus WAV as `f32` in −1..1. The corpus is 16-bit mono; anything else is a corpus
/// problem, hence the `unwrap`s.
pub(super) fn read_wav(audio: &Path) -> (SampleRate, Vec<f32>) {
    let mut reader = hound::WavReader::open(audio).unwrap();
    let sample_rate = SampleRate(reader.spec().sample_rate);
    let samples = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
    (sample_rate, samples)
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
fn bench_external(estimates: &Annotation, annotation: &Annotation, scorings: &[Scoring]) -> Vec<Tally> {
    let mut tallies = vec![Tally::default(); scorings.len()];
    for (index, &seconds) in estimates.times.iter().enumerate() {
        // Strength is what the bank reports alongside a pitch and what `score_frame`
        // ignores (`_strength`); an external CSV carries no such thing, so 1.0 is a
        // placeholder for a field with no reader — not a confidence we are asserting.
        let estimate = estimates.midi[index].map(|midi| (midi, 1.0));
        for (tally, scoring) in tallies.iter_mut().zip(scorings) {
            score_frame(tally, seconds, annotation, scoring, estimate);
        }
    }
    tallies
}

/// Classify one frame's verdict into `tally`. Split out so the lag sweep cannot drift
/// into N slightly-different copies of the same arithmetic.
///
/// `seconds` is the frame's own time — not the lagged moment its truth was read at — and
/// frames of one track must arrive in time order: [`Calm`] counts runs and pairs.
fn score_frame(
    tally: &mut Tally,
    seconds: f32,
    annotation: &Annotation,
    scoring: &Scoring,
    estimate: Option<(f32, f32)>,
) {
    let truth_seconds = seconds - scoring.lag_seconds;
    let truth = scoring.mask.admit(annotation.at(truth_seconds));
    let verdict = classify(tally, truth, estimate);
    let truth_stands = verdict == Verdict::Correct
        && tally.calm.prev.is_some_and(|prev| {
            prev.verdict == Verdict::Correct
                && annotation.stands_between(prev.seconds - scoring.lag_seconds, truth_seconds, &scoring.mask)
        });
    tally.calm.follow(
        PrevFrame {
            seconds,
            truth,
            estimate: estimate.map(|(midi, _strength)| midi),
            verdict,
        },
        &scoring.mask,
        truth_stands,
    );
}

/// The counting half of [`score_frame`].
fn classify(tally: &mut Tally, truth: Option<f32>, estimate: Option<(f32, f32)>) -> Verdict {
    match (truth, estimate) {
        (None, None) => {
            tally.unvoiced += 1;
            Verdict::Unvoiced
        }
        (None, Some(_)) => {
            tally.unvoiced += 1;
            tally.false_voiced += 1;
            Verdict::FalseVoiced
        }
        (Some(_), None) => {
            tally.voiced += 1;
            tally.silent_miss += 1;
            Verdict::SilentMiss
        }
        (Some(truth), Some((estimate, _strength))) => {
            tally.voiced += 1;
            let cents = (estimate - truth) * 100.0;
            if cents.abs() <= RPA_TOLERANCE_CENTS {
                tally.correct += 1;
                tally.correct_cents += cents.abs();
                Verdict::Correct
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
                    Verdict::Octave
                } else {
                    tally.other += 1;
                    Verdict::Other
                }
            }
        }
    }
}

/// The «calm» counters on a hand-made track whose answer is known: two octave runs (one of
/// two frames, one of one), a gap inside a true segment, false voicing between segments, and
/// five standing-note pairs of correct frames.
#[test]
fn calm_counts_runs_pairs_and_pieces() {
    let truth = |i: usize| {
        match i {
            0..=9 => Some(60.0),
            10 | 11 => None,
            _ => Some(62.0),
        }
    };
    let estimates = [
        Some(60.02),
        Some(60.05),
        Some(72.0),
        Some(72.0),
        Some(60.0),
        None,
        Some(72.0),
        Some(60.0),
        Some(60.0),
        Some(60.0),
        Some(50.0),
        Some(50.0),
        Some(62.0),
        Some(62.0),
        Some(62.0),
    ];
    let annotation = Annotation {
        times: (0..estimates.len()).map(|i| i as f32 * 0.01).collect(),
        midi:  (0..estimates.len()).map(truth).collect(),
    };
    let scoring = Scoring {
        mask:        PitchMask::unbounded(),
        lag_seconds: 0.0,
    };
    let mut tally = Tally::default();
    for (i, estimate) in estimates.iter().enumerate() {
        score_frame(
            &mut tally,
            annotation.times[i],
            &annotation,
            &scoring,
            estimate.map(|m| (m, 1.0)),
        );
    }
    let calm = &tally.calm;

    assert_eq!(calm.octave_events, 2, "frames 2–3 are one event, frame 6 another");
    assert!(
        (calm.mean_octave_run_ms() - 15.0).abs() < 0.01,
        "3 frames × 10 ms over 2 events"
    );
    // 13 voiced frames, the first has no predecessor to measure its length by.
    assert!((calm.voiced_seconds - 0.12).abs() < 1e-5);
    assert_eq!(
        (calm.truth_runs, calm.estimate_runs),
        (2, 2),
        "the gap splits the estimate; false voicing connects the two truth segments"
    );
    assert_eq!(calm.jitter_pairs(), 5);
    assert!(
        (calm.jitter_quantile(0.5).unwrap() - 0.1).abs() < 1e-4,
        "three of five pairs do not move"
    );
    assert!(
        (calm.jitter_quantile(0.9).unwrap() - 3.0).abs() < 0.11,
        "60.02 → 60.05 is 3 ¢"
    );
}

/// A 3 ¢ native step qualifies at every detector cadence, even when its accumulated drift
/// over the detector hop exceeds 5 ¢. The metric still reports that hop's full movement.
#[test]
fn calm_selects_standing_truth_on_the_annotation_grid() {
    let annotation = Annotation {
        times: (0..=28).map(|i| i as f32 * 0.0029).collect(),
        midi:  (0..=28).map(|i| Some(60.0 + i as f32 * 0.03)).collect(),
    };
    let scoring = Scoring {
        mask:        PitchMask::unbounded(),
        lag_seconds: 0.0,
    };
    for step in [3, 6, 7, 14] {
        let mut tally = Tally::default();
        for i in (0..=28).step_by(step) {
            score_frame(
                &mut tally,
                annotation.times[i],
                &annotation,
                &scoring,
                annotation.midi[i].map(|midi| (midi, 1.0)),
            );
        }
        assert_eq!(tally.calm.jitter_pairs(), 28 / step as u32, "hop {step}");
        assert!((tally.calm.jitter_quantile(0.9).unwrap() - step as f32 * 3.0).abs() < 0.11);
    }
}

/// Identical endpoints do not make a standing note if a native step jumped, went silent
/// or left the band. Lagged scoring must inspect the same interval as its RPA verdicts.
#[test]
fn calm_rejects_hidden_transitions_and_uses_the_scoring_lag() {
    let scoring = Scoring {
        mask:        PitchMask {
            lo_midi: 59.0,
            hi_midi: 61.0,
        },
        lag_seconds: 0.0,
    };
    for middle in [Some(60.2), None, Some(72.0)] {
        let annotation = Annotation {
            times: vec![0.0, 0.01, 0.02, 0.03, 0.04, 0.05],
            midi:  vec![Some(60.0), middle, Some(60.0), Some(60.0), Some(60.0), Some(60.0)],
        };
        for (lag, expected_pairs) in [(0.0, 1), (0.03, 0)] {
            let mut tally = Tally::default();
            let lagged = Scoring {
                mask:        scoring.mask.clone(),
                lag_seconds: lag,
            };
            for seconds in [0.03, 0.05] {
                score_frame(&mut tally, seconds, &annotation, &lagged, Some((60.0, 1.0)));
            }
            assert_eq!(tally.correct, 2);
            assert_eq!(tally.calm.jitter_pairs(), expected_pairs);
        }
    }
    let threshold = Annotation {
        times: vec![0.0, 0.0029],
        midi:  vec![Some(0.0), Some(0.05)],
    };
    assert!(
        !threshold.stands_between(0.0, 0.0029, &PitchMask::unbounded()),
        "strictly below 5 ¢"
    );
}

#[test]
fn calm_counts_independent_runs_in_band_and_merges_tracks() {
    let annotation = Annotation {
        times: vec![0.01, 0.02, 0.03, 0.04, 0.05, 0.06],
        midi:  vec![Some(60.0), Some(60.0), None, None, Some(60.0), Some(60.0)],
    };
    let scoring = Scoring {
        mask:        PitchMask {
            lo_midi: 59.0,
            hi_midi: 73.0,
        },
        lag_seconds: 0.0,
    };
    let estimates = [
        Some(72.0),
        Some(60.0),
        Some(60.0),
        Some(60.0),
        Some(80.0),
        Some(60.0),
    ];
    let mut tally = Tally::default();
    for (&seconds, estimate) in annotation.times.iter().zip(estimates) {
        score_frame(
            &mut tally,
            seconds,
            &annotation,
            &scoring,
            estimate.map(|m| (m, 1.0)),
        );
    }
    assert_eq!((tally.calm.truth_runs, tally.calm.estimate_runs), (2, 2));
    assert_eq!(tally.calm.fragmentation(), Some(1.0));
    assert_eq!(tally.calm.octave_events, 1);
    assert!(
        (tally.calm.mean_octave_run_ms() - 10.0).abs() < 1e-4,
        "first frame owns one hop"
    );
    assert!((tally.calm.voiced_seconds - 0.04).abs() < 1e-5);
    let mut total = Tally::default();
    total.merge(&tally);
    total.merge(&tally);
    assert!(
        total.calm.prev.is_none(),
        "tracks must not form cross-track pairs or runs"
    );
    assert_eq!((total.calm.truth_runs, total.calm.estimate_runs), (4, 4));
    assert_eq!(total.calm.octave_events, 2);
    assert_eq!(total.calm.jitter_pairs(), 2 * tally.calm.jitter_pairs());

    let silence = Annotation {
        times: vec![0.01],
        midi:  vec![None],
    };
    let mut no_truth = Tally::default();
    score_frame(&mut no_truth, 0.01, &silence, &scoring, Some((60.0, 1.0)));
    assert_eq!(no_truth.calm.estimate_runs, 1);
    assert_eq!(
        no_truth.calm.fragmentation(),
        None,
        "no truth runs ⇒ ratio is undefined"
    );
}

/// The H2 noise is unit-RMS, zero-mean, and the same stream for the same key — the three
/// things `corpus_audio`'s SNR and reproducibility rest on.
#[test]
fn bench_noise_is_unit_gaussian_and_reproducible() {
    let draw = |key: &str| {
        let mut noise = GaussianNoise::seeded(key);
        (0..200_000).map(|_| noise.next()).collect::<Vec<f32>>()
    };
    let a = draw("AClassicEducation_NightOwl_STEM_08.RESYN.wav");
    let n = a.len() as f32;
    let mean = a.iter().sum::<f32>() / n;
    let rms = (a.iter().map(|x| x * x).sum::<f32>() / n).sqrt();
    assert!(mean.abs() < 0.01, "mean {mean}");
    assert!((rms - 1.0).abs() < 0.01, "rms {rms}");
    // Gaussian, not uniform: ~4.6 % of draws beyond 2σ (uniform at unit RMS has none past 1.73).
    let tail = a.iter().filter(|x| x.abs() > 2.0).count() as f32 / n;
    assert!((tail - 0.0455).abs() < 0.005, "tail beyond 2σ {tail}");
    assert_eq!(a, draw("AClassicEducation_NightOwl_STEM_08.RESYN.wav"));
    assert_ne!(a[..16], draw("another.wav")[..16]);
}

/// The corpus layout, as Zenodo record 1481172 unpacks it.
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasets/MDB-stem-synth")
}

/// Pair every annotation with its audio. Returns `(track_id, wav, csv)`, sorted, so a run
/// is reproducible and two runs are diffable.
pub(super) fn tracks() -> Vec<(String, PathBuf, PathBuf)> {
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
/// `NightOwl` — ids name *pieces*, not instruments), `PITCH_BENCH_INSTRUMENT` (see
/// [`instrument_filter`]) and `PITCH_BENCH_LIMIT` (max tracks).
///
/// They exist because the full corpus is 15.5 hours through a per-sample IIR bank — hours
/// of CPU, which is a fine overnight run and a terrible edit-test loop. Default: every
/// track, because the default should be the honest number.
fn selected_tracks() -> Vec<(String, PathBuf, PathBuf)> {
    let filter = std::env::var("PITCH_BENCH_FILTER").unwrap_or_default();
    let instruments = instrument_filter();
    let limit: usize = std::env::var("PITCH_BENCH_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let selected: Vec<_> = tracks()
        .into_iter()
        .filter(|(id, _, _)| filter.is_empty() || id.to_lowercase().contains(&filter.to_lowercase()))
        .filter(|(id, _, _)| instruments.as_ref().is_none_or(|keep| keep.contains(id)))
        .take(limit)
        .collect();
    assert!(!selected.is_empty(), "no tracks matched filter {filter:?}");
    if !filter.is_empty() {
        println!("    filter: {filter:?}");
    }
    selected
}

/// The track ids whose instrument is in `PITCH_BENCH_INSTRUMENT` (comma-separated MedleyDB
/// names, e.g. `violin` or `violin,viola,cello`), or `None` when the variable is unset.
///
/// The corpus does not say which instrument a stem is; MedleyDB's metadata does, and
/// `tools/medleydb_instruments.py` turns it into `datasets/medleydb_meta/stem_instruments.tsv`.
/// That this is the instrument *before* resynthesis matters: the audio is a synthetic
/// harmonic tone following a real violinist's f0 — vibrato, glides and all — not a bowed
/// string's spectrum. It answers "does the detector follow a violinist", not "does it
/// survive a violin's body".
fn instrument_filter() -> Option<std::collections::HashSet<String>> {
    let wanted = std::env::var("PITCH_BENCH_INSTRUMENT").ok()?;
    let wanted: Vec<String> = wanted.split(',').map(|name| name.trim().to_lowercase()).collect();
    let table = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasets/medleydb_meta/stem_instruments.tsv");
    let text = std::fs::read_to_string(&table).unwrap_or_else(|_| {
        panic!(
            "{} missing — run `uv run tools/medleydb_instruments.py`",
            table.display()
        )
    });
    let keep: std::collections::HashSet<String> = text
        .lines()
        .map(|line| line.split_once('\t').unwrap())
        .filter(|(_, instrument)| wanted.contains(&instrument.to_lowercase()))
        .map(|(stem, _)| format!("{stem}.RESYN"))
        .collect();
    println!("    instruments: {wanted:?} — {} stems", keep.len());
    Some(keep)
}

/// Worker threads for [`run_corpus`], from `PITCH_BENCH_JOBS` or half the machine.
///
/// Half by default because this box is shared: a benchmark that takes every core is a
/// benchmark that slows down everything else running on it.
fn jobs_from_env() -> usize {
    std::env::var("PITCH_BENCH_JOBS")
        .map(|v| v.parse().unwrap())
        .unwrap_or_else(|_| (std::thread::available_parallelism().unwrap().get() / 2).max(1))
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

/// The bands a run scores in, from `PITCH_BENCH_MASK_HZ` (`"65,2093"`) or, by default,
/// **both** of the two that mean something on this corpus.
///
/// Reporting two is not indecision. On MDB the reference's 65 Hz floor removes 11.7 % of
/// all voiced frames (its 2093 Hz ceiling removes none — nothing in the corpus is that
/// high), so the two bands are different questions with materially different answers:
///
/// - **full band** — the detector as it ships, which hears down to MIDI 24 (~33 Hz). The
///   honest headline, and the only one that describes what the app does.
/// - **the reference's band** — the only one comparable to SwiftF0, whose numbers exist
///   solely inside it. Quoting our full-band figure against theirs would be scoring an
///   eighth of the corpus that they never attempted.
///
/// Both come out of one pass, because the bank is the cost and the annotation lookup is
/// not — see `Scoring`.
fn bands_from_env() -> Vec<(String, PitchMask)> {
    match std::env::var("PITCH_BENCH_MASK_HZ") {
        Ok(spec) => {
            let (lo, hi) = spec.split_once(',').unwrap();
            let (lo, hi) = (lo.trim().parse().unwrap(), hi.trim().parse().unwrap());
            vec![(format!("{lo}-{hi} Hz"), PitchMask::from_hz(lo, hi))]
        }
        Err(_) => {
            vec![
                (
                    "full band — the detector as it ships".to_string(),
                    PitchMask::unbounded(),
                ),
                (
                    format!(
                        "{}-{} Hz — the reference's MDB band, comparable to SwiftF0",
                        REFERENCE_MDB_MASK_HZ.0, REFERENCE_MDB_MASK_HZ.1
                    ),
                    PitchMask::from_hz(REFERENCE_MDB_MASK_HZ.0, REFERENCE_MDB_MASK_HZ.1),
                ),
            ]
        }
    }
}

/// Every (band, lag) pair, in the order `report` expects to slice them: all lags of the
/// first band, then all lags of the second.
fn scorings(bands: &[(String, PitchMask)], lags_ms: &[f32]) -> Vec<Scoring> {
    bands
        .iter()
        .flat_map(|(_label, mask)| {
            lags_ms.iter().map(move |ms| {
                Scoring {
                    mask:        mask.clone(),
                    lag_seconds: ms / 1000.0,
                }
            })
        })
        .collect()
}

/// **The baseline.** RPA of the shipped SWIPE′ over the corpus.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn swipe_rpa_on_corpus() {
    // The default brackets the bank's measured 8–29 ms group delay so the plateau is
    // visible rather than assumed.
    run_corpus(
        "SWIPE′ RPA · MDB-stem-synth",
        &[0.0, 8.0, 16.0, 24.0, 32.0],
        bench_track,
    );
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

/// **Track R's number.** RPA of the FFT frontend over the corpus, against the bank's.
///
/// Same corpus, same harness, same scorer, same sweep as [`swipe_rpa_on_corpus`] — the
/// frontend is the only variable, which is the whole design of `dsp::rtswipe`. Read the two
/// runs side by side; the baseline is printed below so a reading of this log needs nothing
/// else open.
///
/// Prints rather than asserts, as every corpus run in this module does: a threshold baked in
/// here would be a number nobody re-measures, and the bank's own figures move whenever the
/// bank does. The *gate* is a judgement made against these numbers (the kickstart's R2), not
/// an `assert` — and its second half, the phantom octave on `testdata/`, lives where the
/// violin takes are.
///
/// ⚠ Before reading the gap to Marttila & Reiss's 96.1 %: **their metric has not been read**
/// (`published-numbers-need-their-metric-read`). Track O showed on a live example that a
/// published "RPA" was a different quantity by 9 points. The bank baseline below is the
/// comparison that is definitely apples-to-apples; the literature line is scale, not a gate.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn rtswipe_rpa_on_corpus() {
    // The bank's sweep, unchanged, so the two logs' columns line up literally.
    run_corpus(
        "RT-SWIPE (FFT frontend) RPA · MDB-stem-synth",
        &[0.0, 8.0, 16.0, 24.0, 32.0],
        bench_rtswipe,
    );
    println!(
        "\n  The baseline this is here to beat — the bank over the same 230 tracks,\n  \
         `swipe_rpa_on_corpus`, 2026-07-15 (datasets/bench_runs/):\n    \
         full band     : 82.99% @ lag 0   90.18% @ lag 16 (its peak)\n    \
         65-2093 Hz    : 83.93% @ lag 0   91.02% @ lag 16 (its peak)\n  \
         and SwiftF0, scored by this same harness in the reference's band:\n    \
         65-2093 Hz    : 87.87% @ lag 0   89.54% @ lag +8 (its peak)"
    );
    println!(
        "\n  ⚠ A single lag handicaps this frontend: its delay is 4 periods OF THE PITCH\n  \
         (1.6 ms at E7, 20.7 at G3, 130 at C1), so no one lag fits a whole track. A flat,\n  \
         low sweep is that, not necessarily a worse scorer — see `bench_rtswipe`.\n  \
         ⚠ Marttila & Reiss's 96.1% is NOT known to be this metric. Read their paper\n  \
         before treating the gap as real — track O found exactly that trap costing 9 points."
    );
}

/// **The harmonic sieve's number** (`dsp::sieve`), on the same corpus, harness and sweep.
///
/// The sieve is built for a range, so it searches the reference's band (65–2093 Hz): that
/// band's figures are the comparable ones, and the full-band ones count the 11.7 % of voiced
/// frames below 65 Hz it is not asked to find. The default sweep brackets its delay — half
/// of a 2048-sample window at 44.1 kHz, 23 ms.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn sieve_rpa_on_corpus() {
    println!("Sieve variant: {:?}", ScoringVariant::from_env());
    let range = (Hz(REFERENCE_MDB_MASK_HZ.0), Hz(REFERENCE_MDB_MASK_HZ.1));
    run_corpus(
        "harmonic sieve RPA · MDB-stem-synth",
        &[0.0, 8.0, 16.0, 24.0, 32.0],
        |audio, annotation, scorings| bench_sieve(audio, annotation, scorings, range),
    );
    print_baselines();
}

#[test]
#[ignore = "needs datasets/MDB-stem-synth; optionally PITCH_BENCH_YINFFT_ORACLE_DIR with R2 CSVs"]
fn yinfft_rpa_on_corpus() {
    run_corpus(
        "Rust YINFFT 2048/~16 ms RPA · MDB-stem-synth",
        &[0.0, 8.0, 16.0, 24.0, 32.0, 48.0, 64.0, 80.0, 96.0],
        bench_yinfft,
    );
}

/// **pYIN's number** — the shipped `PitchTracker`, at its own 40 ms cadence.
///
/// The slow plane the sieve is meant to compete with. Its window is 6144 samples, so its
/// delay is ~70 ms at 44.1 kHz; the sweep reaches past that.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn pyin_rpa_on_corpus() {
    run_corpus(
        "pYIN RPA · MDB-stem-synth",
        &[0.0, 24.0, 48.0, 72.0, 96.0],
        bench_pyin,
    );
    print_baselines();
}

/// **The peak comb's number** (`dsp::peak_comb`), smoothed: 4096-sample window, 8 ms cadence,
/// 6-frame semitone average.
///
/// Its delay is half the window (46 ms at 44.1 kHz) plus about half the smoothing span
/// (~20 ms), so the sweep reaches well past the sieve's.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn peak_comb_rpa_on_corpus() {
    run_corpus(
        "peak comb (smoothed) RPA · MDB-stem-synth",
        &[0.0, 24.0, 48.0, 64.0, 80.0],
        |audio, annotation, scorings| bench_peak_comb(audio, annotation, scorings, true),
    );
    print_baselines();
}

/// The peak comb's per-frame estimate **without** its smoothing — what the comb alone does.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn peak_comb_raw_rpa_on_corpus() {
    run_corpus(
        "peak comb (raw, per frame) RPA · MDB-stem-synth",
        &[0.0, 24.0, 40.0, 48.0, 64.0],
        |audio, annotation, scorings| bench_peak_comb(audio, annotation, scorings, false),
    );
    print_baselines();
}

/// **The shipped melody line** — the whole engine, frontend through trellis, repair layer and
/// level gate, as the pianoroll draws it (H0 of `docs/detector_hypotheses_plan.md`).
///
/// The frontend is `PITCH_BENCH_FRONTEND` — `bank` (default), `rtswipe` or `sieve`.
/// Read against the same frontend's own `*_rpa_on_corpus`: the difference is what
/// the chain after the frontend adds and takes away.
///
/// Slow: the analysis plane (spectrum, pYIN, onset) runs beside the bank, and the bank runs
/// even under RT-SWIPE because it drives the display.
#[test]
#[ignore = "needs datasets/MDB-stem-synth (~5 GB) — run with --ignored --nocapture"]
fn melody_rpa_on_corpus() {
    let frontend =
        PitchFrontend::from_key(&std::env::var("PITCH_BENCH_FRONTEND").unwrap_or_else(|_| "bank".into()))
            .unwrap();
    // `PITCH_BENCH_MELODY_STAGE=salience` scores the trellis's input instead of its output —
    // the decomposition, see `MelodyStage`.
    let stage = match std::env::var("PITCH_BENCH_MELODY_STAGE").as_deref() {
        Err(_) | Ok("line") => MelodyStage::Line,
        Ok("salience") => MelodyStage::Salience,
        Ok("raw") => MelodyStage::Raw,
        Ok("reference") => {
            assert_eq!(frontend, PitchFrontend::PeakComb);
            MelodyStage::Reference
        }
        Ok(other) => panic!("PITCH_BENCH_MELODY_STAGE={other:?} — expected line, salience, raw or reference"),
    };
    println!(
        "Frontend beta: {}",
        crate::audio::dsp::melody::salience_beta(frontend)
    );
    run_corpus(
        &format!("melody {stage:?} (whole engine, {frontend:?}) RPA · MDB-stem-synth"),
        &[0.0, 16.0, 32.0, 48.0, 64.0],
        |audio, annotation, scorings| bench_melody(audio, annotation, scorings, frontend, stage),
    );
    print_baselines();
}

/// Score captured application outputs through the existing scorer, at their own timestamps.
/// CSV files must cover every selected track; unlike the SwiftF0 oracle, there is no
/// reference-harness verdict or resampled truth to compare against here.
#[test]
#[ignore = "needs PITCH_BENCH_TRAJECTORY_DIR with captured time,frequency CSVs and run.json"]
fn trajectory_rpa_on_corpus() {
    assert!(
        std::env::var_os("PITCH_BENCH_SNR_DB").is_none(),
        "noise belongs to the captured audio; unset PITCH_BENCH_SNR_DB when scoring CSVs"
    );
    let directory = PathBuf::from(
        std::env::var_os("PITCH_BENCH_TRAJECTORY_DIR")
            .expect("set PITCH_BENCH_TRAJECTORY_DIR to the captured trajectory directory"),
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.join("run.json")).expect("read trajectory run.json"),
    )
    .expect("parse trajectory run.json");
    let detector = manifest["detector"].as_str().expect("run.json detector");
    println!("captured trajectory: {manifest}");
    run_corpus(
        &format!("{detector} (captured trajectory) · MDB-stem-synth"),
        &[0.0, 16.0, 32.0, 48.0, 64.0, 80.0, 96.0, 112.0, 128.0],
        |audio, annotation, scorings| {
            let csv = directory.join(audio.with_extension("csv").file_name().unwrap());
            let estimates = load_annotation(&csv);
            bench_external(&estimates, annotation, scorings)
        },
    );
}

/// Export exactly what internal detectors hear, including the deterministic H2 noise.
/// Float WAV preserves the f32 samples without PCM16 requantization or clipping.
#[test]
#[ignore = "needs PITCH_BENCH_AUDIO_DUMP_DIR; exports selected corpus audio as float WAV"]
fn dump_audio_for_external_detector() {
    let directory = PathBuf::from(
        std::env::var_os("PITCH_BENCH_AUDIO_DUMP_DIR")
            .expect("set PITCH_BENCH_AUDIO_DUMP_DIR for the external detector's audio"),
    );
    std::fs::create_dir_all(&directory).expect("create audio dump directory");
    let tracks = selected_tracks();
    for (_, audio, _) in &tracks {
        let destination = directory.join(audio.file_name().unwrap());
        assert!(
            !destination.exists(),
            "refusing to overwrite {}",
            destination.display()
        );
        let (sample_rate, samples) = corpus_audio(audio);
        let mut writer = hound::WavWriter::create(
            &destination,
            hound::WavSpec {
                channels:        1,
                sample_rate:     sample_rate.0,
                bits_per_sample: 32,
                sample_format:   hound::SampleFormat::Float,
            },
        )
        .expect("create float WAV");
        for sample in samples {
            writer.write_sample(sample).expect("write float sample");
        }
        writer.finalize().expect("finalize float WAV");
    }
    println!("exported {} tracks to {}", tracks.len(), directory.display());
}

/// Figures every corpus run is read against, printed so a log needs nothing else open.
fn print_baselines() {
    println!(
        "\n  Baselines, same harness (datasets/bench_runs/, 2026-07-15):\n    \
         bank SWIPE′, 65-2093 Hz : 83.93% @ lag 0   91.02% @ lag 16 (its peak)\n    \
         SwiftF0,     65-2093 Hz : 87.87% @ lag 0   89.54% @ lag +8 (its peak)"
    );
}

/// Run `bench` over the selected tracks at every (band, lag) and print the corpus report.
///
/// One loop for every frontend, so that what differs between two logs is the frontend and
/// nothing else. `default_lags_ms` must start at 0 — see [`lags_ms_from_env`].
///
/// Tracks are independent, so they run on [`jobs_from_env`] threads; the results are then
/// merged and printed **in track order**, so a parallel run's log is line-for-line the
/// sequential one's and two runs stay diffable.
fn run_corpus(
    headline: &str,
    default_lags_ms: &[f32],
    bench: impl Fn(&Path, &Annotation, &[Scoring]) -> Vec<Tally> + Sync,
) {
    let lags_ms = lags_ms_from_env(default_lags_ms);
    let bands = bands_from_env();
    let grid = scorings(&bands, &lags_ms);

    println!("\n=== {headline} ===");
    let selected = selected_tracks();
    let jobs = jobs_from_env();
    println!(
        "    {} tracks × {} bands × {} lags, {} threads",
        selected.len(),
        bands.len(),
        lags_ms.len(),
        jobs
    );
    if let Ok(snr) = std::env::var("PITCH_BENCH_SNR_DB") {
        println!("    white noise at {snr} dB SNR (whole-track RMS) — see `corpus_audio`");
    }

    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut finished: Vec<(usize, Vec<Tally>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((_, wav, csv)) = selected.get(index) else {
                            break;
                        };
                        done.push((index, bench(wav, &load_annotation(csv), &grid)));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    });
    finished.sort_by_key(|(index, _)| *index);

    let mut per_scoring = vec![Tally::default(); grid.len()];
    for (index, tallies) in &finished {
        let id = &selected[*index].0;
        for (corpus, tally) in per_scoring.iter_mut().zip(tallies) {
            corpus.merge(tally);
        }
        // Per track, report the first band's sweep as bare RPAs — the shape (does it
        // climb? where does it flatten?) is the per-track story; the breakdown, and the
        // other bands, belong to the corpus summary.
        let sweep: Vec<String> = tallies[..lags_ms.len()]
            .iter()
            .map(|t| format!("{:>5.1}", t.rpa()))
            .collect();
        println!("  {:<48} RPA {}", id, sweep.join(" "));
    }

    for (index, (label, _)) in bands.iter().enumerate() {
        let band = &per_scoring[index * lags_ms.len()..(index + 1) * lags_ms.len()];
        report(label, &lags_ms, band);
    }
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
    // The oracle's band is a property of the oracle, not of this run — it is whatever
    // their dataset class declared when the CSVs were written, so there is nothing here
    // for an environment variable to decide.
    let band = (
        format!(
            "SwiftF0 in {}-{} Hz, on the corpus annotation",
            mask_hz.0, mask_hz.1
        ),
        PitchMask::from_hz(mask_hz.0, mask_hz.1),
    );

    // Lag 0 is the comparison point: their harness has no lag concept — it scores frame k
    // against the annotation resampled onto frame k — so lag 0 is the only setting that
    // asks them and us the same question. The rest of the sweep is diagnosis: SwiftF0
    // centres its window and is delay-compensated by construction, so a peak *away* from
    // 0 would mean the two grids are offset, not that the model is late.
    let lags_ms = lags_ms_from_env(&[0.0, -16.0, -8.0, 8.0, 16.0]);
    let report_grid = scorings(std::slice::from_ref(&band), &lags_ms);
    // The gate's single scoring: their annotation, no band (their `_validate_pitch` already
    // zeroed the periodicity of out-of-band frames before `truth/` was written, so masking
    // again would mask twice), and no lag — both files sit on the same grid by
    // construction, so `Annotation::at` lands on the exact frame and interpolates nothing.
    let gate_grid = [Scoring {
        mask:        PitchMask::unbounded(),
        lag_seconds: 0.0,
    }];

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
    let mut per_lag = vec![Tally::default(); report_grid.len()];
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
        let their_truth = load_annotation(&truth_dir.join(format!("{id}.csv")));
        gate_tally.merge(&bench_external(&estimates, &their_truth, &gate_grid)[0]);

        // Pass 2 — the report: the corpus's own annotation, which is the accurate one.
        let annotation = load_annotation(csv);
        let tallies = bench_external(&estimates, &annotation, &report_grid);
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
    for (lag_ms, tally) in lags_ms.iter().zip(per_lag) {
        tally.transitions.report(*lag_ms);
    }
    // H1: what the roll shows. Per lag, because «correct» — and with it the jitter pairs and
    // the octave runs — depends on which moment of the truth a frame is held to.
    println!("\n=== calm — what the roll shows ===");
    println!(
        "  octave events per minute of voiced audio (mean run), jitter |Δ| on a standing note\n  \
         between correct neighbours (median / p90, pairs), independent in-band voiced runs\n  \
         standing = every native annotation step <5 cents across the lagged pair;\n  \
         jitter is per detector hop, not per second (cadences differ)"
    );
    println!("  lag(ms)   oct/min   run ms   jitter med   p90 ¢     pairs   pieces/seg    est/truth runs");
    let cents = |q: Option<f32>| q.map_or("    —".to_string(), |c| format!("{c:>5.1}"));
    for (lag_ms, tally) in lags_ms.iter().zip(per_lag) {
        let calm = &tally.calm;
        let pieces = calm
            .fragmentation()
            .map_or("     —".to_string(), |ratio| format!("{ratio:>6.2}"));
        println!(
            "  {:>5.0}    {:>6.2}   {:>6.0}   {}      {}   {:>8}   {}     {}/{}",
            lag_ms,
            calm.octave_events_per_minute(),
            calm.mean_octave_run_ms(),
            cents(calm.jitter_quantile(0.5)),
            cents(calm.jitter_quantile(0.9)),
            calm.jitter_pairs(),
            pieces,
            calm.estimate_runs,
            calm.truth_runs
        );
    }
}
