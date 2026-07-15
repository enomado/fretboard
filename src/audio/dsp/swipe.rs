//! SWIPE′ pitch salience (Camacho & Harris, 2008) over the resonator bank's column.
//!
//! Design and its justification: `docs/swipe_salience_design.md`. The short version of
//! why this module exists: the previous scorer
//! (`analysis_math::resonator_fundamental`) added credit at a candidate's harmonics and
//! could never subtract. Camacho's thesis enumerates exactly that function and exactly
//! its failure (Fig. 3-13): a **reward-only comb peaks at sub *and supra*harmonics**.
//! The violin's phantom octave *is* that supraharmonic peak — there is no term in a
//! reward-only comb that can punish 2·f0, because every harmonic of 2·f0 is also a
//! harmonic of f0.
//!
//! # The two devices, and which one does what
//!
//! - **Negative valleys** kill the *supra*-harmonic (our bug). A candidate at 2·f0 must
//!   explain the energy at 3·f0, 5·f0 … — and those land in *its own* valleys, so they
//!   subtract from its score. For an open G (196 Hz) misread as G4 (392), the real
//!   harmonics at 588 and 980 sit at `f'/f` = 1.5 and 2.5: dead centre of two valleys.
//! - **First-and-prime harmonics** kill the *sub*-harmonic. A candidate below f0 can
//!   match at most **one** of f0's harmonics with a prime harmonic of its own, so it
//!   cannot accumulate credit the way a full comb lets it.
//!
//! Neither device needs the fundamental partial to be present. Candidate f0 scores from
//! 196, 392, 588, 784 … — all positive lobes — **even when the 196 bin is empty**, which
//! is the whole point on a violin: the instrument barely radiates its own open-G
//! fundamental. This is why there is no `FUNDAMENTAL_FLOOR` here: requiring a candidate's
//! own bin to carry energy is precisely the assumption a bowed G string violates.
//!
//! # Why this is one fixed vector
//!
//! `K` depends on `f` and `f'` only through the ratio `r = f'/f`, and the bank's output
//! grid is uniform in log-frequency. Put `u = log₂(r)`: the lobe positions (`log₂ h`),
//! the lobe edges (`log₂(h ± ¼)`) and the envelope shape (`2^(−u/2)`) are then all
//! **independent of `f`**. So the kernel is built once and the whole salience curve is a
//! cross-correlation. The `1/√f` factor that remains cancels between numerator and
//! denominator (see [`SwipeKernel::salience`]).
//!
//! # Contract
//!
//! The column handed to [`SwipeKernel::salience_curve`] must be **raw magnitudes** on a
//! log-frequency grid of `bins_per_semitone`. This module applies SWIPE's own `√`
//! warping ([`sqrt_warp`]) and must never be fed the display's `gamma`-warped copy: that
//! `gamma` is a user-facing waterfall-contrast slider, and letting it reach the detector
//! is how a display knob came to steer the octave decision (design §1.4).

use std::f32::consts::TAU;

use super::analysis_math::parabolic_tau;
use super::pitch::{
    TRACKED_MAX_MIDI,
    TRACKED_MIN_MIDI,
};

/// Harmonics the SWIPE′ kernel keeps: `{1} ∪ primes`.
///
/// The ceiling is **not** a tuning knob — it falls out of the grid. Lobe `h` spans
/// `12·log₂((h+¼)/(h−¼))` semitones, which shrinks as `1/h`: at 8 bins/semitone that is
/// 71 bins for h=1, 6.3 for h=11, 5.3 for h=13 and 4.1 for h=17. Past h≈13 a cosine lobe
/// is sampled by ~5 bins and the kernel stops being represented honestly, so 13 is where
/// this grid runs out. If more harmonics are ever wanted the answer is a finer grid, not
/// a bigger constant here.
const HARMONICS: [usize; 7] = [1, 2, 3, 5, 7, 11, 13];

/// Per-harmonic weight envelope at ratio `r`, for the lobe belonging to harmonic `h`.
///
/// Each harmonic owns its **peak** outright and **half** of each valley beside it —
/// "each valley is associated to two peaks" (thesis §3.9). Summing these over the kept
/// harmonics reproduces eq. 3-12 exactly when every harmonic is present (valley between
/// two kept peaks: ½+½ = 1; first and last valleys: ½, i.e. halved, as the equation
/// requires) *and* gives SWIPE′'s stated weights when they are not: only the valleys at
/// r=1.5 and r=2.5 keep −1, because only there are both neighbours kept.
fn lobe_weight(r: f32, h: usize) -> f32 {
    let distance = (r - h as f32).abs();
    if distance < 0.25 {
        1.0 // the peak
    } else if distance < 0.75 {
        0.5 // half of the valley on this side
    } else {
        0.0
    }
}

/// A SWIPE′ kernel sampled once onto a log-frequency grid.
#[derive(Debug)]
pub(crate) struct SwipeKernel {
    /// `weights[j]` applies at bin offset `first_offset + j` from the candidate's bin.
    /// Already carries the `1/√r` envelope (SWIPE's harmonic decay, `p = 1/2` — the
    /// thesis tested `p ∈ {1/2, 1, 2}` and found ½ best; the old comb used `p = 1`).
    weights:            Vec<f32>,
    /// Bin offset of `weights[0]` relative to the candidate. Negative: the kernel
    /// reaches *below* the candidate, where its first (halved) valley punishes energy at
    /// `f/2` — the other half of the anti-supraharmonic story.
    first_offset:       isize,
    /// `positive_sq_prefix[j] = Σ_{i<j} max(0, weights[i])²`.
    ///
    /// SWIPE normalizes by the L2 norm of the kernel's **positive part only**, and the
    /// valid offsets for a candidate always form a contiguous run, so a prefix sum makes
    /// the truncation-aware norm O(1). This is what the old comb got wrong: it `break`ed
    /// out of range and then compared unnormalized scores, so a candidate near the top of
    /// the grid summed fewer harmonics and lost for a reason unrelated to the signal.
    positive_sq_prefix: Vec<f32>,
}

