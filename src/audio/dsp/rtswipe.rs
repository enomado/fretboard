//! RT-SWIPE — SWIPE′ scored from windowed FFTs instead of from the resonator bank.
//!
//! Source: [RT-SWIPE, CMMR 2025](https://www.audiolabs-erlangen.de/resources/MIR/2025-CMMR-RTSWIPE)
//! (Meier, Strahl, Schwär, Müller, Balke), which is Camacho's SWIPE′ made causal; the
//! algorithm underneath is Camacho's thesis and its reference implementation `swipep.m`.
//! The survey's reading of both is `docs/pitch_detection_survey.md`.
//!
//! # This is a frontend, not a second detector
//!
//! [`SwipeKernel`] already scores a column of magnitudes on a log-frequency grid, and it
//! does not care where the column came from. So this module is **the same kernel with a
//! different frontend**: eight windowed FFTs in place of the bank's IIR resonators, each
//! resampled onto the bank's own grid (8 bins/semitone, MIDI 12..108), each scored by the
//! same [`SwipeKernel::salience`], and the results blended into one curve that becomes an
//! ordinary [`SalienceFrame`]. Everything downstream — `resolve`, `argmax`, the Viterbi in
//! `dsp::melody`, `pitch_bench` — cannot tell the two apart, which is the point: what the
//! benchmark then compares is the *frontend*, with the scorer held fixed.
//!
//! # Why eight windows
//!
//! A window is a compromise between resolving a pitch and reacting to it, and the right
//! compromise depends on the pitch: Camacho fixes the window at `PERIODS_PER_WINDOW`
//! periods **of the candidate**, which makes it a different window for every candidate.
//! Powers of two make that affordable — one FFT per octave, and each candidate reads the
//! two rungs that straddle its ideal length, interpolated (`λ`, [`RtSwipe::blend`]).
//!
//! # Causal by right-alignment, and what that costs
//!
//! Every window ends at the newest sample rather than being centred on it (RT-SWIPE's one
//! structural change to SWIPE). The estimate is therefore late by half a window — `N_k/2`,
//! i.e. **4 periods of the pitch**: ~10 ms at 400 Hz, ~20 ms at 200 Hz, ~40 ms at 100 Hz.
//! Pitch-dependent delay, unlike the bank's 8–29 ms. The user has explicitly allowed it
//! ("если что я не особо настаиваю на безлаговости"), which is what makes this worth
//! building: the comparison is against the *slow* plane (pYIN, 128 ms), not the fast one.
//!
//! # Two deliberate departures from `swipep.m`, both forced by the reuse
//!
//! - **The grid is log-frequency, not ERB.** Camacho interpolates onto an ERB scale; our
//!   kernel is built for the bank's log grid and validated on it (`docs/swipe_salience_design.md`).
//!   Sharing the kernel means sharing its grid — that is the whole trade this module makes.
//! - **The grid stops at the bank's edges** (MIDI 12..108 = 16 Hz..4186 Hz), where Camacho's
//!   runs to `fs/2`. Since [`spectrum_norm`] sums the whole column, each rung's norm here
//!   sees a narrower band than his. It is the same band for every rung and for the bank, so
//!   it cannot tilt the comparison the benchmark exists to make.

use std::sync::Arc;

use realfft::{
    RealFftPlanner,
    RealToComplex,
};
use resonators::midi_to_hz;
use rustfft::num_complex::Complex32;

use super::analysis_math::{
    NOTE_BUCKET_MIN_MIDI,
    SPIRAL_BIN_COUNT,
    SPIRAL_BINS_PER_SEMITONE,
    hann_taper,
};
use super::pitch::{
    TRACKED_MAX_MIDI,
    TRACKED_MIN_MIDI,
};
use super::swipe::{
    SalienceFrame,
    SwipeKernel,
    spectrum_norm,
    tracked_bins,
};

/// Periods of a candidate's own pitch spanned by the window used to score it.
///
/// Camacho's constant (`swipep.m`: `ws = 8·fs/p`), not a knob: it is what sets both the
/// ladder and the delay, and it is the number the published RPA figures were measured with.
/// Halving it would halve the delay and cost the low register its resolution — a trade to
/// *measure* against the corpus if it is ever wanted, not to guess at here.
const PERIODS_PER_WINDOW: f32 = 8.0;

/// The grid this scores on: the bank's own, because the kernel is the bank's own.
///
/// Fixed, where the bank's span is a user slider (`ResonatorSettings::min_midi`/`max_midi`,
/// a CPU dial for weak devices). Nothing to dial here — an FFT's cost is set by its length,
/// and the ladder's lengths come from the pitch domain, not from the user.
const MIN_MIDI: f32 = NOTE_BUCKET_MIN_MIDI as f32;
const BINS_PER_SEMITONE: f32 = SPIRAL_BINS_PER_SEMITONE as f32;

/// Which rung of the window ladder.
///
/// A newtype because this module carries two index domains at once — rungs (0..8) and grid
/// bins (0..769) — and both are small integers that appear in the same loops, indexing
/// different vectors. Exactly the mix-up the house rule is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowId(usize);

/// One rung: an FFT of a fixed power-of-two length, and the pitch it is ideal for.
struct AnalysisWindow {
    /// `N_k`, a power of two.
    size:       usize,
    /// `PERIODS_PER_WINDOW·fs/N_k` — the pitch whose ideal window is exactly this long.
    /// The rungs are octave-spaced in this, which is what makes λ a two-tap blend.
    optimal_hz: f32,
    /// Cached because the ladder is fixed: 32 KB of tables against ~32k cosines per frame.
    taper:      Vec<f32>,
    /// Real-input FFT. On a real signal it does about half the butterflies of the full complex
    /// FFT this used to plan, and its output bins 0..=N/2 equal that FFT's bins 0..=N/2 to
    /// rounding — so the magnitude column the kernel reads is unchanged, and nothing downstream
    /// (RPA, octave errors) moves. It is a throughput fix, not an accuracy one.
    fft:        Arc<dyn RealToComplex<f32>>,
}

