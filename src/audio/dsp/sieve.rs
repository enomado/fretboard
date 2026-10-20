//! Harmonic sieve over spectral peaks — a per-frame f0 estimator.
//!
//! One right-aligned FFT window, its spectral peaks, and a search for the fundamental whose
//! harmonic series explains those peaks best. No trellis, no history between frames: every
//! frame stands on its own, so a note change costs one window and nothing else.
//!
//! # Where each piece comes from
//!
//! - **Peaks, not the whole spectrum** — the sinusoidal-analysis frontend of PARSHL/SMS
//!   (Smith & Serra 1987, Serra 1989): local maxima of a windowed log-magnitude spectrum,
//!   each refined by a parabola through its three bins ([`parabolic_vertex`]).
//! - **The sieve** — Duifhuis, Willems & Sluyter 1982, "Measurement of pitch in speech: an
//!   implementation of Goldstein's theory of pitch perception": a candidate f0 is judged by
//!   how well the peaks fall into meshes around its harmonics.
//! - **Scoring both directions** — Maher & Beauchamp 1994, "Fundamental frequency estimation
//!   of musical signals using a two-way mismatch procedure" (TWM): *predicted → measured*
//!   (every harmonic the candidate predicts should find a peak — this is what rejects a
//!   sub-octave) and *measured → predicted* (every peak should sit on a predicted harmonic —
//!   this is what rejects an octave above).
//! - **Meshes with zero mean** — the idea behind SWIPE's kernel (Camacho & Harris 2008),
//!   whose negative lobes between harmonics make a spectrum that is merely *busy* score
//!   nothing. Here each mesh is a Gaussian minus its own average over one harmonic spacing,
//!   so a peak landing at a random place contributes zero in expectation, not a bonus.
//! - **Log-magnitude weights** — each peak votes with its level in dB above the frame's
//!   floor. Compression of this kind is standard (SWIPE uses √, Klapuri 2006 a power law);
//!   the log is the strong end of it, and is chosen so that the weak upper partials of a
//!   bowed string — which pin the frequency far more precisely than the fundamental does —
//!   are not drowned out by the two loudest ones.
//! - **Refinement by harmonic least squares** — once a candidate's harmonics are matched,
//!   `f0 = Σ w·k·x / Σ w·k²` is the weighted least-squares fit of `x_k ≈ k·f0`. A partial at
//!   harmonic `k` constrains f0 `k` times more tightly than the fundamental, which is the
//!   whole reason to look at many of them.
//!
//! Every numeric constant below is a starting point to be argued against
//! `pitch_bench::sieve_rpa_on_corpus`, not a value taken from anywhere.
//!
//! # Causal by right-alignment
//!
//! As in `dsp::rtswipe`, the window ends at the newest sample, so the estimate describes the
//! signal half a window ago: [`HarmonicSieve::delay_seconds`].

use std::sync::Arc;

use realfft::{
    RealFftPlanner,
    RealToComplex,
};
use rustfft::num_complex::Complex32;

use super::analysis_math::{
    hann_taper,
    parabolic_vertex,
    slide_in,
    subtract_moving_mean,
};
use crate::audio::sample_rate::SampleRate;
use crate::core_types::pitch::Hz;

/// Periods of the **lowest** searchable pitch that the window must span.
///
/// Sets the one compromise this estimator has: resolution against reaction. Fewer periods
/// and the partials of the lowest note blur into each other; more and every note change
/// takes longer to leave the window. The window is then rounded up to a power of two.
const PERIODS_PER_WINDOW: f32 = 3.0;

/// Peaks more than this far below the frame's loudest one are not peaks, they are floor.
///
/// Also the zero of the weights: a peak's vote is its height above `max − this`.
const DYNAMIC_RANGE_DB: f32 = 60.0;

/// Below this the frame is silence and no pitch is claimed, whatever the peaks say.
const SILENCE: Dbfs = Dbfs(-70.0);

/// The strongest peaks kept per frame. Bounds the cost; a bowed string rarely shows more
/// resolved partials than this inside `DYNAMIC_RANGE_DB`.
const MAX_PEAKS: usize = 40;

/// How many of the strongest peaks seed candidates (each at `peak / k`, k = 1, 2, …).
///
/// The fundamental itself need not be among the peaks — on the violin's G string it is often
/// the weakest partial — but some strong partial always is, and the fundamental is one of
/// its integer fractions.
const CANDIDATE_SEEDS: usize = 3;

/// Highest harmonic number a candidate is judged on. Peaks above this harmonic of the
/// candidate are left out of both of its sums rather than counted against it.
const MAX_HARMONICS: usize = 24;

/// Mesh width, in FFT bins: how far a well-resolved partial may sit from its exact harmonic
/// purely because of the window (parabolic refinement leaves a fraction of a bin).
const MESH_SIGMA_BINS: f32 = 0.5;

/// Mesh width, relative to the harmonic's own frequency: the part of a partial's deviation
/// that grows with `k` — vibrato and glide inside the window, string inharmonicity.
const MESH_SIGMA_RELATIVE: f32 = 0.005;

