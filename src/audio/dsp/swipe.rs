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
        // `ResonatorViewSettings::default().gamma` — the *display's* contrast, which the
        // shipped `resonator_snapshot` bakes into the column before scoring it. Undone
        // below to recover the bank's own magnitudes; see design §1.4 for why a display
        // knob must not reach a detector in the first place.
        const DISPLAY_GAMMA: f32 = 0.72;
        const G3_MIDI: f32 = 55.0;
        const G4_MIDI: f32 = 67.0;

        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = NOTE_BUCKET_MIN_MIDI as f32;
        let kernel = SwipeKernel::new(bps);

        for name in ["g_open_slow_strokes", "g_open_fast_strokes", "g_open_real_octave"] {
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
            let mut new_verdicts: Vec<f32> = Vec::new();

            let mut fed = 0usize;
            while fed + hop <= samples.len() {
                analyzer.process_samples(&samples[fed..fed + hop], true);
                fed += hop;
                let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);

                let Some((old_midi, _strength)) = snapshot.fundamental else {
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

                // Undo the display's gamma to recover the bank's magnitudes up to the
                // column max: `normalize_bars` computes `(v/max)^gamma` and the clamp is a
                // no-op because `max` is the max. The residual `1/max` is a global scale,
                // and SWIPE's numerator is linear in `√|X|` with a candidate-independent
                // denominator, so it cannot move the argmax.
                let magnitudes: Vec<f32> = snapshot
                    .spectrum
                    .iter()
                    .map(|v| v.powf(1.0 / DISPLAY_GAMMA))
                    .collect();
                let warped = sqrt_warp(&magnitudes);
                let curve = kernel.salience_curve(&warped);
                let best = curve
                    .iter()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(bin, _)| bin)
                    .unwrap();
                let new_midi = min_midi + best as f32 / bps;
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
                "  SWIPE'   : G3 {new_g3:>4} ({:>5.1}%)   G4          {new_g4:>4} ({:>5.1}%)   other {new_other:>4} ({:>5.1}%)",
                pct(new_g3),
                pct(new_g4),
                pct(new_other)
            );
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