pub(crate) struct RtSwipe {
    sample_rate:  f32,
    /// A4, as everywhere else the grid's bins are turned into frequencies. Taken rather
    /// than assumed: the bank's grid follows `concert_pitch_hz`, and a frontend that
    /// silently pinned 440 would put its columns on a *different* grid from the bank's the
    /// moment a user retunes — the one difference the benchmark must not have.
    reference_hz: f32,
    /// The newest `N_max` samples, oldest first — see [`Self::process_samples`].
    history:      Vec<f32>,
    windows:      Vec<AnalysisWindow>,
    kernel:       SwipeKernel,
    /// `lambda[bin]` = the rungs that explain `bin`, and their weights, summing to 1.
    /// Precomputed: the grid and the ladder are both fixed at construction.
    lambda:       Vec<Vec<(WindowId, f32)>>,

    // Reused scratch, so [`Self::frame`] allocates nothing on the audio thread beyond the
    // one frame it returns (its curve, and the winner's column). This is a real-time-safety
    // fix, not just a speed one: `malloc` can block on a global lock, so per-frame allocation
    // on the audio path is a *jitter* hazard regardless of its average cost. Per window: a real
    // input buffer, the FFT's complex output, and the FFT's own scratch (a real FFT is not an
    // in-place transform); plus the per-rung grid columns the blend reads.
    scratch_input:    Vec<Vec<f32>>, // one per window, `window.size` (windowed real input)
    scratch_fft_out:  Vec<Vec<Complex32>>, // one per window, `window.size / 2 + 1` (DC..Nyquist)
    scratch_fft_work: Vec<Vec<Complex32>>, // one per window, the FFT's own scratch (`get_scratch_len`)
    scratch_spectrum: Vec<Vec<f32>>, // one per window, `window.size / 2` long (`|X|`)
    scratch_columns:  Vec<Vec<f32>>, // one per window, `SPIRAL_BIN_COUNT` long (raw)
    scratch_warped:   Vec<Vec<f32>>, // one per window, `SPIRAL_BIN_COUNT` long (√-warped)
    scratch_norms:    Vec<f32>,      // one per window
}

impl RtSwipe {
    pub(crate) fn new(sample_rate: f32, reference_hz: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let windows: Vec<AnalysisWindow> = window_ladder(sample_rate, reference_hz)
            .into_iter()
            .map(|size| {
                AnalysisWindow {
                    size,
                    optimal_hz: PERIODS_PER_WINDOW * sample_rate / size as f32,
                    taper: hann_taper(size),
                    fft: planner.plan_fft_forward(size),
                }
            })
            .collect();

        let longest = windows.iter().map(|window| window.size).max().unwrap();
        let rungs = windows.len();
        // The real FFT's own vector shapes: `make_output_vec` is N/2+1, `make_scratch_vec` is
        // `get_scratch_len`. Asking the plan rather than hardcoding keeps this correct if the
        // backend's scratch needs ever change.
        let scratch_input = windows.iter().map(|window| vec![0.0f32; window.size]).collect();
        let scratch_fft_out = windows
            .iter()
            .map(|window| window.fft.make_output_vec())
            .collect();
        let scratch_fft_work = windows
            .iter()
            .map(|window| window.fft.make_scratch_vec())
            .collect();
        let scratch_spectrum = windows
            .iter()
            .map(|window| vec![0.0f32; window.size / 2])
            .collect();
        Self {
            sample_rate,
            reference_hz,
            // Starts as silence, exactly as the bank starts discharged: the first frames of
            // a take are junk for a reason that has nothing to do with scoring.
            history: vec![0.0; longest],
            lambda: blend_weights(&windows, reference_hz),
            kernel: SwipeKernel::new(BINS_PER_SEMITONE),
            windows,
            scratch_input,
            scratch_fft_out,
            scratch_fft_work,
            scratch_spectrum,
            scratch_columns: vec![vec![0.0; SPIRAL_BIN_COUNT]; rungs],
            scratch_warped: vec![vec![0.0; SPIRAL_BIN_COUNT]; rungs],
            scratch_norms: vec![0.0; rungs],
        }
    }

    /// Feed the newest samples. The caller's cadence is its own business — the frame is
    /// built from the history on demand — but `pitch_bench` and `dsp::melody` drive this at
    /// the bank's 16 ms hop so that frames from either frontend land on the same grid.
    pub(crate) fn process_samples(&mut self, samples: &[f32]) {
        let capacity = self.history.len();
        // Anything older than N_max has slid out of every window, so a hop longer than the
        // history is not an error — only its tail was ever going to matter.
        let fresh = samples.len().min(capacity);
        self.history.copy_within(fresh.., 0);
        self.history[capacity - fresh..].copy_from_slice(&samples[samples.len() - fresh..]);
    }

