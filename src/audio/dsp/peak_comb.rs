//! A peak comb with between-harmonic penalties — a deliberately simple FFT pitch detector,
//! kept as a reference point for the benchmark.
//!
//! # Why it is here
//!
//! One FFT, a six-harmonic comb, a six-frame moving average: every part of it is weaker than
//! what `dsp::sieve` or `dsp::rtswipe` do, yet detectors of exactly this shape are what
//! commercial vocal monitors are built from, and on a scrolling piano roll they look *calm*.
//! Running it through `pitch_bench` puts a number on that and says which of its few decisions
//! carry it — `docs/peak_comb.md`. It remains a yardstick when selected in the app.
//!
//! Its constants are fixed reference settings, not tuned on the benchmark corpus: the point
//! is to measure a design, and a design re-tuned to the ruler measures the ruler.
//!
//! # The algorithm
//!
//! 1. **Level gate** on the mean |x| of the raw frame, in dBFS ([`DEFAULT_GATE_DBFS`]).
//! 2. **Hann window, 4096 samples**, spectrum in dBFS (`20·log10(2|X| / Σw)`).
//! 3. **Whitening**: the dB spectrum minus its own 31-bin moving average (edges reflected),
//!    so a bin's value is how far it stands above its neighbourhood — formants flattened.
//! 4. **Peaks** of the *raw* dB spectrum in 55–4200 Hz: strict local maxima no lower than
//!    35 dB under the frame's loudest bin.
//! 5. **Candidates** are the peaks themselves, never their fractions: each peak, refined by a
//!    parabola, spawns seven candidates at `p − 0.6 … p + 0.6` bins in steps of 0.2. A
//!    fundamental that is not a visible peak cannot be found.
//! 6. **HNR gate** per candidate: `Σ_{k=1..11} max(|X| in ±1 bin of k·f0)² / Σ|X|²` must be
//!    ≥ 0.13 to be scored at all, and the winner's must be ≥ 0.22.
//! 7. **Score** on the whitened spectrum `S` (linear interpolation between bins):
//!    `S(f0) ≥ 1.5 dB` or the candidate is dropped; then
//!    `2.1·S(f0) + Σ_{k=2..6} S(k·f0) − Σ_r w_r·max(S(r·f0), 0)` over the "negative harmonics"
//!    `r ∈ {½, 1½, 2½, ⅓, ⅔, 1⅓, 1⅔, ⅕, ⅖, ⅗}` — energy between the candidate's harmonics
//!    says the true pitch is lower. The negative half applies only above 110 Hz.
//! 8. **Smoothing**, in semitones (`12·log2(f/27.5)`, *not* Hz): a frame more than 10
//!    semitones from the previous *raw* frame is replaced by "no pitch" (a jump is accepted on
//!    its second frame), and the output is the mean of the last 6 frames that had a pitch.

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
use crate::core_types::pitch::{
    Hz,
    Midi,
};

/// Analysis window, fixed whatever the sample rate.
pub(crate) const WINDOW: usize = 4096;

/// Frame cadence. The smoothing counts frames, so it is part of the design: at another hop
/// the same six-frame average would be a different filter.
pub(crate) const HOP_SECONDS: f32 = 0.008;

/// The level gate a phone microphone wants: frames whose mean |x| is under this are silence.
///
/// Too high for quiet recordings — on MDB-stem-synth's violin stems it silences a third of
/// the voiced frames — so the benchmark can lower it (`PITCH_BENCH_COMB_GATE_DBFS`).
pub(crate) const DEFAULT_GATE_DBFS: f32 = -45.0;

const MIN_HZ: f32 = 55.0;
const MAX_HZ: f32 = 4200.0;

/// Peaks more than this far under the frame's loudest bin are floor.
const PEAK_RANGE_DB: f32 = 35.0;

/// Width of the whitening moving average — odd, centred, in bins.
const WHITENING_BINS: usize = 31;