/// What a harmonic slot costs per dB of *shortfall* below the frame's mean peak weight —
/// an empty slot pays this times the whole mean, a slot holding a junk peak nearly as much.
///
/// This is the predicted → measured term doing its job: one octave down, every other slot
/// has nothing real in it, so a sub-octave pays on half its slots while the true f0 pays
/// only on its genuinely weak partials. Charging only *empty* slots was not enough: on the
/// A-string take (`testdata/a_string_trill.wav`) 10 % of frames went an octave down,
/// because junk peaks inside the 60 dB range kept a sub-octave's odd slots "occupied".
/// Charging the shortfall took that to 0.9 %, at −0.7 points of corpus RPA.
const MISSING_HARMONIC_PENALTY: f32 = 1.5;

/// H4 experiments from `docs/detector_hypotheses_plan.md`, fixed before corpus runs.
/// The half-frequency valley is weighted 1.8; the other four valleys have unit weight.
const INTERHARMONIC_VALLEYS: [(f32, f32); 5] = [
    (0.5, 1.8),
    (1.5, 1.0),
    (2.5, 1.0),
    (1.0 / 3.0, 1.0),
    (2.0 / 3.0, 1.0),
];
const FUNDAMENTAL_SHORTFALL_MULTIPLIER: f32 = 2.0;

/// H5 width fixed by the plan (~31 bins at 4096 / 44.1 kHz), before corpus runs.
/// Converted to the nearest odd number of bins for this sieve's range-dependent window.
const WHITENING_WIDTH: Hz = Hz(330.0);

fn whitening_bins(bin_hz: f32) -> usize {
    2 * ((WHITENING_WIDTH.0 / bin_hz - 1.0) / 2.0).round().max(0.0) as usize + 1
}

/// Independent H4/H5 experiments. Baseline remains the default until acceptance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScoringVariant {
    #[default]
    Baseline,
    Interharmonic,
    Fundamental,
    Combined,
    Whitened,
}

impl ScoringVariant {
    pub(crate) fn from_env() -> Self {
        match std::env::var("PITCH_BENCH_SIEVE_VARIANT").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("baseline") => Self::Baseline,
            Ok("a") => Self::Interharmonic,
            Ok("b") => Self::Fundamental,
            Ok("c") => Self::Combined,
            Ok("h5") => Self::Whitened,
            value => panic!("PITCH_BENCH_SIEVE_VARIANT must be baseline/a/b/c/h5, got {value:?}"),
        }
    }
}

/// Weight of the measured → predicted sum against the predicted → measured one (TWM's ρ).
const UNEXPLAINED_PEAK_WEIGHT: f32 = 1.0;

/// Normalised score under which the frame is called unvoiced.
///
/// A trade, not a truth: RPA counts only voiced frames and rewards answering always, while
/// "pitch in silence" punishes it. Set on the corpus together with
/// `MISSING_HARMONIC_PENALTY`, whose shortfall charge lowers every score — see
/// `docs/harmonic_sieve.md` for the run that chose it.
const VOICING_THRESHOLD: f32 = 0.1;

/// Least-squares refinement passes per candidate. Each re-matches the peaks at the refined
/// f0, so a candidate that started a bin off can pull in partials it first missed.
const REFINE_PASSES: usize = 2;

/// How far refinement may move a candidate, in semitones. It is a fit, not a search: a
/// candidate that wants to move further was the wrong candidate, and another seed covers it.
const REFINE_LIMIT_SEMITONES: f32 = 0.5;

/// Floor under `|X|` before the log, as a ratio to full scale (−200 dB). Numerical only — it
/// keeps an all-zero bin finite for the parabola; [`SILENCE`] is the real decision.
const MAGNITUDE_FLOOR: f32 = 1e-10;

/// A level relative to digital full scale: 0 dBFS is a full-scale sine.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct Dbfs(f32);

/// A harmonic number, `1` being the fundamental. Indexes the slot vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Harmonic(usize);

impl Harmonic {
    /// The harmonic of `f0` nearest to `hz`. Never 0: a peak below `f0/2` is assigned to the
    /// fundamental, where it lands far outside the mesh and counts as unexplained.
    fn nearest(hz: Hz, f0: Hz) -> Harmonic {
        Harmonic(((hz.0 / f0.0).round() as usize).max(1))
    }

    fn hz(self, f0: Hz) -> Hz {
        Hz(self.0 as f32 * f0.0)
    }
}

/// One spectral peak, refined between bins.
#[derive(Clone, Copy, Debug)]
struct Peak {
    hz:     Hz,
    /// Positive dB above the frame floor, or above the local mean in H5.
    weight: f32,
}

/// The best peak a candidate found for one of its harmonic slots.
#[derive(Clone, Copy, Debug)]
struct SlotMatch {
    /// `weight · mesh` — what this slot adds to predicted → measured.
    contribution: f32,
    hz:           Hz,
    weight:       f32,
}

/// One candidate, matched against the frame's peaks.
#[derive(Clone, Copy, Debug)]
struct Verdict {
    f0:    Hz,
    /// `≤ 1`; 1 = every slot up to the top peak holds exactly one peak dead on its harmonic,
    /// and every peak is on a harmonic.
    score: f32,
    /// The weighted least-squares f0 over the slots that matched.
    fit:   Hz,
}