    /// Score the history as it stands: one frame, right-aligned at the newest sample.
    ///
    /// `None` only for digital silence across the whole ladder — the same "no column at all"
    /// that [`SalienceFrame::score`] returns `None` for. A frame whose curve is merely
    /// *negative* everywhere is still a frame: whether that means "no pitch" is
    /// [`SalienceFrame::argmax`]'s call, in one place, as it is for the bank.
    pub(crate) fn frame(&mut self) -> Option<SalienceFrame> {
        // Each rung on its own terms: raw column → √-warp → *its own* spectral norm, all into
        // reused scratch (see the fields). Disjoint field borrows, so no `self` method call in
        // the loop — that would borrow all of `self` and clash with the scratch it writes.
        for i in 0..self.windows.len() {
            fill_column(
                &self.windows[i],
                &self.history,
                self.sample_rate,
                self.reference_hz,
                &mut self.scratch_input[i],
                &mut self.scratch_fft_out[i],
                &mut self.scratch_fft_work[i],
                &mut self.scratch_spectrum[i],
                &mut self.scratch_columns[i],
            );
            sqrt_warp_into(&self.scratch_columns[i], &mut self.scratch_warped[i]);
            self.scratch_norms[i] = spectrum_norm(&self.scratch_warped[i]);
        }
        if self.scratch_norms.iter().all(|&norm| norm <= 0.0) {
            return None;
        }

        let curve = self.blend(&self.scratch_warped, &self.scratch_norms);
        // The winner must be found *before* the frame is built, because it decides which of
        // the eight columns the frame carries. `tracked_bins` is called rather than
        // re-derived: see its docs for what re-deriving it cost last time.
        let candidates = tracked_bins(curve.len(), MIN_MIDI, BINS_PER_SEMITONE).unwrap();
        let winner = candidates
            .max_by(|&left, &right| curve[left].total_cmp(&curve[right]))
            .unwrap();

        // The column whose window is optimal for the winner: of the rungs that explain it,
        // the one with the largest λ, i.e. the nearer in log₂(N). That window's length was
        // chosen to resolve *this* candidate's partials, and resolving partials is precisely
        // what the frame is about to do with the column (`refine_on_partials`).
        //
        // The `unwrap` is contractual: every candidate has at least one rung. The ladder is
        // built by rounding log₂ of the domain's edges, so its end rungs sit within a
        // half-octave of those edges, while a rung's λ reaches a full octave either side —
        // `the_ladder_explains_every_candidate` holds it to that across sample rates.
        let WindowId(rung) = self.lambda[winner]
            .iter()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .unwrap()
            .0;
        // The winner's raw column is cloned out because the scratch it lives in is overwritten
        // next frame; this and the `curve` are the only two allocations a frame makes, and both
        // are data the returned `SalienceFrame` owns.
        Some(SalienceFrame::from_curve(
            curve,
            self.scratch_columns[rung].clone(),
            MIN_MIDI,
            BINS_PER_SEMITONE,
        ))
    }

    /// Camacho eq. 3-14: `S(b) = Σ_k λ_k(b)·S_k(b)`, over the ≤2 rungs that explain bin `b`.
    ///
    /// # Why each rung is divided by its own norm
    ///
    /// `S_k` is a **normalized** inner product in Camacho — `swipep.m` divides each window's
    /// column by its own L2 norm (`L = L./sqrt(sum(L.*L))`) before scoring it. Our
    /// [`SwipeKernel::salience`] divides only by the *kernel's* norm and leaves the spectral
    /// one to the caller (the bank applies it once, at `resolve`), so its output still scales
    /// with the column it was handed. Blending eight differently-scaled curves is not a thing
    /// one should do, and dividing the *curve* by the norm is the same as dividing the
    /// *column* by it — `salience` is linear in the column — so this is Camacho's arithmetic
    /// with one pass fewer.
    ///
    /// What the division demonstrably buys is the **scale**: a blend of normalized inner
    /// products is a convex combination of 0..1 confidences and is therefore itself one,
    /// which is the `norm = 1.0` invariant [`SalienceFrame::from_curve`] is built on. Skip it
    /// and `resolve` reports a strength of ~50 on a 0..1 scale.
    ///
    /// # What it does *not* buy — the kickstart's mine, defused by measurement
    ///
    /// The plan predicted that skipping this would silently break the whole track: `|X|` grows
    /// with the window, so long rungs' magnitudes should run ~2 orders above short ones', and
    /// since low candidates are the ones scored on long windows, a raw blend should tilt
    /// toward **low pitch — sub-octaves**. On a detector built to not hear the octave that
    /// isn't there, that would be the exact wrong thumb on the scale.
    ///
    /// It does not happen, and `the_norm_sets_the_scale_not_the_ranking` is that measurement.
    /// On the log grid the raw saliences of one candidate across the ladder peak at its
    /// **ideal** window and fall ~1.7× either side (A4: 32 · 55 · 55 · 46 · 33 · 26 from the
    /// 512-rung up) — not two orders, and not monotonic in N at all. The reading: a longer
    /// window makes a partial taller by `√N` *and* narrower in bins by about as much, since
    /// the grid is logarithmic and the lobe's width in Hz shrinks with N; the kernel sums over
    /// bins, so the two effects largely cancel. The raw blend picks the same note as the
    /// normalized one.
    ///
    /// Both halves are worth keeping straight: the division stays (it is Camacho's, and the
    /// confidence needs it), but it is **not** what stands between this track and a silently
    /// wrong answer. Whether raw-vs-normalized moves any *ranking* on real audio is an R2
    /// measurement, not a claim to make from a steady tone either way.
    fn blend(&self, warped: &[Vec<f32>], norms: &[f32]) -> Vec<f32> {
        let mut curve = vec![0.0f32; SPIRAL_BIN_COUNT];
        for (bin, taps) in self.lambda.iter().enumerate() {
            for &(WindowId(rung), lambda) in taps {
                if norms[rung] <= 0.0 {
                    // This rung is looking at digital silence while a longer one is not (a
                    // note's first milliseconds). Half the evidence is missing, so it
                    // contributes nothing and the candidate is attenuated — which is honest.
                    // `swipep.m` divides by zero here and suppresses the warning.
                    continue;
                }
                curve[bin] += lambda * self.kernel.salience(&warped[rung], bin) / norms[rung];
            }
        }
        curve
    }

    /// One rung's **raw** magnitude column on the shared grid — the allocating form, kept for
    /// the tests that read a single rung; [`Self::frame`] uses [`fill_column`] into scratch.
    #[cfg(test)]
    fn column(&self, window: &AnalysisWindow) -> Vec<f32> {
        let mut input = vec![0.0f32; window.size];
        let mut fft_out = window.fft.make_output_vec();
        let mut work = window.fft.make_scratch_vec();
        let mut spectrum = vec![0.0f32; window.size / 2];
        let mut out = vec![0.0f32; SPIRAL_BIN_COUNT];
        fill_column(
            window,
            &self.history,
            self.sample_rate,
            self.reference_hz,
            &mut input,
            &mut fft_out,
            &mut work,
            &mut spectrum,
            &mut out,
        );
        out
    }