impl SwipeKernel {
    /// Build the kernel for a grid with `bins_per_semitone` bins per semitone.
    pub(crate) fn new(bins_per_semitone: f32) -> Self {
        let bins_per_octave = 12.0 * bins_per_semitone;
        // Support is r ∈ (h_first − ¾, h_last + ¾) = (0.25, 13.75): the outermost
        // half-valleys of the outermost kept harmonics. The lower edge at ¼ is exactly
        // eq. 3-12's truncated DC lobe.
        let r_low = HARMONICS[0] as f32 - 0.75;
        let r_high = HARMONICS[HARMONICS.len() - 1] as f32 + 0.75;
        let first_offset = (bins_per_octave * r_low.log2()).floor() as isize;
        let last_offset = (bins_per_octave * r_high.log2()).ceil() as isize;

        let mut weights = Vec::with_capacity((last_offset - first_offset + 1) as usize);
        for offset in first_offset..=last_offset {
            let r = (offset as f32 / bins_per_octave).exp2();
            // K(r) = cos(2πr) · Σ_h w_h(r) — the cosine is the kernel; the weights just
            // say how much of each lobe this harmonic set is entitled to.
            let envelope: f32 = HARMONICS.iter().map(|&h| lobe_weight(r, h)).sum();
            let k = (TAU * r).cos() * envelope;
            // The `1/√f'` envelope of eq. 3-11, split as `1/√f · 1/√r`; the `1/√f` part
            // is constant per candidate and cancels in `salience`.
            weights.push(k / r.sqrt());
        }

        let mut positive_sq_prefix = Vec::with_capacity(weights.len() + 1);
        let mut running = 0.0f32;
        positive_sq_prefix.push(0.0);
        for &w in &weights {
            // max(0, K/√r)² = [max(0,K)]²/r — i.e. exactly eq. 3-11's `(1/f')[K⁺]²` term,
            // again with the constant `1/f` factored out.
            running += w.max(0.0).powi(2);
            positive_sq_prefix.push(running);
        }

        Self {
            weights,
            first_offset,
            positive_sq_prefix,
        }
    }

    /// Pitch strength of the candidate sitting at `candidate_bin`, against a **√-warped**
    /// column (see [`sqrt_warp`]).
    ///
    /// This is eq. 3-11's normalized inner product, less the `‖√|X|‖` factor: that norm
    /// is the same for every candidate in a frame, so it cannot move the argmax. Divide
    /// by [`spectrum_norm`] if an absolute 0..1 confidence is wanted rather than a
    /// ranking.
    ///
    /// Returns 0.0 when the candidate keeps no positive kernel mass in range at all
    /// (only possible for candidates right at the grid's edge).
    pub(crate) fn salience(&self, warped: &[f32], candidate_bin: usize) -> f32 {
        // Offsets whose bin exists. Contiguous by construction, which is what lets the
        // norm come off a prefix sum.
        let low = self.first_offset.max(-(candidate_bin as isize));
        let high = self
            .weights
            .len()
            .checked_sub(1)
            .map(|last| self.first_offset + last as isize)
            .unwrap()
            .min(warped.len() as isize - 1 - candidate_bin as isize);
        if high < low {
            return 0.0;
        }

        let mut numerator = 0.0f32;
        for offset in low..=high {
            let w = self.weights[(offset - self.first_offset) as usize];
            let bin = (candidate_bin as isize + offset) as usize;
            numerator += w * warped[bin];
        }

        let lo_index = (low - self.first_offset) as usize;
        let hi_index = (high - self.first_offset) as usize;
        let norm_sq = self.positive_sq_prefix[hi_index + 1] - self.positive_sq_prefix[lo_index];
        if norm_sq <= 0.0 {
            return 0.0;
        }
        numerator / norm_sq.sqrt()
    }

    /// Salience for every bin of the column — the whole curve, which is the natural
    /// output here (see the module docs) and the input an online Viterbi would want if
    /// the fast channel ever turns out to need temporal smoothing.
    pub(crate) fn salience_curve(&self, warped: &[f32]) -> Vec<f32> {
        (0..warped.len()).map(|bin| self.salience(warped, bin)).collect()
    }
}

/// Bins searched either side of a partial's predicted position when reading its fine
/// frequency.
///
/// Not a tolerance to tune: the salience's own refine is clamped to ±½ bin, and a
/// log-frequency axis shifts *every* harmonic of a candidate by the same amount, so one
/// bin covers the coarse estimate's whole error for all of them. The second bin covers
/// [`splat_linear`]'s two-bin footprint.
///
/// [`splat_linear`]: super::analysis_math::splat_linear
const PARTIAL_SEARCH_BINS: isize = 2;

/// Fine pitch for an already-decided harmonic series: read the frequency off the
/// **strongest partial** and divide by its harmonic number.
///
/// Two things this deliberately does *not* do, both of which were tried and measured:
///
/// - **Not the salience curve's apex.** That curve's peak is *broad* — the h=1 lobe alone
///   spans 71 bins at 8 bins/semitone — so its parabolic apex sits nowhere near a
///   partial's true position. Refining it throws away exactly the sub-bin precision the
///   Δφ reassignment exists to provide; measured at **23 cents** of error on a clean A4,
///   against a 20-cent budget (`resonator::fundamental_on_harmonic_tone_resolves_to_root`).
/// - **Not the fundamental's own peak** (what the old comb refined). That is the one
///   partial this app cannot rely on: on a bowed G it is nearly absent, which is the whole
///   reason SWIPE′ is here.
///
/// So the two jobs are split by what each source is actually good at: SWIPE′ says *which*
/// harmonic series, and the loudest member of that series — whichever it turns out to be —
/// says *exactly where*. On an open G that is the 2nd harmonic at 392 Hz, loud and cleanly
/// reassigned, which yields **better** cents than the old code could ever read off the
/// 196 Hz fundamental it insisted on.
///
/// The estimator is a **centroid**, not a parabola, because of how the energy got here:
/// `splat_linear` distributes each resonator's magnitude *linearly* across two bins at its
/// reassigned position, and many resonators splat into the same neighbourhood. The
/// energy-weighted mean of that blob is precisely the mean reassigned frequency — which is
/// what the reassignment measured in the first place. A parabola would assume a symmetric
/// peak shape that a linear splat does not have (for a lone partial at bin 100.3 it
/// returns 99.93; the centroid returns 100.3).
fn refine_on_partials(magnitudes: &[f32], coarse_bin: f32, bins_per_semitone: f32) -> f32 {
    let bins_per_octave = 12.0 * bins_per_semitone;
    let mut best_magnitude = 0.0f32;
    let mut best_f0_bin = coarse_bin;

    for &h in &HARMONICS {
        let harmonic_offset = bins_per_octave * (h as f32).log2();
        let centre = (coarse_bin + harmonic_offset).round() as isize;
        let low = (centre - PARTIAL_SEARCH_BINS).max(0);
        let high = (centre + PARTIAL_SEARCH_BINS).min(magnitudes.len() as isize - 1);
        if high < low {
            continue;
        }

        let mut weight_sum = 0.0f32;
        let mut weighted_bin = 0.0f32;
        let mut peak = 0.0f32;
        for bin in low..=high {
            let magnitude = magnitudes[bin as usize];
            weight_sum += magnitude;
            weighted_bin += magnitude * bin as f32;
            peak = peak.max(magnitude);
        }
        // Compare partials by their peak, not their summed window: the sum grows with
        // whatever noise shares the window, the peak is the partial itself.
        if weight_sum <= 0.0 || peak <= best_magnitude {
            continue;
        }
        best_magnitude = peak;
        best_f0_bin = weighted_bin / weight_sum - harmonic_offset;
    }
    best_f0_bin
}