/// A candidate's own bin must stand at least this many dB above its neighbourhood.
const MIN_FUNDAMENTAL_CONTRAST_DB: f32 = 1.5;

/// Candidate grid around each refined peak: `offset + step·k`, `k = 0..=CANDIDATE_STEPS`.
const CANDIDATE_OFFSET_BINS: f32 = -0.6;
const CANDIDATE_STEP_BINS: f32 = 0.2;
const CANDIDATE_STEPS: usize = 6;

/// Harmonics that vote for a candidate, `(ratio, weight)`: the fundamental counts 2.1×.
const POSITIVE_HARMONICS: [(f32, f32); 6] = [
    (1.0, 2.1),
    (2.0, 1.0),
    (3.0, 1.0),
    (4.0, 1.0),
    (5.0, 1.0),
    (6.0, 1.0),
];

/// Places between the candidate's harmonics that vote against it, `(ratio, weight)`:
/// halves (it is an octave too high), thirds (a twelfth too high), fifths.
const NEGATIVE_HARMONICS: [(f32, f32); 10] = [
    (0.5, 1.8),
    (1.5, 1.0),
    (2.5, 1.0),
    (1.0 / 3.0, 1.8),
    (2.0 / 3.0, 1.0),
    (4.0 / 3.0, 1.0),
    (5.0 / 3.0, 1.0),
    (0.2, 1.0),
    (0.4, 1.0),
    (0.6, 1.0),
];

/// Below this the negative harmonics are not charged: their ratios would land under the
/// search range, where the spectrum is rumble.
const NEGATIVE_HARMONICS_ABOVE_HZ: f32 = 110.0;

/// Harmonics summed into the harmonic-to-noise ratio.
const HNR_HARMONICS: usize = 11;
/// A candidate under this HNR is not scored.
const HNR_CANDIDATE: f32 = 0.13;
/// The winner under this HNR means no pitch.
const HNR_WINNER: f32 = 0.22;

/// dBFS given to an exactly-zero bin, so the log stays finite.
const ZERO_BIN_DBFS: f32 = -150.0;

/// Smoothing: frames averaged, and the jump in semitones that gets a frame thrown out.
const SMOOTHING_FRAMES: usize = 6;
const JUMP_SEMITONES: f32 = 10.0;

/// A position on the FFT's frequency axis, in bins, between bins allowed.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct FracBin(f32);

/// The smoothing's pitch unit: semitones above A0 (27.5 Hz), i.e. MIDI − 21 at A4 = 440 Hz.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct SemitonesAboveA0(f32);

impl SemitonesAboveA0 {
    fn from_hz(hz: Hz) -> Self {
        SemitonesAboveA0(12.0 * (hz.0 / 27.5).log2())
    }

    fn midi(self) -> f32 {
        self.0 + 21.0
    }
}

pub(crate) struct PeakComb {
    sample_rate:    SampleRate,
    taper:          Vec<f32>,
    /// `Σ taper`: `|X|` of a full-scale sine is half of it.
    taper_sum:      f32,
    fft:            Arc<dyn RealToComplex<f32>>,
    /// Level gate, dBFS of mean |x|.
    gate_dbfs:      f32,
    start_bin:      usize,
    end_bin:        usize,
    negative_above: FracBin,
    /// The newest [`WINDOW`] samples, oldest first.
    history:        Vec<f32>,
    /// The previous frame's raw (unsmoothed) answer — what a jump is measured from.
    last_raw:       Option<SemitonesAboveA0>,
    /// The last [`SMOOTHING_FRAMES`] smoothing inputs, oldest first. `None` is a frame that
    /// had no pitch or was thrown out as a jump; it does not vote.
    recent:         [Option<SemitonesAboveA0>; SMOOTHING_FRAMES],

    // Scratch, reused frame to frame.
    input:     Vec<f32>,
    fft_out:   Vec<Complex32>,
    work:      Vec<Complex32>,
    /// `|X|` per bin, DC..Nyquist.
    magnitude: Vec<f32>,
    /// dBFS per bin.
    levels:    Vec<f32>,
    /// `levels` minus its moving average.
    whitened:  Vec<f32>,
    /// `levels` reflected by `WHITENING_BINS / 2` at both ends.
    padded:    Vec<f32>,
}