    /// The delay this frontend owes a candidate at `hz`: half its window, in seconds.
    ///
    /// Reported rather than hidden because it is *pitch-dependent*, which no consumer of a
    /// frame can work out from the frame. `pitch_bench`'s lag sweep needs it to know where to
    /// look, and the melody line would need it to place a note in time.
    ///
    /// ⚠ Read it as a delay, not as a lag-sweep prediction: a sweep's peak is
    /// `delay + envelope`, and the envelope's share is a property of the *note*, not of this
    /// module (`docs/pitch_benchmark.md`, "A lag-sweep peak is not a group delay").
    pub(crate) fn delay_seconds(&self, hz: f32) -> f32 {
        let taps = &self.lambda[self.bin_of(hz)];
        // The blend's own weights: a candidate scored half from a 2048-window and half from a
        // 4096-window is late by the weighted mean of the two, not by either.
        taps.iter()
            .map(|&(WindowId(rung), lambda)| lambda * self.windows[rung].size as f32 / 2.0)
            .sum::<f32>()
            / self.sample_rate
    }

    /// The grid bin nearest `hz`, clamped to the grid. Private: bins are this module's
    /// internal coordinate, and the ones that leave it do so inside a [`SalienceFrame`].
    fn bin_of(&self, hz: f32) -> usize {
        let midi = 69.0 + 12.0 * (hz / self.reference_hz).log2();
        (((midi - MIN_MIDI) * BINS_PER_SEMITONE).round().max(0.0) as usize).min(SPIRAL_BIN_COUNT - 1)
    }
}

/// The ladder of power-of-two window lengths, one per octave, covering the tracked domain.
///
/// Camacho's rule verbatim (`swipep.m`: `logWs = round(log2(8·fs./plim))`, then every power
/// of two between). **`round`, not `ceil`**: the end rungs are allowed to miss the domain's
/// edges by up to half an octave, and λ's clamp covers the remainder ([`blend_weights`]).
///
/// So the ladder follows the sample rate, and both ends move: 8 rungs of {128 … 16384} at
/// 48 kHz (N_max = 341 ms), 8 rungs of {64 … 8192} at the corpus's 44.1 kHz (N_max = 186 ms).
/// The corpus therefore measures a slightly shorter-windowed detector than a 48 kHz device
/// runs — Camacho's rounding, applied honestly, not a discrepancy to patch out.
fn window_ladder(sample_rate: f32, reference_hz: f32) -> Vec<usize> {
    let ideal_length = |midi: f32| {
        let hz = midi_to_hz(midi, reference_hz);
        (PERIODS_PER_WINDOW * sample_rate / hz).log2().round() as u32
    };
    let shortest = ideal_length(TRACKED_MAX_MIDI);
    let longest = ideal_length(TRACKED_MIN_MIDI);
    (shortest..=longest).map(|exponent| 1usize << exponent).collect()
}

/// λ: which rungs explain each grid bin, and how much — Camacho's triangular interpolation
/// in `log₂(N)`, precomputed for the whole grid.
///
/// A candidate's ideal window is `8·fs/f` and the rungs are octave-spaced, so a candidate
/// sits between two of them and takes `1 − |log₂(f/pO_k)|` from each; the two weights sum to
/// 1. Interpolating **the curves** and not the spectra is what makes this cheap enough to be
/// worth doing: a spectral blend would have to be rebuilt per candidate, since each candidate
/// wants a different mix — which is a cross-correlation destroyed. The kernel's output, by
/// contrast, is already per-candidate, so the blend is a weighted sum of eight curves.
///
/// The normalization at the end is the ladder's ends: `swipep.m` spells them out as a special
/// case (`mu = ones(size(j))` for the first and last window), because a candidate beyond the
/// end rungs has no second rung to pair with and must not be silently attenuated for it.
/// Dividing by the sum says the same thing in one line — inside the ladder the sum is already
/// 1 and the division is a no-op.
fn blend_weights(windows: &[AnalysisWindow], reference_hz: f32) -> Vec<Vec<(WindowId, f32)>> {
    (0..SPIRAL_BIN_COUNT)
        .map(|bin| {
            let midi = MIN_MIDI + bin as f32 / BINS_PER_SEMITONE;
            let hz = midi_to_hz(midi, reference_hz);
            let mut taps: Vec<(WindowId, f32)> = windows
                .iter()
                .enumerate()
                .filter_map(|(rung, window)| {
                    let weight = 1.0 - (hz / window.optimal_hz).log2().abs();
                    (weight > 0.0).then_some((WindowId(rung), weight))
                })
                .collect();
            let total: f32 = taps.iter().map(|(_, weight)| weight).sum();
            for (_, weight) in &mut taps {
                *weight /= total;
            }
            taps
        })
        .collect()
}