/// One bank frame's pitch evidence: SWIPE′'s salience curve, and the column it was scored
/// from.
///
/// This exists because the curve is the natural output of the algorithm (see the module
/// docs) and an argmax throws away everything the *next* frame's decision would want. The
/// remaining errors on a real violin are 4–6% near-ties — the right note losing to junk by a
/// few percent — and a scalar per frame cannot express a tie at all, let alone let
/// continuity break it. So the whole curve rides to [`super::melody`], where the Viterbi
/// lives.
///
/// The **column** rides along with it for a reason that is easy to miss: the decoded bin is
/// only a *coarse* f0 (it names the harmonic series), and the fine pitch is read off the
/// series' loudest partial by [`refine_on_partials`]. When continuity overrules the
/// per-frame argmax — which is the entire point — the fine pitch has to be re-read at the
/// bin that *won*, and that needs the partials. Carrying a scalar would silently cost the
/// cents on exactly the frames this phase exists to fix.
#[derive(Clone, Debug)]
pub(crate) struct SalienceFrame {
    /// Pitch strength per bin of the bank's output grid.
    curve:             Vec<f32>,
    /// The **raw** column — `gamma`-free, as the contract above demands.
    magnitudes:        Vec<f32>,
    min_midi:          f32,
    bins_per_semitone: f32,
    /// `‖√|X|‖₂`, the per-frame constant that turns a salience into a 0..1 confidence.
    norm:              f32,
}

impl SalienceFrame {
    /// Score one raw column. `None` for a column with no energy at all — silence is *not*
    /// decided here (see [`pick_fundamental`]).
    pub(crate) fn score(
        magnitudes: &[f32],
        min_midi: f32,
        bins_per_semitone: f32,
        kernel: &SwipeKernel,
    ) -> Option<Self> {
        let warped = sqrt_warp(magnitudes);
        let norm = spectrum_norm(&warped);
        if norm <= 0.0 {
            return None;
        }
        Some(Self {
            curve: kernel.salience_curve(&warped),
            magnitudes: magnitudes.to_vec(),
            min_midi,
            bins_per_semitone,
            norm,
        })
    }

    pub(crate) fn bins_per_semitone(&self) -> f32 {
        self.bins_per_semitone
    }

    /// Salience at a bin of *this* grid.
    ///
    /// Contract: `bin` comes from [`Self::tracked_bins`]. It is **not** enough for it to be a
    /// bin of the app's pitch domain — this grid is the user's bank, which may stop below
    /// that domain's ceiling, and the difference is what panicked the audio thread
    /// (`melody::tests::a_bank_narrower_than_the_pitch_domain_decodes_instead_of_panicking`).
    /// Left as a bare index on purpose: this is the decoder's inner loop, and the bounds
    /// question belongs at the domain's one gate, not once per bin.
    pub(crate) fn salience_at(&self, bin: usize) -> f32 {
        self.curve[bin]
    }