impl PeakComb {
    /// `gate_dbfs`: frames whose mean |x| is under this are silence ([`DEFAULT_GATE_DBFS`]).
    pub(crate) fn new(sample_rate: SampleRate, gate_dbfs: f32) -> Self {
        let taper = hann_taper(WINDOW);
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let bins_per_hz = WINDOW as f32 / sample_rate.hz();
        let bins = WINDOW / 2 + 1;
        Self {
            sample_rate,
            taper_sum: taper.iter().sum(),
            gate_dbfs,
            start_bin: (MIN_HZ * bins_per_hz).round() as usize,
            // Clamped to the Nyquist bin.
            end_bin: ((MAX_HZ * bins_per_hz).round() as usize).min(WINDOW / 2),
            negative_above: FracBin(NEGATIVE_HARMONICS_ABOVE_HZ * bins_per_hz),
            history: vec![0.0; WINDOW],
            last_raw: None,
            recent: [None; SMOOTHING_FRAMES],
            input: vec![0.0; WINDOW],
            fft_out: fft.make_output_vec(),
            work: fft.make_scratch_vec(),
            fft,
            magnitude: vec![0.0; bins],
            levels: vec![0.0; bins],
            whitened: vec![0.0; bins],
            padded: vec![0.0; bins + WHITENING_BINS - 1],
            taper,
        }
    }

    pub(crate) fn process_samples(&mut self, samples: &[f32]) {
        slide_in(&mut self.history, samples);
    }

    /// The smoothed answer for this frame: raw estimate, then smoothing. Advances the
    /// smoothing state, so call it exactly once per frame.
    ///
    /// MIDI rather than Hz because the smoothing averages in semitones: the answer is a mean
    /// of pitches, and there is no Hz it was computed in.
    #[cfg(test)]
    pub(crate) fn pitch(&mut self) -> Option<Midi> {
        let raw = self.raw_pitch();
        self.smooth_pitch(raw)
    }

    /// Advance the reference mean from the very same raw estimate used by the trellis.
    pub(crate) fn smooth_pitch(&mut self, raw: Option<Hz>) -> Option<Midi> {
        self.smooth(raw.map(SemitonesAboveA0::from_hz))
            .map(|st| Midi(st.midi()))
    }

    /// The frame on its own, no smoothing.
    pub(crate) fn raw_pitch(&mut self) -> Option<Hz> {
        let mean_abs = self.history.iter().map(|s| s.abs()).sum::<f32>() / WINDOW as f32;
        if 20.0 * mean_abs.log10() < self.gate_dbfs {
            return None;
        }
        self.fill_spectrum();

        let mut best: Option<(FracBin, f32, f32)> = None; // (bin, score, hnr)
        for peak in self.peaks() {
            let refined = FracBin(parabolic_vertex(&self.levels, peak).0);
            for step in 0..=CANDIDATE_STEPS {
                let bin = FracBin(refined.0 + CANDIDATE_OFFSET_BINS + CANDIDATE_STEP_BINS * step as f32);
                if bin.0 <= 0.0 {
                    continue;
                }
                let hnr = self.harmonic_to_noise(bin);
                if hnr < HNR_CANDIDATE {
                    continue;
                }
                let Some(score) = self.harmonic_score(bin) else {
                    continue;
                };
                // Strictly greater: of equal scores the earlier candidate stays.
                if best.is_none_or(|(_, b, _)| score > b) {
                    best = Some((bin, score, hnr));
                }
            }
        }
        let (bin, _, hnr) = best?;
        (hnr >= HNR_WINNER).then(|| Hz(bin.0 * self.sample_rate.hz() / WINDOW as f32))
    }