/// One rung's **raw** magnitude column into `out`, allocating nothing: window the history into
/// `input`, real-FFT it into `fft_out` (with `work` as the transform's scratch), and resample
/// the magnitudes onto the shared log grid.
///
/// Free rather than a method so [`RtSwipe::frame`] can call it while holding disjoint mutable
/// borrows of its scratch fields — a `&self` method would borrow all of `self` and clash.
/// `out.len()` is [`SPIRAL_BIN_COUNT`]; `input.len()` is `window.size`, `fft_out.len()` is
/// `window.size / 2 + 1`.
fn fill_column(
    window: &AnalysisWindow,
    history: &[f32],
    sample_rate: f32,
    reference_hz: f32,
    input: &mut [f32],
    fft_out: &mut [Complex32],
    work: &mut [Complex32],
    spectrum: &mut [f32],
    out: &mut [f32],
) {
    // Right-aligned: the window ends at the newest sample. This is RT-SWIPE's one structural
    // change to SWIPE, and the reason the delay is N/2 rather than N_max/2.
    let start = history.len() - window.size;
    for (slot, (sample, taper)) in input.iter_mut().zip(history[start..].iter().zip(&window.taper)) {
        *slot = sample * taper;
    }
    // Real-input FFT: `input` is N reals, `fft_out` the N/2+1 bins DC..Nyquist. About half the
    // butterflies of the complex FFT this replaced, because a real signal's spectrum is
    // conjugate-symmetric and the upper half was always redundant. `process_with_scratch` takes
    // `work` from the caller so the audio thread still allocates nothing (see the fields). The
    // `unwrap` is contractual: the only errors are length mismatches, and the lengths are the
    // plan's own `make_*_vec` sizes, fixed at construction.
    window.fft.process_with_scratch(input, fft_out, work).unwrap();
    // |X|, not |X|², materialized **once** into `spectrum` (a low bin of the grid reads the
    // same FFT bin dozens of times, so norming on demand would recompute a `sqrt` per read —
    // measured 40 % slower). Only the first N/2 bins are taken, exactly as the complex path took
    // `&fft_buf[..N/2]`; the extra Nyquist bin the real FFT also returns sits above the grid's
    // top (4186 Hz) and is never resampled. The display's FFT reaches for `norm_sqr` because a
    // waterfall wants power; SWIPE warps the *magnitude*, with √ (`sqrt_warp`), and the thesis
    // picks that exponent on a worked example that is this app's exact signal — handing the
    // kernel power would be the √-warp cancelling into no warping at all.
    for (mag, bin) in spectrum.iter_mut().zip(&fft_out[..window.size / 2]) {
        *mag = bin.norm();
    }
    resample_to_log_grid(spectrum, sample_rate / window.size as f32, reference_hz, out);
}

/// √-warp `src` into `dst` (both [`SPIRAL_BIN_COUNT`] long) — the in-place form of
/// `swipe::sqrt_warp`, so a frame reuses `dst` instead of allocating.
fn sqrt_warp_into(src: &[f32], dst: &mut [f32]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = s.max(0.0).sqrt();
    }
}