    /// A copy of the curve for a consumer that wants to *look* at the evidence rather than
    /// decode it — the pitch roll's salience layer.
    ///
    /// Two properties, both load-bearing, neither of them cosmetic:
    ///
    /// - **Zeroed outside [`Self::tracked_bins`].** The grid spans the waterfall's C0..C8
    ///   but candidates only ever come from the app's pitch domain, so salience below C1 is
    ///   scored-but-never-eligible. Painting it would show a hump the decoder provably never
    ///   considered, i.e. invent evidence. Zeroing keeps the **length** — the caller's
    ///   bin→pitch mapping is derived from it (`ui::pianoroll::draw_heat`), so a trimmed
    ///   curve would silently paint at the wrong pitch.
    /// - **Raw, not normalized.** The scale is display's business (`normalize_bars` and its
    ///   `gamma` live with the column this layer replaces); the scorer has no opinion about
    ///   contrast. Zero *then* normalize, or an ineligible hump outside the domain would set
    ///   the scale for the eligible ones inside it.
    ///
    /// Negative values survive on purpose: SWIPE′'s valleys are what punish 2·f0 (the whole
    /// reason this scorer is here), and `normalize_bars` clamps them to 0 downstream — a
    /// punished candidate should read as *absent*, not be quietly rectified into evidence.
    pub(crate) fn curve_over_tracked(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; self.curve.len()];
        if let Some(bins) = self.tracked_bins() {
            out[bins.clone()].copy_from_slice(&self.curve[bins]);
        }
        out
    }

    /// MIDI at a bin's position on this grid.
    ///
    /// There is deliberately **no** `bin_of_midi` beside it any more. There was, and it is
    /// how the crash came to be written: it answered "which bin is C8" for a grid that may
    /// well stop at C6, cheerfully, because the arithmetic cannot see how long the curve is.
    /// Bins now only ever come from [`Self::tracked_bins`], which can.
    pub(crate) fn midi_of_bin(&self, bin: usize) -> f32 {
        self.min_midi + bin as f32 / self.bins_per_semitone
    }

    /// The bins of the app's pitch domain (C1..C8) on this grid — the only candidates worth
    /// scoring — **clamped to what this column actually reaches**.
    ///
    /// Candidates come from the app's pitch domain, **not** from the column's own extent.
    /// The column reaches down to C0 = 16 Hz because that suits the spectrum waterfall, and
    /// scoring candidates there lets room rumble win: see `pitch::TRACKED_MIN_MIDI` for the
    /// mechanism and the measurement.
    ///
    /// But the domain is an *intent* and the column is a *fact*, and where they disagree the
    /// column wins: the bank's span is a user-facing slider (`ResonatorSettings::min_midi`/
    /// `max_midi`, a direct CPU dial for weak devices), so a bank that stops at C6 has no
    /// evidence about C7 to offer and pretending otherwise reads off the end of the curve.
    /// This clamp is the only place that reconciles the two, which is why every consumer must
    /// come through it rather than re-derive the domain from the constants — one that did
    /// crashed the audio thread on any ceiling but the default (`melody::decode`).
    pub(crate) fn tracked_bins(&self) -> Option<std::ops::RangeInclusive<usize>> {
        let first = (((TRACKED_MIN_MIDI - self.min_midi) * self.bins_per_semitone)
            .ceil()
            .max(0.0)) as usize;
        let last = (((TRACKED_MAX_MIDI - self.min_midi) * self.bins_per_semitone)
            .floor()
            .max(0.0) as usize)
            .min(self.curve.len().saturating_sub(1));
        (first <= last).then_some(first..=last)
    }

    /// The fine pitch and strength of an **already-decided** bin — by whatever decided it:
    /// this frame's argmax, or a Viterbi weighing this frame against the ones before it.
    ///
    /// Splitting the decision from the resolution is what lets continuity overrule the
    /// argmax without costing a single cent of precision.
    pub(crate) fn resolve(&self, bin: usize) -> (f32, f32) {
        // The salience decides the harmonic series, to within its own grid: parabola-refined
        // and clamped to ±½ bin, because a peak that is not a clean local extremum must not
        // fling the estimate across the grid. This is a *coarse* f0 — good enough to identify
        // the series, nowhere near good enough for cents.
        let coarse = parabolic_tau(&self.curve, bin).clamp(bin as f32 - 0.5, bin as f32 + 0.5);
        // The partials decide the frequency. See `refine_on_partials` for why this is a
        // separate step and not a refinement of the line above.
        let refined = refine_on_partials(&self.magnitudes, coarse, self.bins_per_semitone);
        let midi = self.min_midi + refined / self.bins_per_semitone;
        (midi, (self.curve[bin] / self.norm).clamp(0.0, 1.0))
    }

    /// This frame's own best guess, weighing nothing but this frame — the per-frame argmax
    /// over the pitch domain, as `(fractional_midi, pitch_strength)`.
    ///
    /// Kept as the bank's *raw* opinion (it rides to the UI as `TunerReading::fast_pitch`,
    /// and `dsp::melody` needs an unsmoothed witness to cross-examine) and as the honest
    /// baseline the Viterbi is measured against.
    ///
    /// This is what replaced `analysis_math::resonator_fundamental`, and the differences are
    /// consequences of the algorithm rather than choices:
    ///
    /// - **No floor.** Nothing requires the candidate's own bin to carry energy, because on
    ///   the instrument this app is for, it frequently does not.
    /// - **The refine happens on the salience curve, not on the spectrum.** The old code
    ///   parabola-refined the fundamental's own spectral peak, which is meaningless when
    ///   that peak is absent. SWIPE refines its own strength curve — thesis Chapter 5:
    ///   *"compute a coarse pitch strength curve and then fine tune it by using parabolic
    ///   interpolation"*.
    /// - **`pitch_strength` means something different, and better.** It used to be "how loud
    ///   is the fundamental's bin" — which reads ~0 for a perfectly clear open G. It is now
    ///   the normalized inner product: *how well does a harmonic series at this f0 explain
    ///   the whole spectrum*, in 0..1. That is what a confidence should mean.
    ///
    /// `None` when nothing in C1..C8 scores positive, i.e. the frame looks like no harmonic
    /// series at all. Silence is **not** decided here — the engine gates the melody on
    /// `level` upstream (`core::MELODY_LEVEL_GATE`), and a second opinion here would just be
    /// a magic number in a new place.
    pub(crate) fn argmax(&self) -> Option<(f32, f32)> {
        let (peak_bin, &peak) = self.curve[self.tracked_bins()?]
            .iter()
            .enumerate()
            .map(|(offset, value)| (offset, value))
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap();
        if peak <= 0.0 {
            return None;
        }
        let first = *self.tracked_bins().unwrap().start();
        Some(self.resolve(first + peak_bin))
    }
}

/// SWIPE's spectral warping: the **square root** of the magnitude.
///
/// Not the magnitude and emphatically not the power. The thesis picks √ on a worked
/// example that is our exact signal — a spectrum with *"a missing fundamental and a
/// salient second harmonic"*, f0 at 190 Hz and h2 at 380 (the violin G is 196/392) —
/// because √ *"neither overemphasizes the missing fundamental (as the logarithm does) nor
/// the salient second harmonic (as the square does)"*. It also makes each harmonic's
/// contribution to the inner product proportional to its amplitude rather than to its
/// square.
pub(crate) fn sqrt_warp(magnitudes: &[f32]) -> Vec<f32> {
    magnitudes.iter().map(|m| m.max(0.0).sqrt()).collect()
}