/// A frame's answer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SieveEstimate {
    pub(crate) hz:    Hz,
    /// The winning candidate's normalised score — see [`Verdict::score`].
    pub(crate) score: f32,
}

pub(crate) struct HarmonicSieve {
    variant:     ScoringVariant,
    sample_rate: SampleRate,
    min_hz:      Hz,
    max_hz:      Hz,
    taper:       Vec<f32>,
    /// `|X|` of a full-scale sine under `taper`: `Σ taper / 2`. Turns magnitude into dBFS.
    full_scale:  f32,
    fft:         Arc<dyn RealToComplex<f32>>,
    /// The newest `taper.len()` samples, oldest first.
    history:     Vec<f32>,

    // Scratch, reused frame to frame so the audio thread allocates nothing per frame.
    input:    Vec<f32>,
    fft_out:  Vec<Complex32>,
    work:     Vec<Complex32>,
    /// dBFS per bin, DC..Nyquist.
    levels:   Vec<f32>,
    /// H5 local contrast; raw levels still select and locate peaks and gate silence.
    contrast: Vec<f32>,
    padded:   Vec<f32>,
    peaks:    Vec<Peak>,
    /// Indexed by [`Harmonic`]; slot 0 is never used. `None` = the candidate predicts this
    /// harmonic and no peak was assigned to it, which is a verdict, not a gap in the data.
    slots:    Vec<Option<SlotMatch>>,
}

impl HarmonicSieve {
    /// A sieve searching `min_hz..=max_hz`.
    ///
    /// The range is the instrument's, and it matters twice: `min_hz` sets the window length
    /// (`PERIODS_PER_WINDOW` of its periods) and both ends bound the candidates, so a pitch
    /// outside the range cannot be reported at all.
    pub(crate) fn new(sample_rate: SampleRate, min_hz: Hz, max_hz: Hz) -> Self {
        Self::with_variant(sample_rate, min_hz, max_hz, ScoringVariant::Baseline)
    }

    pub(crate) fn with_variant(
        sample_rate: SampleRate,
        min_hz: Hz,
        max_hz: Hz,
        variant: ScoringVariant,
    ) -> Self {
        assert!(
            0.0 < min_hz.0 && min_hz.0 < max_hz.0 && max_hz.0 < sample_rate.hz() / 2.0,
            "sieve range {min_hz:?}..{max_hz:?} must sit inside (0, Nyquist) at {sample_rate:?}"
        );
        let size = ((PERIODS_PER_WINDOW * sample_rate.hz() / min_hz.0).ceil() as usize).next_power_of_two();
        let taper = hann_taper(size);
        let full_scale = taper.iter().sum::<f32>() / 2.0;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(size);
        Self {
            variant,
            sample_rate,
            min_hz,
            max_hz,
            full_scale,
            fft_out: fft.make_output_vec(),
            work: fft.make_scratch_vec(),
            fft,
            history: vec![0.0; size],
            input: vec![0.0; size],
            levels: vec![0.0; size / 2 + 1],
            contrast: vec![0.0; size / 2 + 1],
            padded: vec![0.0; size / 2 + whitening_bins(sample_rate.hz() / size as f32)],
            peaks: Vec::with_capacity(size / 2),
            slots: vec![None; MAX_HARMONICS + 1],
            taper,
        }
    }

    /// Feed the newest samples; the frame is computed on demand by [`Self::estimate`].
    pub(crate) fn process_samples(&mut self, samples: &[f32]) {
        slide_in(&mut self.history, samples);
    }

    /// How late an estimate is: half the window, since the window ends at the newest sample.
    pub(crate) fn delay_seconds(&self) -> f32 {
        self.taper.len() as f32 / 2.0 / self.sample_rate.hz()
    }

    /// Estimate f0 over the history as it stands. `None` = silence, no peaks, or no
    /// candidate scoring above [`VOICING_THRESHOLD`].
    pub(crate) fn estimate(&mut self) -> Option<SieveEstimate> {
        let bin_hz = self.sample_rate.hz() / self.taper.len() as f32;
        self.fill_levels();
        if !self.pick_peaks(bin_hz) {
            return None;
        }

        // Seeds are the strongest peaks; `pick_peaks` leaves them sorted strongest first.
        let mut best: Option<Verdict> = None;
        for seed in 0..CANDIDATE_SEEDS.min(self.peaks.len()) {
            let seed_hz = self.peaks[seed].hz;
            let mut k = 1.0f32;
            loop {
                let f0 = Hz(seed_hz.0 / k);
                k += 1.0;
                if f0.0 < self.min_hz.0 {
                    break;
                }
                if f0.0 > self.max_hz.0 {
                    continue;
                }
                let Some(verdict) = self.refine(f0, bin_hz) else {
                    continue;
                };
                if best.is_none_or(|b| verdict.score > b.score) {
                    best = Some(verdict);
                }
            }
        }

        let best = best?;
        (best.score >= VOICING_THRESHOLD).then_some(SieveEstimate {
            hz:    best.f0,
            score: best.score,
        })
    }