/// Read a linear-frequency `spectrum` (`|X|`, bins `hz_per_bin` apart) at every bin of the
/// shared log-frequency grid, by linear interpolation, into `out`.
///
/// A **gather**, deliberately, where `analysis_math::splat_linear` scatters. The scatter is
/// right when the source is dense and the destination coarse (the display's spiral: many FFT
/// bins per note). Here it is the other way around at the bottom of the grid — 8
/// bins/semitone is 0.24 Hz apart at C1 while a 16384-point FFT at 48 kHz is 2.9 Hz apart —
/// so scattering would leave nine of every ten low bins empty and hand the kernel a comb of
/// holes to correlate against. Camacho interpolates for this reason (`swipep.m` splines it;
/// linear is the conservative choice, and the rounding the rest of this app's grids use).
fn resample_to_log_grid(spectrum: &[f32], hz_per_bin: f32, reference_hz: f32, out: &mut [f32]) {
    for (bin, slot) in out.iter_mut().enumerate() {
        let midi = MIN_MIDI + bin as f32 / BINS_PER_SEMITONE;
        let position = midi_to_hz(midi, reference_hz) / hz_per_bin;
        let left = position.floor() as usize;
        // Guards the grid against a rung that cannot reach its top. Unreachable for the shipped
        // ladder — even the shortest rung's Nyquist is 22 kHz against a grid that stops at
        // 4186 Hz — but the two extents are independent facts, and reading off the end of one
        // of them is not the failure to leave to chance.
        if left + 1 >= spectrum.len() {
            *slot = 0.0;
            continue;
        }
        let fraction = position - left as f32;
        *slot = spectrum[left] * (1.0 - fraction) + spectrum[left + 1] * fraction;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::super::swipe::sqrt_warp;
    use super::*;

    /// A tone with `partials` relative amplitudes, at `hz`.
    fn tone(hz: f32, sample_rate: f32, seconds: f32, partials: &[f32]) -> Vec<f32> {
        let len = (sample_rate * seconds) as usize;
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                partials
                    .iter()
                    .enumerate()
                    .map(|(harmonic, amplitude)| amplitude * (TAU * hz * (harmonic + 1) as f32 * t).sin())
                    .sum::<f32>()
                    / 3.0
            })
            .collect()
    }

    /// Drive the analyser over a signal at the bank's hop and return the last frame — i.e.
    /// score the tone once it is fully in every window.
    fn steady_frame(hz: f32, sample_rate: f32, partials: &[f32]) -> SalienceFrame {
        let mut rtswipe = RtSwipe::new(sample_rate, 440.0);
        // Long enough for the 16384-sample rung (341 ms) to be full of tone and nothing else.
        let samples = tone(hz, sample_rate, 0.8, partials);
        let hop = (sample_rate * 0.016) as usize;
        let mut fed = 0usize;
        while fed + hop <= samples.len() {
            rtswipe.process_samples(&samples[fed..fed + hop]);
            fed += hop;
        }
        rtswipe.frame().unwrap()
    }

    /// **Real-FFT ≡ complex-FFT, bin for bin** — the golden check the whole optimization
    /// rests on. Its only claim is that it changes throughput and nothing else, so this pins
    /// the shipped column to the exact path it replaced: the same right-aligned windowed
    /// history through a **full complex FFT** (rustfft directly, an independent implementation
    /// — the reference is *run*, not asserted from), the same `|X|` of the first N/2 bins, the
    /// same resample. If the two ever diverge past FFT rounding, "non-lossy" is false and every
    /// RPA/octave number measured after this commit is suspect — so the tolerance is a part in
    /// 1e4 of the column's own peak, not a loose "same note".
    #[test]
    fn the_real_fft_column_equals_the_complex_fft() {
        use rustfft::FftPlanner;

        let sample_rate = 48_000.0f32;
        let mut driven = RtSwipe::new(sample_rate, 440.0);
        // A real, harmonically rich tone so every rung's column carries something across the
        // grid — a divergence that only bit near-null bins would hide behind a pure sinusoid.
        let samples = tone(233.0, sample_rate, 0.8, &[1.0, 0.7, 0.5, 0.3, 0.2, 0.15]);
        driven.process_samples(&samples);

        let mut planner = FftPlanner::<f32>::new();
        println!("\n=== real-FFT column vs full complex FFT, per rung ===");
        for window in &driven.windows {
            // The reference: the path this commit deleted. Full complex FFT of the same
            // right-aligned windowed history, magnitudes of the first N/2 bins, same resample.
            let start = driven.history.len() - window.size;
            let mut buf: Vec<Complex32> = driven.history[start..]
                .iter()
                .zip(&window.taper)
                .map(|(sample, taper)| Complex32::new(sample * taper, 0.0))
                .collect();
            planner.plan_fft_forward(window.size).process(&mut buf);
            let mut spectrum = vec![0.0f32; window.size / 2];
            for (mag, bin) in spectrum.iter_mut().zip(&buf[..window.size / 2]) {
                *mag = bin.norm();
            }
            let mut reference = vec![0.0f32; SPIRAL_BIN_COUNT];
            resample_to_log_grid(&spectrum, sample_rate / window.size as f32, 440.0, &mut reference);

            let shipped = driven.column(window);
            // Absolute worst divergence measured against the column's peak: the two transforms
            // recombine their butterflies in a different order, so their bits differ, but the
            // magnitude of any bin must agree to a part in 1e4 of the largest bin.
            let peak = reference.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
            let worst = reference
                .iter()
                .zip(&shipped)
                .map(|(r, s)| (r - s).abs())
                .fold(0.0f32, f32::max);
            println!(
                "  N={:>6}  peak {peak:>10.2}  worst |Δ| {worst:.3e}  ({:.1e} rel)",
                window.size,
                worst / peak
            );
            assert!(
                worst < 1e-4 * peak,
                "N={}: real-FFT column diverges from the complex FFT by {worst:.3e} \
                 against peak {peak:.3e} — the optimization is not non-lossy",
                window.size
            );
        }
    }

    /// The ladder is what the kickstart predicted, at both rates that matter, and it is
    /// octave-spaced — which is the property that makes λ a two-tap blend.
    #[test]
    fn the_ladder_is_eight_octave_spaced_rungs() {
        assert_eq!(
            window_ladder(48_000.0, 440.0),
            vec![128, 256, 512, 1024, 2048, 4096, 8192, 16384],
            "48 kHz: N_max = 16384 = 341 ms"
        );
        assert_eq!(
            window_ladder(44_100.0, 440.0),
            vec![64, 128, 256, 512, 1024, 2048, 4096, 8192],
            "44.1 kHz: Camacho's `round` lands the whole ladder an octave down — the corpus \
             measures a shorter-windowed detector than a 48 kHz device runs, on purpose"
        );
    }

    /// **The ladder's contract**, and the one that justifies the `unwrap` in `frame`: every
    /// candidate in the tracked domain is explained by at least one rung, and its weights sum
    /// to exactly 1 — no candidate silently attenuated for sitting near an end of the ladder.
    ///
    /// Swept across sample rates because the ladder is derived from `fs` and the domain's
    /// edges by *rounding*: the claim is that the rounding can never open a hole, and a claim
    /// about rounding is worth checking at more than one operating point.
    #[test]
    fn the_ladder_explains_every_candidate() {
        for sample_rate in [8_000.0f32, 16_000.0, 22_050.0, 44_100.0, 48_000.0, 96_000.0] {
            let rtswipe = RtSwipe::new(sample_rate, 440.0);
            let candidates = tracked_bins(SPIRAL_BIN_COUNT, MIN_MIDI, BINS_PER_SEMITONE).unwrap();
            for bin in candidates {
                let taps = &rtswipe.lambda[bin];
                let midi = MIN_MIDI + bin as f32 / BINS_PER_SEMITONE;
                assert!(
                    !taps.is_empty(),
                    "{sample_rate} Hz: MIDI {midi:.2} has no window at all"
                );
                assert!(
                    taps.len() <= 2,
                    "{sample_rate} Hz: MIDI {midi:.2} blends {} windows — the rungs are no \
                     longer octave-spaced, and λ's whole shape assumes they are",
                    taps.len()
                );
                let total: f32 = taps.iter().map(|(_, weight)| weight).sum();
                assert!(
                    (total - 1.0).abs() < 1e-5,
                    "{sample_rate} Hz: MIDI {midi:.2} sums λ to {total:.6}, not 1"
                );
            }
        }
    }

    /// **The kickstart's mine, run rather than believed** — and it turns out to be a
    /// different animal than the plan described. See [`RtSwipe::blend`] for the full reading;
    /// this is the measurement it rests on, and it prints its evidence.
    ///
    /// Predicted: a raw blend tilts toward low pitch, because `|X|` grows with the window and
    /// long windows serve low candidates. Measured: the raw blend picks **the same note**,
    /// and one candidate's raw saliences across the ladder peak at its *ideal* rung rather
    /// than at the longest one. So this asserts what the division actually does — it makes
    /// the curve a **confidence** — and pins the falsified half in the opposite direction:
    /// if the raw blend ever *does* start losing A4, this test says so and the reasoning
    /// above needs redoing rather than quoting.
    #[test]
    fn the_norm_sets_the_scale_not_the_ranking() {
        let sample_rate = 48_000.0f32;
        let rtswipe = RtSwipe::new(sample_rate, 440.0);
        // A4: its ideal window is 8·48000/440 = 873 samples, so it is blended from the 512-
        // and 1024-rungs — the two whose disparity the blend has to reconcile.
        let samples = tone(440.0, sample_rate, 0.8, &[1.0, 0.8, 0.6, 0.35, 0.2]);
        let mut driven = RtSwipe::new(sample_rate, 440.0);
        driven.process_samples(&samples);

        let bin = driven.bin_of(440.0);
        println!("\n=== A4, one candidate, every rung's opinion of it ===");
        println!("     N     λ    raw salience    ÷ its own norm");
        let mut raw: Vec<f32> = Vec::new();
        let mut normalized: Vec<f32> = Vec::new();
        for (rung, window) in driven.windows.iter().enumerate() {
            let warped = sqrt_warp(&driven.column(window));
            let norm = spectrum_norm(&warped);
            let salience = driven.kernel.salience(&warped, bin);
            let lambda = driven.lambda[bin]
                .iter()
                .find(|(WindowId(id), _)| *id == rung)
                .map(|(_, weight)| *weight)
                .unwrap_or(0.0);
            println!(
                "  {:>6}  {lambda:.2}  {salience:>12.4}  {:>15.4}",
                window.size,
                salience / norm
            );
            if lambda > 0.0 {
                raw.push(salience);
                normalized.push(salience / norm);
            }
        }

        let _ = (rtswipe, raw, normalized);

        // The counterfactual: the same blend with the division left out. A copy of six lines
        // of `blend`, because the null hypothesis has to be *run*, and there is no old
        // function to run here (cf. `swipe::tests::the_old_comb_fails_the_same_column`).
        let warped: Vec<Vec<f32>> = driven
            .windows
            .iter()
            .map(|window| sqrt_warp(&driven.column(window)))
            .collect();
        let norms: Vec<f32> = warped.iter().map(|column| spectrum_norm(column)).collect();
        let mut raw_curve = vec![0.0f32; SPIRAL_BIN_COUNT];
        for (bin, taps) in driven.lambda.iter().enumerate() {
            for &(WindowId(rung), lambda) in taps {
                raw_curve[bin] += lambda * driven.kernel.salience(&warped[rung], bin);
            }
        }
        let shipped = driven.blend(&warped, &norms);

        let peak = |curve: &[f32]| {
            let bin = tracked_bins(curve.len(), MIN_MIDI, BINS_PER_SEMITONE)
                .unwrap()
                .max_by(|&left, &right| curve[left].total_cmp(&curve[right]))
                .unwrap();
            (MIN_MIDI + bin as f32 / BINS_PER_SEMITONE, curve[bin])
        };
        let (raw_midi, raw_peak) = peak(&raw_curve);
        let (norm_midi, norm_peak) = peak(&shipped);
        println!("\n  raw blend        : MIDI {raw_midi:.2}, peak {raw_peak:.4}");
        println!("  normalized blend : MIDI {norm_midi:.2}, peak {norm_peak:.4}  (truth A4 = 69)");

        assert!(
            (norm_midi - 69.0).abs() < 0.5,
            "the shipped blend lost A4: {norm_midi:.2}"
        );
        // What the division actually buys, on this signal: the curve is a *confidence*.
        // `from_curve` records that by setting `norm = 1.0`, so an unnormalized curve does not
        // merely rank oddly — it reports a strength of tens on a 0..1 scale.
        assert!(
            (0.0..=1.0).contains(&norm_peak),
            "normalized peak {norm_peak:.4} is not a confidence"
        );
        assert!(
            raw_peak > 1.0,
            "the raw peak is {raw_peak:.4} — if it already fits 0..1 then the division \
             changes nothing anywhere and this module's `norm = 1.0` needs re-arguing"
        );
    }

    /// **The golden test, carried across the frontend.** A violin open G: the body barely
    /// radiates 196 Hz, so the fundamental is a tenth of the 2nd harmonic, and the phantom
    /// octave at G4 is the error this whole detector exists to not make.
    ///
    /// The bank's kernel passes this on a synthetic *column*
    /// (`swipe::tests::violin_g_with_a_weak_fundamental_decides_g3`). This passes it on
    /// synthetic *audio*, through eight FFTs — same claim, same kernel, different frontend.
    /// If the frontend is what breaks it, the two tests disagree and say exactly where.
    #[test]
    fn violin_g_with_a_weak_fundamental_decides_g3() {
        let frame = steady_frame(196.0, 48_000.0, &[0.08, 1.0, 0.7, 0.5, 0.4, 0.3, 0.22]);
        let (midi, strength) = frame.argmax().unwrap();
        println!("\n=== violin open G through the FFT ladder ===");
        println!("  verdict MIDI {midi:.2} (G3 = 55, phantom G4 = 67), strength {strength:.3}");
        assert!((midi - 55.0).abs() < 0.5, "expected G3 (55), got {midi:.2}");
        assert!(
            (0.0..=1.0).contains(&strength),
            "strength {strength:.3} is outside 0..1 — the `norm = 1.0` invariant of \
             `from_curve` is broken, i.e. the curve was not normalized before the blend"
        );
    }

    /// A tone lands on its own note at every octave the ladder is built for. A single-rung
    /// test would pass with the blend broken; only crossing the rungs can catch a seam
    /// between them.
    ///
    /// # 🔎 FINDING: a **systematic −13 cents**, and it is not where it looks
    ///
    /// Every octave reads ~13 cents flat — one grid bin (1/8 semitone = 12.5 ¢) — and the
    /// same bin at each, which the ladder explains: rung length doubles with the pitch, so
    /// every octave presents the FFT with the *same* geometry and therefore the same error.
    ///
    /// The obvious suspect was [`refine_on_partials`], whose ±2-bin centroid is calibrated
    /// for the bank's two-bin `splat_linear` footprint and meets lobes here that are 4 to 60
    /// bins wide. **It is not the culprit**: this prints the curve's own peak bin beside the
    /// refined pitch, and the *coarse* peak is already 13 ¢ flat, with the refine agreeing to
    /// within a cent. So the bias is in the column — the frontend — not in reading the fine
    /// pitch off it.
    ///
    /// The lever, unmeasured and named here so it is not re-derived: this resamples the
    /// spectrum **linearly** (the kickstart's decision), while Camacho splines it
    /// (`swipep.m`: `interp1(f, abs(X), fERBs, 'spline')`). A rung's lobe is only ~4 FFT bins
    /// wide by construction — 8 periods of the candidate — so how the gaps between those four
    /// samples are filled is not a detail, and a piecewise-linear fill has no way to place an
    /// apex the samples straddle.
    ///
    /// Left as it stands, deliberately: R2 scores RPA at a **50-cent** tolerance, where 13 ¢
    /// is immaterial, and the fix is a trade (a spline per rung per frame) that wants the
    /// corpus numbers to argue with, not a synthetic tone. It matters for R3's tuner-facing
    /// roles, where the app's budget is 20 ¢ — see the kickstart.
    #[test]
    fn a_tone_lands_on_its_own_note_across_the_ladder() {
        println!("\n=== one tone per octave, through the whole ladder ===");
        println!("  truth   coarse peak    refined    error");
        for midi_truth in [36.0f32, 48.0, 60.0, 72.0, 84.0, 96.0] {
            let hz = midi_to_hz(midi_truth, 440.0);
            let frame = steady_frame(hz, 48_000.0, &[1.0, 0.5, 0.25]);
            let (midi, _strength) = frame.argmax().unwrap();
            // The curve's own peak bin, before `refine_on_partials` reads the fine pitch off
            // the column: it separates "the kernel chose the wrong series" from "the series
            // was right and the fine pitch is biased", which want opposite fixes.
            let coarse = frame
                .tracked_bins()
                .unwrap()
                .max_by(|&left, &right| frame.salience_at(left).total_cmp(&frame.salience_at(right)))
                .unwrap();
            println!(
                "  {midi_truth:>5.0}  {:>11.2}  {midi:>9.2}  {:>+6.0}¢",
                frame.midi_of_bin(coarse),
                (midi - midi_truth) * 100.0
            );
            assert!(
                (midi - midi_truth).abs() < 0.5,
                "MIDI {midi_truth} ({hz:.1} Hz) read as {midi:.2}"
            );
        }
    }

    /// **What real-FFT bought, measured** — printed like the delay probe, not asserted, because
    /// a timing is machine-specific and a debug build's is noise (run `--release`). Times one
    /// ladder of the shipped `process_with_scratch` against the full complex FFT it replaced,
    /// each rung including its identical windowing copy, so the ratio is the FFT's own share.
    #[test]
    #[ignore = "timing probe — run with --release --ignored --nocapture"]
    fn what_did_the_real_fft_buy() {
        use std::time::Instant;

        use rustfft::FftPlanner;

        let sample_rate = 48_000.0f32;
        let mut driven = RtSwipe::new(sample_rate, 440.0);
        driven.process_samples(&tone(233.0, sample_rate, 0.8, &[1.0, 0.7, 0.5, 0.3]));
        let start = driven.history.len();

        let mut complex_planner = FftPlanner::<f32>::new();
        const ITERS: u32 = 4000;
        println!("\n=== FFT cost per rung: full-complex (deleted) vs real (shipped), {ITERS}× ===");
        println!("       N      complex        real     speedup");
        let (mut sum_complex, mut sum_real) = (0u128, 0u128);
        for window in &driven.windows {
            let from = start - window.size;

            let complex = complex_planner.plan_fft_forward(window.size);
            let mut cbuf = vec![Complex32::new(0.0, 0.0); window.size];
            let t = Instant::now();
            for _ in 0..ITERS {
                for (slot, (s, taper)) in cbuf
                    .iter_mut()
                    .zip(driven.history[from..].iter().zip(&window.taper))
                {
                    *slot = Complex32::new(s * taper, 0.0);
                }
                complex.process(&mut cbuf);
            }
            let c_ns = t.elapsed().as_nanos() / ITERS as u128;

            let mut input = vec![0.0f32; window.size];
            let mut out = window.fft.make_output_vec();
            let mut work = window.fft.make_scratch_vec();
            let t = Instant::now();
            for _ in 0..ITERS {
                for (slot, (s, taper)) in input
                    .iter_mut()
                    .zip(driven.history[from..].iter().zip(&window.taper))
                {
                    *slot = s * taper;
                }
                window
                    .fft
                    .process_with_scratch(&mut input, &mut out, &mut work)
                    .unwrap();
            }
            let r_ns = t.elapsed().as_nanos() / ITERS as u128;

            sum_complex += c_ns;
            sum_real += r_ns;
            println!(
                "  {:>6}  {c_ns:>8} ns  {r_ns:>8} ns  {:.2}×",
                window.size,
                c_ns as f32 / r_ns as f32
            );
        }
        println!(
            "  ladder  {sum_complex:>8} ns  {sum_real:>8} ns  {:.2}×  (all eight, one frame)",
            sum_complex as f32 / sum_real as f32
        );
    }

    /// The delay is pitch-dependent and this is the shape of it — the number R3 would trade
    /// against pYIN's 128 ms, printed rather than asserted because it is a property to read,
    /// not a bound to defend.
    #[test]
    fn what_does_the_ladder_cost_in_delay() {
        let rtswipe = RtSwipe::new(48_000.0, 440.0);
        println!("\n=== RT-SWIPE delay = half the blended window (4 periods) ===");
        println!("  the bank pays 8–29 ms flat; pYIN pays ~128 ms");
        for (name, hz) in [
            ("C1", 32.7f32),
            ("G3 (open G)", 196.0),
            ("A4", 440.0),
            ("E7", 2637.0),
        ] {
            println!(
                "  {name:>12} {hz:>7.1} Hz : {:>6.1} ms",
                rtswipe.delay_seconds(hz) * 1000.0
            );
        }
    }
}