/// `‖√|X|‖₂` — the per-frame constant that turns [`SwipeKernel::salience`] into a true
/// normalized inner product. Constant across candidates, so it is *not* needed to pick a
/// pitch; it is here for when a 0..1 confidence is.
pub(crate) fn spectrum_norm(warped: &[f32]) -> f32 {
    warped.iter().map(|v| v * v).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::analysis_math::{
        NOTE_BUCKET_MIN_MIDI,
        SPIRAL_BINS_PER_SEMITONE,
        resonator_fundamental,
    };
    use crate::audio::dsp::resonator::ResonatorAnalyzer;
    use crate::core_types::note::AccidentalStyle;

    /// The kernel must reproduce eq. 3-12 where the harmonic set is unbroken, and SWIPE′'s
    /// stated valley weights where it is not. Checked on the raw cosine·envelope product,
    /// i.e. before the `1/√r` decay, by dividing it back out.
    #[test]
    fn kernel_matches_equation_3_12() {
        let bps = 64.0; // fine grid so a ratio lands near a sample
        let kernel = SwipeKernel::new(bps);
        let bins_per_octave = 12.0 * bps;
        let at = |r: f32| -> f32 {
            let offset = (bins_per_octave * r.log2()).round() as isize;
            let w = kernel.weights[(offset - kernel.first_offset) as usize];
            w * r.sqrt() // undo the envelope → back to K(r)
        };

        // Peaks of kept harmonics: cos(2πh) = 1, full weight.
        for h in [1.0, 2.0, 3.0, 5.0, 7.0] {
            assert!((at(h) - 1.0).abs() < 0.02, "peak at h={h} was {}", at(h));
        }
        // Valleys with BOTH neighbours kept keep the full −1. The thesis names exactly
        // these two: "the valley between the first and second harmonics, and the valley
        // between the second and third harmonics".
        assert!((at(1.5) + 1.0).abs() < 0.02, "valley 1.5 was {}", at(1.5));
        assert!((at(2.5) + 1.0).abs() < 0.02, "valley 2.5 was {}", at(2.5));
        // Valley with only one kept neighbour (h4 is dropped) → −1/2.
        assert!((at(3.5) + 0.5).abs() < 0.02, "valley 3.5 was {}", at(3.5));
        // The first valley is halved by eq. 3-12 — and it is what punishes energy at
        // half the candidate, i.e. the true f0 when the candidate is the phantom octave.
        assert!((at(0.5) + 0.5).abs() < 0.02, "valley 0.5 was {}", at(0.5));
        // A dropped harmonic's peak contributes nothing at all.
        assert!(at(4.0).abs() < 0.02, "dropped peak h=4 was {}", at(4.0));
    }

    /// Build a synthetic log-frequency column: one bin per partial, no skirts.
    fn column_with_partials(f0_hz: f32, amplitudes: &[f32], min_midi: f32, bps: f32) -> Vec<f32> {
        let len = (96.0 * bps) as usize + 1; // C0..C8, the bank's own span
        let mut column = vec![0.0f32; len];
        for (index, &amplitude) in amplitudes.iter().enumerate() {
            let hz = f0_hz * (index + 1) as f32;
            let midi = 69.0 + 12.0 * (hz / 440.0).log2();
            let bin = ((midi - min_midi) * bps).round() as usize;
            if bin < len {
                column[bin] += amplitude;
            }
        }
        column
    }

    /// The display layer's contract with the renderer, which is **geometric** and fails
    /// silently when broken.
    ///
    /// `ui::pianoroll::draw_heat` derives bins-per-semitone from the column's *length* —
    /// deliberately, because that length changes with the reassignment toggle. So a curve
    /// trimmed to the tracked domain would not draw a smaller picture, it would draw the
    /// whole picture **at the wrong pitch**, with nothing anywhere to say so. Hence the
    /// zeroing: same grid, same length, and what was never a candidate simply reads empty.
    ///
    /// The trimmed-vs-zeroed choice is invisible in every other test, which is why it gets
    /// one of its own.
    #[test]
    fn the_display_curve_keeps_the_grid_and_zeroes_what_was_never_a_candidate() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        // The waterfall's C0 — pointedly *not* the tracker's floor. That gap is the whole
        // subject of this test; if the two ever coincide it becomes vacuous, so it says so.
        let min_midi = 12.0;
        let kernel = SwipeKernel::new(bps);
        let column = column_with_partials(196.0, &[0.08, 1.0, 0.7, 0.5], min_midi, bps);
        let frame = SalienceFrame::score(&column, min_midi, bps, &kernel).unwrap();
        let display = frame.curve_over_tracked();

        assert_eq!(
            display.len(),
            column.len(),
            "the renderer reads bin→pitch off this length; a short curve paints at the wrong note"
        );

        let first_tracked = ((TRACKED_MIN_MIDI - min_midi) * bps).ceil() as usize;
        assert!(
            first_tracked > 0,
            "vacuous: the grid no longer reaches below the tracked domain"
        );
        assert!(
            display[..first_tracked].iter().all(|&v| v == 0.0),
            "salience painted below C1, where the decoder scores no candidate at all"
        );
        // Inside the domain it is the evidence itself — no smoothing, and above all no
        // rectifying: a negative salience is a *punished* candidate (the valleys are why
        // this scorer exists), and it must reach the display as such.
        for bin in first_tracked..display.len() {
            assert_eq!(
                display[bin],
                frame.salience_at(bin),
                "bin {bin} was altered on the way to the display"
            );
        }
    }

    /// The layer has to be *legible*, and "broad" is not the same claim as "a solid wash".
    ///
    /// `ui::pianoroll::draw_heat` paints every bin over `HEAT_GATE` (0.16 of the column's
    /// own max). The spectrum clears that in a handful of bins because partials are spikes;
    /// SWIPE′'s curve is a 71-bin hump, so the honest worry is that normalizing it to its
    /// own max floods the plot and the "layer" is a rectangle. This pins the actual number
    /// so a future change to the kernel, the grid or the gate cannot quietly cross that line.
    #[test]
    fn the_display_curve_is_a_hump_and_not_a_wash() {
        const HEAT_GATE: f32 = 0.16; // ui::pianoroll's, mirrored — it is private there

        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0;
        let kernel = SwipeKernel::new(bps);
        let column = column_with_partials(196.0, &[0.08, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22], min_midi, bps);
        let frame = SalienceFrame::score(&column, min_midi, bps, &kernel).unwrap();
        let mut display = frame.curve_over_tracked();
        crate::audio::dsp::analysis_math::normalize_bars(&mut display, 0.72); // the default gamma

        let painted = display.iter().filter(|&&v| v >= HEAT_GATE).count();
        let share = painted as f32 / display.len() as f32;
        let semitones = painted as f32 / bps;
        println!("\n=== violin open G, the salience layer as painted ===");
        println!(
            "  bins over the gate : {painted} / {} ({:.1}%)",
            display.len(),
            share * 100.0
        );
        println!(
            "  = {semitones:.1} semitones of the {:.0}-semitone grid",
            display.len() as f32 / bps
        );

        assert!(
            share < 0.5,
            "the salience layer painted {:.0}% of the grid — that is a wash, not a picture",
            share * 100.0
        );
    }

    /// PROBE: does the **bank's ceiling** decide the octave?
    ///
    /// The kickstart records an unexplained result — on a synthetic pure A5, `s(A4)`
    /// overtakes `s(A5)`: SWIPE′ prefers the sub-octave on its own. The hypothesis "the
    /// kernel is truncated at the grid's ceiling" was written down and dismissed as weak,
    /// on the grounds that *"the bank runs to MIDI 108"*.
    ///
    /// That ground does not hold for a user: `max_midi` is a slider, and the settings that
    /// crashed the audio thread had it at **84**. The kernel needs `12·log₂(13.75)` ≈ **45
    /// semitones of headroom** above a candidate to be scored whole. At a ceiling of 84 the
    /// candidate A5 has 3 semitones of it — only its `h=1` lobe survives — while the
    /// sub-octave A4 keeps `h=1` *and* `h=2`, and `h=2` is exactly where A5's energy sits.
    /// So the sub-octave can explain the note and the note cannot explain itself.
    ///
    /// Prints rather than asserts a threshold: it exists to say whether the ceiling is a
    /// mechanism or a red herring, and that is a number to read, not a bound to defend.
    #[test]
    fn does_the_bank_ceiling_feed_the_sub_octave() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0f32;
        let kernel = SwipeKernel::new(bps);

        println!("\n=== pure A5 (880 Hz + partials), scored against its own sub-octave ===");
        println!("  kernel needs ~45 semitones of headroom above a candidate");
        for max_midi in [84.0f32, 96.0, 108.0] {
            let len = ((max_midi - min_midi) * bps) as usize + 1;
            let mut column = vec![0.0f32; len];
            for (index, amplitude) in [1.0f32, 0.8, 0.6, 0.35, 0.2].into_iter().enumerate() {
                let hz = 880.0 * (index + 1) as f32;
                let midi = 69.0 + 12.0 * (hz / 440.0).log2();
                let bin = ((midi - min_midi) * bps).round() as usize;
                if bin < len {
                    column[bin] += amplitude;
                }
            }
            let warped = sqrt_warp(&column);
            let bin_of = |midi: f32| ((midi - min_midi) * bps).round() as usize;
            let a5 = kernel.salience(&warped, bin_of(81.0));
            let a4 = kernel.salience(&warped, bin_of(69.0));
            let headroom = max_midi - 81.0;
            println!(
                "  ceiling MIDI {max_midi:>5.0} ({headroom:>2.0} st above A5) : A5 {a5:.4}  A4 {a4:.4}  -> {}",
                if a5 > a4 {
                    "A5 (correct)"
                } else {
                    "A4 — THE SUB-OCTAVE WINS"
                }
            );
        }
    }

    /// **The golden test.** A violin open G: the body barely radiates 196 Hz, so the
    /// fundamental is a *tenth* of the 2nd harmonic. The salience must still say G3.
    ///
    /// The old `resonator_fundamental` cannot pass this and does not even get as far as
    /// scoring: at 0.08 the fundamental is under `FUNDAMENTAL_FLOOR`, so G3 is skipped as
    /// a candidate outright and only G4 survives to be scored. That guard is the defect,
    /// made executable.
    #[test]
    fn violin_g_with_a_weak_fundamental_decides_g3() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0;
        let kernel = SwipeKernel::new(bps);
        // Open G: f0 = 196.0 Hz. Harmonic amplitudes with the fundamental crushed.
        let column = column_with_partials(196.0, &[0.08, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22], min_midi, bps);
        let warped = sqrt_warp(&column);
        let curve = kernel.salience_curve(&warped);

        let best = curve
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(bin, _)| bin)
            .unwrap();
        let best_midi = min_midi + best as f32 / bps;
        assert!(
            (best_midi - 55.0).abs() < 0.5,
            "expected G3 (55), got {best_midi:.2}"
        );
    }

    /// The claim "SWIPE does not need the fundamental" is the whole reason this is the
    /// right algorithm here, so it gets asserted at the limit rather than argued: sweep
    /// the fundamental's amplitude from a tenth all the way to **exactly zero** and the
    /// decision must never move off G3. This is "by construction" made falsifiable.
    #[test]
    fn the_decision_holds_as_the_fundamental_vanishes() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0;
        let kernel = SwipeKernel::new(bps);
        for f0_amplitude in [0.10f32, 0.08, 0.05, 0.02, 0.01, 0.0] {
            let column = column_with_partials(
                196.0,
                &[f0_amplitude, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22],
                min_midi,
                bps,
            );
            let warped = sqrt_warp(&column);
            let curve = kernel.salience_curve(&warped);
            let best = curve
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .map(|(bin, _)| bin)
                .unwrap();
            let best_midi = min_midi + best as f32 / bps;
            assert!(
                (best_midi - 55.0).abs() < 0.5,
                "fundamental {f0_amplitude}: expected G3 (55), got {best_midi:.2}"
            );
        }
    }

    /// The mechanism, isolated: G4 must lose *because its valleys are fed*, not because
    /// of some other accident. Prints the arithmetic so the claim in the design doc is
    /// checkable rather than quotable.
    #[test]
    fn phantom_octave_probe() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0;
        let kernel = SwipeKernel::new(bps);
        let column = column_with_partials(196.0, &[0.08, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22], min_midi, bps);
        let warped = sqrt_warp(&column);

        let bin_of = |midi: f32| ((midi - min_midi) * bps).round() as usize;
        let g3 = kernel.salience(&warped, bin_of(55.0)); // 196 Hz
        let g4 = kernel.salience(&warped, bin_of(67.0)); // 392 Hz — the phantom
        println!("\n=== violin open G, fundamental at 0.08 of the 2nd harmonic ===");
        println!("  salience G3 (196 Hz, the truth)   = {g3:.4}");
        println!("  salience G4 (392 Hz, the phantom) = {g4:.4}");
        println!("  margin = {:.4}", g3 - g4);
        assert!(g3 > g4, "the phantom octave won: G3 {g3:.4} vs G4 {g4:.4}");
    }

    /// **The non-circular half.** A new algorithm passing its own tests proves nothing
    /// until the thing it replaces is shown to *fail* them, on the same input. So: run
    /// the shipped `resonator_fundamental` over the identical violin-G column.
    ///
    /// This test asserts the **bug**, and it is expected to start failing the day the
    /// old comb is deleted — at which point delete this too. Until then it is the only
    /// thing standing between "SWIPE′ is better" and "SWIPE′ agrees with itself".
    #[test]
    fn the_old_comb_fails_the_same_column() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = 12.0;
        let column = column_with_partials(196.0, &[0.08, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22], min_midi, bps);
        // 0.18 is the shipped FUNDAMENTAL_FLOOR, and the column is already 0..1 — i.e.
        // exactly the state `resonator_snapshot` hands it over in.
        let verdict = resonator_fundamental(&column, min_midi, bps, 0.18);
        println!("\n=== the OLD reward-only comb, same violin-G column ===");
        match verdict {
            Some((midi, strength)) => {
                println!("  picked MIDI {midi:.2}  (truth G3 = 55, phantom G4 = 67), strength {strength:.3}");
                assert!(
                    (midi - 55.0).abs() > 0.5,
                    "the old comb got G3 right — then Phase 1.11's premise is wrong and \
                     the diagnosis needs redoing, not the algorithm"
                );
            }
            None => println!("  picked NOTHING — the floor rejected every candidate"),
        }
    }

    /// **The real thing.** Everything above runs on synthetic columns — one bin per
    /// partial, no skirts. The real reassigned column is *spiky*, because reassignment
    /// splats energy at its measured instantaneous frequency and the coherence gate drops
    /// what does not cohere, whereas SWIPE expects a dense √-spectrum. Whether the
    /// kernel's valleys still collect the odd harmonics on such a column is design §4.3 —
    /// the one risk that can sink the whole approach, and it cannot be reasoned out.
    ///
    /// So: real violin, real bow, real bank. Recorded on the open G (196 Hz) — the string
    /// whose fundamental the instrument barely radiates — through the same PulseAudio
    /// source the app captures from, unprocessed and unclipped.
    ///
    /// Both scorers run on the **same** column from the **same** bank frame, so the only
    /// difference is the scoring. Tallied over the frames the old scorer itself considers
    /// voiced, which is apples-to-apples: those are exactly the frames that reach the
    /// melody line today.
    #[test]
    #[ignore = "needs testdata/*.wav — run with --ignored --nocapture"]
    fn real_violin_g_probe() {
        const G3_MIDI: f32 = 55.0;
        const G4_MIDI: f32 = 67.0;

        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = NOTE_BUCKET_MIN_MIDI as f32;

        for name in [
            "g_open_slow_strokes",
            "g_open_fast_strokes",
            "g_open_real_octave",
            "g_string_trill",
            "a_string_trill",
        ] {
            let path = format!("{}/testdata/{name}.wav", env!("CARGO_MANIFEST_DIR"));
            let mut reader = hound::WavReader::open(&path).unwrap();
            let sample_rate = reader.spec().sample_rate as f32;
            let samples: Vec<f32> = reader
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect();

            let mut analyzer = ResonatorAnalyzer::new(sample_rate);
            // Sample the decision at the bank's own ~16 ms publish cadence, which is the
            // rate `MelodyTracker` is contractually driven at.
            let hop = (sample_rate * 0.016) as usize;

            let (mut old_g3, mut old_g4, mut old_other, mut old_silent) = (0u32, 0u32, 0u32, 0u32);
            let (mut new_g3, mut new_g4, mut new_other) = (0u32, 0u32, 0u32);
            let mut new_none = 0u32;
            let mut new_verdicts: Vec<f32> = Vec::new();

            let mut fed = 0usize;
            while fed + hop <= samples.len() {
                analyzer.process_samples(&samples[fed..fed + hop], true);
                fed += hop;
                let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);

                // The OLD comb, fed exactly what it was fed in production: the column
                // *after* `normalize_bars(gamma)`, with the shipped 0.18 floor. That is
                // why `resonator_fundamental` is still alive — it is the oracle, and it
                // goes when this probe does.
                let Some((old_midi, _strength)) =
                    resonator_fundamental(&snapshot.spectrum, min_midi, bps, 0.18)
                else {
                    old_silent += 1;
                    continue;
                };
                let near = |midi: f32, target: f32| (midi - target).abs() < 0.5;
                if near(old_midi, G3_MIDI) {
                    old_g3 += 1;
                } else if near(old_midi, G4_MIDI) {
                    old_g4 += 1;
                } else {
                    old_other += 1;
                }

                // The NEW scorer, straight off the production snapshot — this probe
                // measures the wired engine, not a re-implementation of it.
                //
                // `None` is now reachable and meaningful: since the candidate search was
                // confined to the app's pitch domain (`pitch::TRACKED_MIN_MIDI`), a frame
                // where nothing in C1..C8 scores positive is a frame that looks like no
                // harmonic series at all. The old comb, searching the whole display
                // column down to C0, always found *something* to crown.
                let Some((new_midi, _)) = snapshot.fundamental else {
                    new_none += 1;
                    continue;
                };
                new_verdicts.push(new_midi);
                if near(new_midi, G3_MIDI) {
                    new_g3 += 1;
                } else if near(new_midi, G4_MIDI) {
                    new_g4 += 1;
                } else {
                    new_other += 1;
                }
            }

            // What IS the "other" bucket? An unexplained quarter of the frames is where
            // the next bug hides, so name the notes instead of lumping them.
            let mut histogram: std::collections::BTreeMap<i32, u32> = Default::default();
            for &midi in &new_verdicts {
                *histogram.entry(midi.round() as i32).or_default() += 1;
            }
            let mut ranked: Vec<(i32, u32)> = histogram.into_iter().collect();
            ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

            let voiced = old_g3 + old_g4 + old_other;
            let pct = |n: u32| 100.0 * n as f32 / voiced.max(1) as f32;
            println!("\n=== {name} — {voiced} voiced frames ({old_silent} silent) ===");
            println!(
                "  OLD comb : G3 {old_g3:>4} ({:>5.1}%)   G4(phantom) {old_g4:>4} ({:>5.1}%)   other {old_other:>4} ({:>5.1}%)",
                pct(old_g3),
                pct(old_g4),
                pct(old_other)
            );
            println!(
                "  SWIPE'   : G3 {new_g3:>4} ({:>5.1}%)   G4          {new_g4:>4} ({:>5.1}%)   other {new_other:>4} ({:>5.1}%)   no-pitch {new_none:>4} ({:>5.1}%)",
                pct(new_g3),
                pct(new_g4),
                pct(new_other),
                pct(new_none)
            );
            // First position on a violin string spans the open note up to the 4th finger,
            // i.e. the open string + 7 semitones. `g_string_trill` and `a_string_trill`
            // are the player putting fingers down in first position, so *every* note from
            // the open string to +7 is real content — the earlier reading of these takes
            // as "spread" was the probe's G3/G4 buckets calling real notes "other", not
            // the detector erring.
            let playable: Option<std::ops::RangeInclusive<i32>> = match name {
                "g_string_trill" => Some(55..=62), // G3 open .. D4
                "a_string_trill" => Some(69..=76), // A4 open .. E5
                _ => None,                         // open-G takes: G3 only, see above
            };
            if let Some(band) = playable {
                let inside: u32 = ranked
                    .iter()
                    .filter(|(midi, _)| band.contains(midi))
                    .map(|(_, count)| *count)
                    .sum();
                let total: u32 = ranked.iter().map(|(_, count)| *count).sum();
                println!(
                    "  in first position: {inside}/{total} ({:.1}%) — everything else is a real error",
                    100.0 * inside as f32 / total.max(1) as f32
                );
                // Name the errors. Whether they are octaves of a played note or junk in a
                // register the violin cannot reach decides whether temporal smoothing is
                // even the right tool: a Viterbi can outvote a lone excursion, but it
                // cannot invent a candidate the salience never proposed.
                let mut errors: Vec<(i32, u32)> = ranked
                    .iter()
                    .filter(|(midi, _)| !band.contains(midi))
                    .copied()
                    .collect();
                errors.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                let octave_errors: u32 = errors
                    .iter()
                    .filter(|(midi, _)| band.contains(&(midi - 12)) || band.contains(&(midi + 12)))
                    .map(|(_, count)| *count)
                    .sum();
                println!(
                    "    of which an OCTAVE of a played note: {octave_errors} ({:.1}% of all frames)",
                    100.0 * octave_errors as f32 / total.max(1) as f32
                );
            }
            let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
            let top: Vec<String> = ranked
                .iter()
                .take(6)
                .map(|(midi, count)| {
                    let octave = midi / 12 - 1;
                    let name = names[(midi % 12) as usize];
                    format!("{name}{octave} {:.0}%", pct(*count))
                })
                .collect();
            println!("  SWIPE' says   : {}", top.join(" · "));
        }
    }
}