    /// Window, FFT, `|X|`, dBFS, and the whitened dBFS.
    fn fill_spectrum(&mut self) {
        for (slot, (sample, taper)) in self.input.iter_mut().zip(self.history.iter().zip(&self.taper)) {
            *slot = sample * taper;
        }
        // Contractual: every length is the plan's own, fixed at construction.
        self.fft
            .process_with_scratch(&mut self.input, &mut self.fft_out, &mut self.work)
            .unwrap();
        for ((magnitude, level), bin) in self.magnitude.iter_mut().zip(&mut self.levels).zip(&self.fft_out) {
            *magnitude = bin.norm();
            *level = if *magnitude > 0.0 {
                20.0 * (2.0 * *magnitude / self.taper_sum).log10()
            } else {
                ZERO_BIN_DBFS
            };
        }

        subtract_moving_mean(&self.levels, WHITENING_BINS, &mut self.whitened, &mut self.padded);
    }

    /// Peaks over `start_bin..=end_bin`: strict local maxima of the raw dB spectrum within
    /// [`PEAK_RANGE_DB`] of the loudest bin **anywhere** (DC included).
    fn peaks(&self) -> Vec<usize> {
        let loudest = self.levels.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let floor = loudest - PEAK_RANGE_DB;
        let lo = self.start_bin.max(1);
        let hi = self.end_bin.min(self.levels.len() - 2);
        (lo..=hi)
            .filter(|&i| {
                let level = self.levels[i];
                level > self.levels[i - 1] && level > self.levels[i + 1] && level >= floor
            })
            .collect()
    }

    /// Harmonic-to-noise ratio on the linear `|X|`: the energy of the strongest bin in ±1 of
    /// each of the first [`HNR_HARMONICS`] harmonics, over the frame's total energy.
    fn harmonic_to_noise(&self, f0: FracBin) -> f32 {
        let total: f32 = self.magnitude.iter().map(|m| m * m).sum();
        if total <= 0.0 {
            return 0.0;
        }
        let len = self.magnitude.len();
        let mut harmonic = 0.0f32;
        for k in 1..=HNR_HARMONICS {
            let centre = k as f32 * f0.0;
            if centre >= len as f32 {
                break;
            }
            let i = centre.round() as usize;
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(len - 1);
            let strongest = self.magnitude[lo..=hi]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            harmonic += strongest * strongest;
        }
        harmonic / total
    }

    /// The comb's score on the whitened spectrum. `None`: the candidate's own bin does not
    /// stand out from its neighbourhood, so it is not a candidate at all.
    fn harmonic_score(&self, f0: FracBin) -> Option<f32> {
        if interpolate(&self.whitened, f0) < MIN_FUNDAMENTAL_CONTRAST_DB {
            return None;
        }
        let mut score: f32 = POSITIVE_HARMONICS
            .iter()
            .map(|(ratio, weight)| weight * interpolate(&self.whitened, FracBin(ratio * f0.0)))
            .sum();
        if f0 > self.negative_above {
            for (ratio, weight) in NEGATIVE_HARMONICS {
                let between = interpolate(&self.whitened, FracBin(ratio * f0.0));
                if between > 0.0 {
                    score -= weight * between;
                }
            }
        }
        Some(score)
    }

    /// Jump rejection, then the mean of the frames that had a pitch — see the module docs.
    fn smooth(&mut self, raw: Option<SemitonesAboveA0>) -> Option<SemitonesAboveA0> {
        let jumped = matches!((self.last_raw, raw), (Some(last), Some(now)) if (now.0 - last.0).abs() >= JUMP_SEMITONES);
        self.last_raw = raw;
        self.recent.rotate_left(1);
        self.recent[SMOOTHING_FRAMES - 1] = if jumped { None } else { raw };
        let voting: Vec<f32> = self.recent.iter().flatten().map(|st| st.0).collect();
        (!voting.is_empty()).then(|| SemitonesAboveA0(voting.iter().sum::<f32>() / voting.len() as f32))
    }
}