    /// Window the history, FFT it, and store each bin's level in dBFS.
    fn fill_levels(&mut self) {
        for (slot, (sample, taper)) in self.input.iter_mut().zip(self.history.iter().zip(&self.taper)) {
            *slot = sample * taper;
        }
        // Contractual: the only errors are length mismatches, and every length here is the
        // plan's own `make_*_vec` size, fixed at construction.
        self.fft
            .process_with_scratch(&mut self.input, &mut self.fft_out, &mut self.work)
            .unwrap();
        for (level, bin) in self.levels.iter_mut().zip(&self.fft_out) {
            *level = 20.0 * (bin.norm() / self.full_scale).max(MAGNITUDE_FLOOR).log10();
        }
    }

    /// Collect the frame's peaks into `self.peaks`, strongest first, at most [`MAX_PEAKS`].
    /// `false` when the frame is silent or shows no peak at all.
    fn pick_peaks(&mut self, bin_hz: f32) -> bool {
        // Below half the lowest candidate a peak can only ever be unexplained (rumble, DC
        // leakage); above the top candidate's top harmonic it is never judged. Neither is
        // allowed to set the frame's maximum, which is what the floor hangs from.
        let lo = ((0.5 * self.min_hz.0 / bin_hz).floor() as usize).max(1);
        let hi = ((MAX_HARMONICS as f32 * self.max_hz.0 / bin_hz).ceil() as usize).min(self.levels.len() - 2);
        let loudest = Dbfs(self.levels[lo..=hi].iter().copied().fold(f32::MIN, f32::max));
        if loudest < SILENCE {
            return false;
        }
        let floor = loudest.0 - DYNAMIC_RANGE_DB;
        if self.variant == ScoringVariant::Whitened {
            subtract_moving_mean(
                &self.levels,
                whitening_bins(bin_hz),
                &mut self.contrast,
                &mut self.padded,
            );
        }

        self.peaks.clear();
        for i in lo..=hi {
            let level = self.levels[i];
            if level > floor && level > self.levels[i - 1] && level >= self.levels[i + 1] {
                let (position, height) = parabolic_vertex(&self.levels, i);
                let weight = if self.variant == ScoringVariant::Whitened {
                    // Preserve the raw parabola's frequency and height. Interpolate the
                    // local mean at that position; only the vote/seed ranking changes.
                    let left = position.floor() as usize;
                    let fraction = position - left as f32;
                    let mean_left = self.levels[left] - self.contrast[left];
                    let mean_right = self.levels[left + 1] - self.contrast[left + 1];
                    height - (mean_left + fraction * (mean_right - mean_left))
                } else {
                    height - floor
                };
                // The matcher requires positive votes. A raw local maximum can sit
                // below a broad envelope's mean, and carries no H5 evidence then.
                if weight <= 0.0 {
                    continue;
                }
                self.peaks.push(Peak {
                    hz: Hz(position * bin_hz),
                    weight,
                });
            }
        }
        self.peaks.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        self.peaks.truncate(MAX_PEAKS);
        !self.peaks.is_empty()
    }

    /// Match `f0`, then refine it by least squares while that improves the score. `None` as
    /// [`match_candidate`]'s.
    fn refine(&mut self, f0: Hz, bin_hz: f32) -> Option<Verdict> {
        let lo = f0.0 * (-REFINE_LIMIT_SEMITONES / 12.0).exp2();
        let hi = f0.0 * (REFINE_LIMIT_SEMITONES / 12.0).exp2();
        let mut verdict = match_candidate(&self.peaks, f0, bin_hz, &mut self.slots, self.variant)?;
        for _ in 0..REFINE_PASSES {
            let fitted = Hz(verdict.fit.0.clamp(lo, hi));
            // Refinement moves f0 by at most half a semitone, so the peaks a candidate
            // judged stay inside `MAX_HARMONICS` of it; a `None` here would still be an
            // honest "the fit explains nothing", and keeps the unrefined verdict.
            match match_candidate(&self.peaks, fitted, bin_hz, &mut self.slots, self.variant) {
                Some(next) if next.score >= verdict.score => verdict = next,
                _ => break,
            }
        }
        Some(verdict)
    }
}