#[cfg(test)]
mod low_register_probe {
    //! Why does a bowed G3 ever score C0..C2? Live report: the pitch roll plunges
    //! three or four octaves below the played note, and the plunges drag the panel's
    //! auto-framing down with them, so the real note is squeezed off the top.
    use super::*;
    use crate::audio::dsp::analysis_math::{
        NOTE_BUCKET_MIN_MIDI,
        SPIRAL_BINS_PER_SEMITONE,
    };
    use crate::audio::dsp::resonator::ResonatorAnalyzer;
    use crate::core_types::note::AccidentalStyle;

    /// Find a frame where the low junk wins, then dissect the arithmetic behind it.
    #[test]
    #[ignore = "needs testdata/*.wav — run with --ignored --nocapture"]
    fn what_makes_the_sub_bass_win() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = NOTE_BUCKET_MIN_MIDI as f32;
        let kernel = SwipeKernel::new(bps);

        let path = format!("{}/testdata/g_open_slow_strokes.wav", env!("CARGO_MANIFEST_DIR"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let sample_rate = reader.spec().sample_rate as f32;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut analyzer = ResonatorAnalyzer::new(sample_rate);
        let hop = (sample_rate * 0.016) as usize;
        let mut fed = 0usize;
        let mut dissected = 0;

        while fed + hop <= samples.len() && dissected < 4 {
            analyzer.process_samples(&samples[fed..fed + hop], true);
            fed += hop;
            let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);
            let Some((midi, _)) = snapshot.fundamental else {
                continue;
            };
            // Skip the first second: the bank starts empty and needs to charge, so the
            // opening frames are junk for a reason that has nothing to do with scoring.
            // (The first cut of this probe caught only those and "disproved" itself.)
            if fed as f32 / sample_rate < 1.0 {
                continue;
            }
            // Only interested in the plunges: a verdict far below the played G3.
            if midi > 45.0 {
                continue;
            }
            dissected += 1;

            // Undo the display gamma to get back to the bank's own column.
            let magnitudes: Vec<f32> = snapshot.spectrum.iter().map(|v| v.powf(1.0 / 0.72)).collect();
            let warped = sqrt_warp(&magnitudes);
            let curve = kernel.salience_curve(&warped);
            let bin_of = |m: f32| ((m - min_midi) * bps).round() as usize;
            let winner = bin_of(midi);
            let g3 = bin_of(55.0);

            println!(
                "\n=== frame at {:.2}s — verdict MIDI {midi:.2}, truth G3 (55) ===",
                fed as f32 / sample_rate
            );
            println!("  salience  winner {:.4}   G3 {:.4}", curve[winner], curve[g3]);
            // The suspicion: `salience` divides by the norm of the kernel's positive part
            // *that is still in range*. A candidate near the bottom of the grid has much
            // of its kernel off the low end — including the fat h=1 lobe — so that norm
            // shrinks, and dividing a small numerator by a small norm can inflate a
            // candidate the data never supported.
            let in_range = |bin: usize| -> (f32, f32) {
                let low = kernel.first_offset.max(-(bin as isize));
                let high = (kernel.first_offset + kernel.weights.len() as isize - 1)
                    .min(warped.len() as isize - 1 - bin as isize);
                let lo_i = (low - kernel.first_offset) as usize;
                let hi_i = (high - kernel.first_offset) as usize;
                let norm_sq = kernel.positive_sq_prefix[hi_i + 1] - kernel.positive_sq_prefix[lo_i];
                let full = *kernel.positive_sq_prefix.last().unwrap();
                (norm_sq, full)
            };
            let (win_norm, full) = in_range(winner);
            let (g3_norm, _) = in_range(g3);
            println!(
                "  kernel positive mass in range:  winner {:.1}%   G3 {:.1}%",
                100.0 * win_norm / full,
                100.0 * g3_norm / full
            );
            println!(
                "  numerator (pre-normalization):  winner {:.4}   G3 {:.4}",
                curve[winner] * win_norm.sqrt(),
                curve[g3] * g3_norm.sqrt()
            );
        }
    }
}