/// Between-bin read of `values`; 0 outside the array, the last value exactly at its end.
fn interpolate(values: &[f32], at: FracBin) -> f32 {
    let last = (values.len() - 1) as f32;
    if at.0 < 0.0 || at.0 >= last {
        return if at.0 == last {
            values[values.len() - 1]
        } else {
            0.0
        };
    }
    let i = at.0.floor() as usize;
    let t = at.0 - i as f32;
    values[i] + t * (values[i + 1] - values[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: SampleRate = SampleRate(44_100);

    /// `len` samples of a harmonic tone: `amplitudes[i]` is harmonic `i + 1`.
    fn tone(f0: f32, amplitudes: &[f32]) -> Vec<f32> {
        (0..WINDOW)
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

    fn raw(signal: &[f32]) -> Option<Hz> {
        let mut detector = PeakComb::new(SR, DEFAULT_GATE_DBFS);
        detector.process_samples(signal);
        detector.raw_pitch()
    }

    fn cents(estimate: Hz, truth: f32) -> f32 {
        1200.0 * (estimate.0 / truth).log2()
    }

    fn bright() -> Vec<f32> {
        (1..=12).map(|k| 1.0 / k as f32).collect()
    }

    #[test]
    fn finds_a_bright_tone_within_ten_cents() {
        for midi in [43, 55, 62, 69, 81, 93] {
            let f0 = 440.0 * ((midi as f32 - 69.0) / 12.0).exp2();
            let found = raw(&tone(f0, &bright())).unwrap();
            // The candidate grid is 0.2 bin (~2 Hz), so low notes are only that fine.
            assert!(
                cents(found, f0).abs() < 10.0 + 1200.0 * (1.0 + 2.2 / f0).log2(),
                "MIDI {midi}: {found:?}"
            );
        }
    }

    /// The defining blind spot: candidates come only from peaks, so a fundamental that is not
    /// in the spectrum cannot be reported — the answer is an upper partial or nothing.
    #[test]
    fn missing_fundamental_is_not_found() {
        let mut amplitudes = bright();
        amplitudes[0] = 0.0;
        if let Some(found) = raw(&tone(220.0, &amplitudes)) {
            assert!(cents(found, 220.0).abs() > 600.0, "{found:?}");
        }
    }

    #[test]
    fn silence_is_gated() {
        assert!(raw(&vec![0.0; WINDOW]).is_none());
        // −60 dBFS sine: mean |x| is far under the default −45 dBFS gate.
        let quiet: Vec<f32> = tone(220.0, &bright()).iter().map(|s| s * 1e-3).collect();
        assert!(raw(&quiet).is_none());
    }

    #[test]
    fn smoothing_drops_a_lone_jump_and_averages_in_semitones() {
        let mut detector = PeakComb::new(SR, DEFAULT_GATE_DBFS);
        let a = SemitonesAboveA0(48.0);
        let far = SemitonesAboveA0(60.0);
        assert_eq!(detector.smooth(Some(a)), Some(a));
        // 12 semitones from the last raw frame: thrown out, the average holds.
        assert_eq!(detector.smooth(Some(far)), Some(a));
        // Second frame at the new pitch is accepted and averaged in.
        assert_eq!(detector.smooth(Some(far)), Some(SemitonesAboveA0(54.0)));
        assert_eq!(detector.smooth(None), Some(SemitonesAboveA0(54.0)));
    }

    #[test]
    fn whitening_of_a_flat_spectrum_is_zero() {
        let mut detector = PeakComb::new(SR, DEFAULT_GATE_DBFS);
        // An impulse has a flat |X|.
        let mut impulse = vec![0.0; WINDOW];
        impulse[WINDOW / 2] = 1.0;
        detector.process_samples(&impulse);
        detector.fill_spectrum();
        // Hann-windowed impulse at the centre is still flat in magnitude.
        assert!(
            detector.whitened.iter().all(|w| w.abs() < 1e-3),
            "{:?}",
            &detector.whitened[..8]
        );
    }
}