/// Score candidate `f0` against `peaks` — both directions of the two-way match — and fit a
/// refined f0 from the harmonics it matched. `slots` is scratch, overwritten.
///
/// `None` when every peak sits above harmonic [`MAX_HARMONICS`] of `f0`: the candidate is
/// then not wrong but unjudgeable, and has no score to compare.
fn match_candidate(
    peaks: &[Peak],
    f0: Hz,
    bin_hz: f32,
    slots: &mut [Option<SlotMatch>],
    variant: ScoringVariant,
) -> Option<Verdict> {
    slots.iter_mut().for_each(|slot| *slot = None);

    let mut total_weight = 0.0f32;
    let mut considered = 0usize;
    let mut top = Harmonic(1);
    // measured → predicted: every peak, judged by how close it sits to *some* harmonic.
    let mut explained = 0.0f32;
    for peak in peaks {
        let k = Harmonic::nearest(peak.hz, f0);
        if k.0 > MAX_HARMONICS {
            continue;
        }
        let contribution = peak.weight * mesh(peak.hz.0 - k.hz(f0).0, k, f0, bin_hz);
        explained += contribution;
        total_weight += peak.weight;
        considered += 1;
        top = top.max(k);
        // predicted → measured keeps only the best peak per harmonic: two peaks crowding one
        // slot are one partial and a neighbour, not twice the evidence.
        let slot = &mut slots[k.0];
        if slot.is_none_or(|s| contribution > s.contribution) {
            *slot = Some(SlotMatch {
                contribution,
                hz: peak.hz,
                weight: peak.weight,
            });
        }
    }
    if considered == 0 {
        return None;
    }

    // predicted → measured: every harmonic up to the highest one any peak reached. A slot
    // pays for its *shortfall* below a typical peak, not merely for being empty: with 60 dB
    // of range, some junk peak (noise, a window sidelobe, a sympathetic string) lands in
    // nearly every slot, and if any occupant cleared the slot, a sub-octave's odd slots
    // would almost never pay. An empty slot pays in full, a junk peak nearly in full, a real
    // partial at or above the typical weight not at all.
    let typical = total_weight / considered as f32;
    let mut predicted = 0.0f32;
    let (mut fit_num, mut fit_den) = (0.0f32, 0.0f32);
    for (k, slot) in slots.iter().enumerate().take(top.0 + 1).skip(1) {
        let found = slot.map_or(0.0, |matched| matched.contribution.max(0.0));
        let shortfall_weight =
            if k == 1 && matches!(variant, ScoringVariant::Fundamental | ScoringVariant::Combined) {
                FUNDAMENTAL_SHORTFALL_MULTIPLIER
            } else {
                1.0
            };
        predicted += found - MISSING_HARMONIC_PENALTY * shortfall_weight * (typical - found).max(0.0);
        // Only partials inside their mesh vote on the fit; a slot whose best peak is off to
        // the side would drag the fit toward a neighbour.
        if let Some(matched) = slot
            && matched.contribution > 0.0
        {
            let k = k as f32;
            fit_num += matched.weight * k * matched.hz.0;
            fit_den += matched.weight * k * k;
        }
    }

    if matches!(variant, ScoringVariant::Interharmonic | ScoringVariant::Combined) {
        // Same positive mesh and best-peak policy as the harmonic slots. Fractional
        // positions use their own frequency for the relative bandwidth. A valley never
        // rewards energy outside it, and its penalty is not multiplied by the number of
        // neighbouring noise peaks. The existing measured → predicted term is unchanged.
        for (ratio, weight) in INTERHARMONIC_VALLEYS {
            let target = Hz(ratio * f0.0);
            let energy = peaks
                .iter()
                .map(|peak| peak.weight * mesh(peak.hz.0 - target.0, Harmonic(1), target, bin_hz).max(0.0))
                .fold(0.0, f32::max);
            predicted -= weight * energy;
        }
    }

    let score =
        (predicted + UNEXPLAINED_PEAK_WEIGHT * explained) / ((1.0 + UNEXPLAINED_PEAK_WEIGHT) * total_weight);
    // No slot inside its mesh: nothing to fit, so the fit is the candidate itself — which
    // makes the refinement step a no-op rather than a jump.
    let fit = if fit_den > 0.0 { Hz(fit_num / fit_den) } else { f0 };
    Some(Verdict { f0, score, fit })
}

/// How well a peak `deviation` Hz off harmonic `k` of `f0` sits in that harmonic's mesh.
///
/// A Gaussian of width `σ_k = √((MESH_SIGMA_BINS·Δf)² + (MESH_SIGMA_RELATIVE·k·f0)²)`, minus
/// its own mean over one harmonic spacing, rescaled so a dead-on peak scores 1. The mean is
/// what makes the mesh honest: averaged over a peak that could land anywhere in
/// `[−f0/2, f0/2]` it is exactly 0, so a cluttered spectrum earns nothing for being
/// cluttered, and a peak between two harmonics scores below zero.
fn mesh(deviation: f32, k: Harmonic, f0: Hz, bin_hz: f32) -> f32 {
    let sigma = (MESH_SIGMA_BINS * bin_hz).hypot(MESH_SIGMA_RELATIVE * k.0 as f32 * f0.0);
    let gaussian = (-0.5 * (deviation / sigma).powi(2)).exp();
    // (1/f0)·∫_{−f0/2}^{f0/2} exp(−d²/2σ²) dd = (σ·√(2π) / f0) · erf(f0 / (2√2·σ))
    let mean = sigma * (2.0 * std::f32::consts::PI).sqrt() / f0.0
        * erf(f0.0 / (2.0 * std::f32::consts::SQRT_2 * sigma));
    (gaussian - mean) / (1.0 - mean)
}

/// The error function for `x ≥ 0`, Abramowitz & Stegun 7.1.26 (|error| < 1.5·10⁻⁷).
fn erf(x: f32) -> f32 {
    debug_assert!(x >= 0.0, "erf is only needed, and only implemented, for x >= 0");
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly =
        t * (0.254_829_6 + t * (-0.284_496_7 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    1.0 - poly * (-x * x).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: SampleRate = SampleRate(44_100);

    /// The violin's range with a semitone of margin each side: G3 − 1 .. E7 + 1.
    fn violin_sieve() -> HarmonicSieve {
        let variant = ScoringVariant::from_env();
        if variant == ScoringVariant::Baseline {
            HarmonicSieve::new(SR, Hz(185.0), Hz(2794.0))
        } else {
            HarmonicSieve::with_variant(SR, Hz(185.0), Hz(2794.0), variant)
        }
    }

    /// `len` samples of a harmonic tone: `amplitudes[i]` is harmonic `i + 1`.
    fn tone(f0: f32, amplitudes: &[f32], len: usize) -> Vec<f32> {
        (0..len)
            .map(|n| {
                let t = n as f32 / SR.hz();
                amplitudes
                    .iter()
                    .enumerate()
                    .map(|(i, a)| a * (std::f32::consts::TAU * (i + 1) as f32 * f0 * t).sin())
                    .sum::<f32>()
                    * 0.2
            })
            .collect()
    }

    fn estimate(sieve: &mut HarmonicSieve, signal: &[f32]) -> Option<SieveEstimate> {
        sieve.process_samples(signal);
        sieve.estimate()
    }

    fn cents(estimate: Hz, truth: f32) -> f32 {
        1200.0 * (estimate.0 / truth).log2()
    }

    /// A sawtooth-like series: 1/k over twelve harmonics.
    fn bright() -> Vec<f32> {
        (1..=12).map(|k| 1.0 / k as f32).collect()
    }

    #[test]
    fn h5_width_tracks_hz_across_windows_and_sample_rates() {
        for sr in [44_100.0, 48_000.0] {
            for size in [1024, 2048, 4096] {
                let bin_hz = sr / size as f32;
                let width = whitening_bins(bin_hz);
                assert_eq!(width % 2, 1);
                assert!((width as f32 * bin_hz - WHITENING_WIDTH.0).abs() <= bin_hz);
            }
        }
        assert_eq!(whitening_bins(SR.hz() / 1024.0), 7);
        assert_eq!(whitening_bins(SR.hz() / 2048.0), 15);
        assert_eq!(whitening_bins(SR.hz() / 4096.0), 31);
    }

    /// Direct arithmetic oracle: a louder peak on a broad formant loses to an
    /// isolated peak. Raw parabolas keep both frequencies; only votes change.
    #[test]
    fn h5_weights_and_seed_order_match_local_contrast() {
        let make = |variant| {
            let mut sieve = HarmonicSieve::with_variant(SR, Hz(65.0), Hz(2093.0), variant);
            sieve.levels.fill(-60.0);
            sieve.levels[8..=24].fill(-30.0);
            sieve.levels[16] = -20.0;
            sieve.levels[64] = -25.0;
            assert!(sieve.pick_peaks(SR.hz() / 2048.0));
            sieve
        };
        let baseline = make(ScoringVariant::Baseline);
        let whitened = make(ScoringVariant::Whitened);
        let bin_hz = SR.hz() / 2048.0;
        assert_eq!(baseline.peaks[0].hz, Hz(16.0 * bin_hz));
        assert_eq!(whitened.peaks[0].hz, Hz(64.0 * bin_hz));
        for (bin, expected) in [(16, 10.0 * 14.0 / 15.0), (64, 35.0 * 14.0 / 15.0)] {
            let hz = Hz(bin as f32 * bin_hz);
            let peak = whitened.peaks.iter().find(|peak| peak.hz == hz).unwrap();
            assert!((peak.weight - expected).abs() < 1e-3, "{peak:?}");
        }
    }

    #[test]
    fn erf_matches_known_values() {
        for (x, expected) in [
            (0.0, 0.0),
            (0.5, 0.520_499_9),
            (1.0, 0.842_700_8),
            (2.0, 0.995_322_3),
        ] {
            assert!((erf(x) - expected).abs() < 1e-6, "erf({x}) = {}", erf(x));
        }
    }

    /// The property the whole scoring rests on: a peak landing uniformly at random in one
    /// harmonic spacing earns nothing on average.
    #[test]
    fn mesh_has_zero_mean_over_one_spacing() {
        let (f0, bin_hz) = (Hz(196.0), SR.hz() / 1024.0);
        for k in [Harmonic(1), Harmonic(6), Harmonic(20)] {
            let steps = 20_000;
            let mean = (0..steps)
                .map(|i| mesh(f0.0 * ((i as f32 + 0.5) / steps as f32 - 0.5), k, f0, bin_hz))
                .sum::<f32>()
                / steps as f32;
            assert!(mean.abs() < 1e-3, "mesh mean at {k:?} = {mean}");
            assert!((mesh(0.0, k, f0, bin_hz) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn finds_open_g_within_two_cents() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        let found = estimate(&mut sieve, &tone(196.0, &bright(), len)).unwrap();
        assert!(cents(found.hz, 196.0).abs() < 2.0, "{found:?}");
    }

    #[test]
    fn tracks_every_semitone_of_the_violin() {
        for midi in 55..=100 {
            let f0 = 440.0 * ((midi as f32 - 69.0) / 12.0).exp2();
            let mut sieve = violin_sieve();
            let len = sieve.taper.len();
            let found = estimate(&mut sieve, &tone(f0, &bright(), len)).unwrap();
            assert!(cents(found.hz, f0).abs() < 5.0, "MIDI {midi}: {found:?}");
        }
    }

    /// The bowed G string's own trap: a fundamental far weaker than its second partial.
    #[test]
    fn weak_fundamental_is_not_an_octave_up() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        let amplitudes = [0.05, 1.0, 0.6, 0.5, 0.3, 0.3, 0.2, 0.15];
        let found = estimate(&mut sieve, &tone(196.0, &amplitudes, len)).unwrap();
        assert!(cents(found.hz, 196.0).abs() < 5.0, "{found:?}");
    }

    #[test]
    fn missing_fundamental_is_still_found() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        let mut amplitudes = bright();
        amplitudes[0] = 0.0;
        let found = estimate(&mut sieve, &tone(220.0, &amplitudes, len)).unwrap();
        assert!(cents(found.hz, 220.0).abs() < 5.0, "{found:?}");
    }

    /// An arithmetic oracle on exact peaks: the absent fundamental costs one extra
    /// typical peak in b; a half-frequency peak costs precisely 1.8 times its weight in a.
    #[test]
    fn h4_score_terms_match_hand_calculated_penalties() {
        let mut slots = vec![None; MAX_HARMONICS + 1];
        let peaks = [
            Peak {
                hz:     Hz(400.0),
                weight: 30.0,
            },
            Peak {
                hz:     Hz(600.0),
                weight: 30.0,
            },
        ];
        let baseline = match_candidate(&peaks, Hz(200.0), 1.0, &mut slots, ScoringVariant::Baseline).unwrap();
        let b = match_candidate(&peaks, Hz(200.0), 1.0, &mut slots, ScoringVariant::Fundamental).unwrap();
        assert!((baseline.score - 0.625).abs() < 1e-6);
        assert!((b.score - 0.25).abs() < 1e-6);
        let peaks = [
            Peak {
                hz:     Hz(200.0),
                weight: 30.0,
            },
            Peak {
                hz:     Hz(100.0),
                weight: 10.0,
            },
        ];
        let score = |variant, slots: &mut [Option<SlotMatch>]| {
            match_candidate(&peaks, Hz(200.0), 1.0, slots, variant)
                .unwrap()
                .score
        };
        let base = score(ScoringVariant::Baseline, &mut slots);
        let a = score(ScoringVariant::Interharmonic, &mut slots);
        let c = score(ScoringVariant::Combined, &mut slots);
        assert!((base - a - 18.0 / 80.0).abs() < 1e-6);
        assert!((a - c).abs() < 1e-6, "no fundamental shortfall in this example");
    }

    /// Real-audio acceptance probe for H4. Truth is the played first-position band,
    /// not an inferred per-frame note; only voiced estimates are its denominator.
    #[test]
    #[ignore = "H4 probe: needs testdata/a_string_trill.wav"]
    fn h4_live_a_trill() {
        let variant = ScoringVariant::from_env();
        let mut reader = hound::WavReader::open("testdata/a_string_trill.wav").unwrap();
        let sr = SampleRate(reader.spec().sample_rate);
        assert_eq!(reader.spec().channels, 1);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let mut sieve = HarmonicSieve::with_variant(sr, Hz(174.6), Hz(2794.0), variant);
        let (mut voiced, mut position, mut down, mut unvoiced) = (0, 0, 0, 0);
        for chunk in samples.chunks_exact(256) {
            sieve.process_samples(chunk);
            if let Some(found) = sieve.estimate() {
                voiced += 1;
                let midi = found.hz.to_midi(Hz::A4_STANDARD).0;
                position += i32::from((68.5..=76.5).contains(&midi));
                down += i32::from((56.5..=64.5).contains(&midi));
            } else {
                unvoiced += 1;
            }
        }
        println!(
            "H4 A trill {variant:?}: voiced={voiced}, in_position={position}, octave_down={down}, unvoiced={unvoiced}; in_position={:.4}%, down={:.4}%",
            100.0 * position as f32 / voiced as f32,
            100.0 * down as f32 / voiced as f32
        );
        assert!(voiced > 0);
    }

    /// Phase sweep of an abrupt harmonic-tone change on the corpus range and hop.
    /// Reports first correct frame after the change (not group delay); the correct
    /// answer must persist for the rest of the probe, so an early isolated guess loses.
    #[test]
    #[ignore = "H4 latency measurement, 16 onset phases"]
    fn h4_step_latency() {
        let variant = ScoringVariant::from_env();
        let hop = (SR.hz() * 0.016) as usize;
        for target in [493.8833, 880.0] {
            let mut delays = Vec::new();
            for phase in 0..16 {
                let onset = 10 * hop + phase * hop / 16;
                let mut samples = tone(440.0, &bright(), onset);
                samples.extend(tone(target, &bright(), 10 * hop));
                let mut sieve = HarmonicSieve::with_variant(SR, Hz(65.0), Hz(2093.0), variant);
                let mut settled = None;
                for (index, chunk) in samples.chunks_exact(hop).enumerate() {
                    sieve.process_samples(chunk);
                    let end = (index + 1) * hop;
                    if end < onset {
                        continue;
                    }
                    if sieve
                        .estimate()
                        .is_some_and(|found| cents(found.hz, target).abs() <= 50.0)
                    {
                        settled.get_or_insert(end);
                    } else {
                        settled = None;
                    }
                }
                delays.push(1000.0 * (settled.expect("step must settle") - onset) as f32 / SR.hz());
            }
            println!("H4 step {variant:?} 440->{target} Hz: {delays:?} ms");
        }
    }

    /// Only even harmonics present is, physically, a tone an octave up — and must read so.
    #[test]
    fn even_harmonics_alone_read_as_the_octave() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        let amplitudes: Vec<f32> = (1..=12)
            .map(|k| if k % 2 == 0 { 1.0 / k as f32 } else { 0.0 })
            .collect();
        let found = estimate(&mut sieve, &tone(196.0, &amplitudes, len)).unwrap();
        assert!(cents(found.hz, 392.0).abs() < 5.0, "{found:?}");
    }

    #[test]
    fn silence_has_no_pitch() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        assert!(estimate(&mut sieve, &vec![0.0; len]).is_none());
    }

    /// Dump the sieve's track over a WAV excerpt as CSV, for `tools/pitch_roll_plot.py`.
    ///
    /// ```text
    /// SIEVE_DUMP_WAV=take.wav SIEVE_DUMP_FROM=26 SIEVE_DUMP_TO=36 SIEVE_DUMP_CSV=take.csv \
    ///   cargo test --release --lib sieve::tests::dump_track -- --ignored --nocapture
    /// ```
    ///
    /// Optional: `SIEVE_DUMP_RANGE_HZ=lo,hi` (default: violin, F3..F7). Mono 16-bit WAV.
    ///
    /// Each row is stamped at the **centre** of its window (`end − delay_seconds`), so the
    /// trace lines up with the audio rather than trailing it by half a window. The hop is
    /// 256 samples, finer than the benchmark's 16 ms, because a plot of vibrato wants it.
    #[test]
    #[ignore = "a probe, not a check: needs SIEVE_DUMP_* — see the doc comment"]
    fn dump_track() {
        use std::io::Write;

        let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("set {key}"));
        let seconds = |key: &str| env(key).parse::<f32>().unwrap();
        let (lo, hi) = std::env::var("SIEVE_DUMP_RANGE_HZ")
            .map(|spec| {
                let (lo, hi) = spec.split_once(',').unwrap();
                (lo.trim().parse().unwrap(), hi.trim().parse().unwrap())
            })
            .unwrap_or((174.6, 2794.0));

        let mut reader = hound::WavReader::open(env("SIEVE_DUMP_WAV")).unwrap();
        let sample_rate = SampleRate(reader.spec().sample_rate);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let mut sieve = HarmonicSieve::with_variant(sample_rate, Hz(lo), Hz(hi), ScoringVariant::from_env());
        let delay = sieve.delay_seconds();
        let (from, to) = (seconds("SIEVE_DUMP_FROM"), seconds("SIEVE_DUMP_TO"));

        let hop = 256usize;
        // Start a window early so the first row already has a full window behind it.
        let start = (((from - 2.0 * delay) * sample_rate.hz()).max(0.0) as usize) / hop * hop;
        let end = ((to + delay) * sample_rate.hz()) as usize;
        let mut out = std::io::BufWriter::new(std::fs::File::create(env("SIEVE_DUMP_CSV")).unwrap());
        writeln!(out, "seconds,midi,score,level_dbfs").unwrap();
        let mut fed = start;
        while fed + hop <= end.min(samples.len()) {
            let chunk = &samples[fed..fed + hop];
            sieve.process_samples(chunk);
            fed += hop;
            let centre = fed as f32 / sample_rate.hz() - delay;
            if centre < from || centre > to {
                continue;
            }
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / hop as f32).sqrt();
            let level = 20.0 * rms.max(MAGNITUDE_FLOOR).log10();
            match sieve.estimate() {
                Some(found) => {
                    let midi = found.hz.to_midi(Hz::A4_STANDARD).0;
                    writeln!(out, "{centre:.4},{midi:.4},{:.3},{level:.1}", found.score).unwrap();
                }
                None => writeln!(out, "{centre:.4},,,{level:.1}").unwrap(),
            }
        }
    }

    /// White noise has peaks everywhere and a harmonic series nowhere: the zero-mean meshes
    /// should keep every candidate's score near zero.
    #[test]
    fn white_noise_has_no_pitch() {
        let mut sieve = violin_sieve();
        let len = sieve.taper.len();
        // A fixed LCG, so the test cannot flake.
        let mut state = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        assert!(estimate(&mut sieve, &noise).is_none());
    }
}
