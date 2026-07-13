//! Probabilistic YIN (Mauch & Dixon, 2014) — the octave-robust pitch anchor.
//!
//! Plain YIN commits to *one* period per frame at a single threshold, so a bowed
//! string (whose 2nd harmonic drifts louder/quieter than the fundamental) makes it
//! flip between f0 and 2·f0 frame to frame — the "octave wandering" the violin
//! staff suffered. pYIN fixes this in two stages:
//!
//! 1. **Candidates** — instead of one threshold, integrate YIN's pick over a whole
//!    *distribution* of thresholds (a Beta prior, Mauch's a≈2 b≈18, mean ≈0.1).
//!    Each distinct period the picks land on becomes a candidate weighted by the
//!    prior mass of the thresholds that chose it; the mass of thresholds that found
//!    no dip at all is the frame's *unvoiced* probability. So a frame emits several
//!    weighted pitch hypotheses (typically f0 and its sub/over-octave), not one.
//! 2. **HMM** — an online Viterbi over pitch states (10-cent bins) + an unvoiced
//!    state. Transition costs make octave jumps expensive and staying cheap, so the
//!    smoothest *path* is decoded: a lone octave-outlier frame can't move it. Real
//!    note changes re-enter through the unvoiced state (which reaches any pitch),
//!    so genuine leaps still track. Decoded greedily (argmax of the forward
//!    trellis each frame) for zero added latency.
//!
//! Output is an absolute frequency (taken from the winning candidate, so it keeps
//! sub-cent precision for the tuner) + the voiced probability as a clarity. The
//! violin staff snaps its fast resonator pitch to this frequency's *octave*.

use super::analysis_math::parabolic_tau;
use super::pitch::{
    Cmndf,
    cmndf,
};

// --- Candidate stage ---------------------------------------------------------

/// Threshold samples for the Beta prior. 100 is plenty: the picks quantise to
/// integer lags, so more thresholds only refine the probability weights.
const N_THRESHOLDS: usize = 100;
/// Beta(a, b) shape for the threshold prior (Mauch & Dixon). Mean a/(a+b) = 0.1
/// sits right at YIN's usual 0.1–0.15 operating threshold, with mass spread so
/// both cleaner (deeper) and noisier (shallower) dips get represented.
const BETA_A: f32 = 2.0;
const BETA_B: f32 = 18.0;

/// One pitch hypothesis for a frame: a frequency and the prior mass behind it.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    frequency_hz: f32,
    probability:  f32,
}

/// Precomputed Beta-prior threshold grid (samples + normalized weights summing
/// to 1). Built once and reused every frame.
struct ThresholdPrior {
    thresholds: [f32; N_THRESHOLDS],
    weights:    [f32; N_THRESHOLDS],
}

impl ThresholdPrior {
    fn new() -> Self {
        let mut thresholds = [0.0f32; N_THRESHOLDS];
        let mut weights = [0.0f32; N_THRESHOLDS];
        let mut sum = 0.0f32;
        for i in 0..N_THRESHOLDS {
            // Midpoints of N equal bins in (0, 1); Beta pdf up to its normalizer,
            // which cancels when we renormalize the sampled weights to sum 1.
            let s = (i as f32 + 0.5) / N_THRESHOLDS as f32;
            let w = s.powf(BETA_A - 1.0) * (1.0 - s).powf(BETA_B - 1.0);
            thresholds[i] = s;
            weights[i] = w;
            sum += w;
        }
        for w in &mut weights {
            *w /= sum;
        }
        Self {
            thresholds,
            weights,
        }
    }
}

/// YIN's absolute-threshold pick for one threshold: the first lag whose CMNDF dips
/// below `threshold`, walked down to the bottom of that dip. `None` if the curve
/// never crosses (this threshold votes "unvoiced").
fn yin_pick(c: &Cmndf, threshold: f32) -> Option<usize> {
    for tau in c.min_lag..c.max_lag {
        if c.d[tau] < threshold {
            let mut t = tau;
            while t + 1 <= c.max_lag && c.d[t + 1] < c.d[t] {
                t += 1;
            }
            return Some(t);
        }
    }
    None
}

/// Turn one CMNDF into weighted pitch candidates + the voiced probability, by
/// integrating [`yin_pick`] over the Beta threshold prior. Candidates merge by the
/// integer lag their picks land on (the same dip yields the same bottom lag), then
/// each lag is parabola-refined to a sub-sample period.
fn pyin_candidates(c: &Cmndf, prior: &ThresholdPrior, sample_rate: f32) -> (Vec<Candidate>, f32) {
    // (lag, accumulated prior mass); few distinct lags, so a linear scan beats a map.
    let mut lag_prob: Vec<(usize, f32)> = Vec::new();
    let mut voiced = 0.0f32;
    for k in 0..N_THRESHOLDS {
        let Some(tau) = yin_pick(c, prior.thresholds[k]) else {
            continue; // no dip below this threshold → contributes to unvoiced
        };
        voiced += prior.weights[k];
        match lag_prob.iter_mut().find(|(t, _)| *t == tau) {
            Some(entry) => entry.1 += prior.weights[k],
            None => lag_prob.push((tau, prior.weights[k])),
        }
    }
    let candidates = lag_prob
        .into_iter()
        .filter_map(|(tau, p)| {
            let refined = parabolic_tau(&c.d, tau);
            (refined > 0.0).then(|| Candidate {
                frequency_hz: sample_rate / refined,
                probability:  p,
            })
        })
        .collect();
    (candidates, voiced.clamp(0.0, 1.0))
}

// --- HMM stage ---------------------------------------------------------------

/// Pitch-grid resolution: 10 cents/bin (Mauch's default). Fine enough that the
/// octave decision is exact; the reported frequency comes from the candidate, not
/// the bin, so display cents stay sharp.
const BINS_PER_SEMITONE: usize = 10;
/// Tracked pitch span, C1..C7 — covers voice and the whole violin range with room.
const MIN_MIDI: f32 = 24.0;
const MAX_MIDI: f32 = 96.0;
/// Number of pitch states; index `N_PITCH` is the extra unvoiced state.
const N_PITCH: usize = ((MAX_MIDI - MIN_MIDI) as usize) * BINS_PER_SEMITONE;
/// Transition reaches ±1 octave; beyond that a move must go through unvoiced.
const TRANS_WINDOW: usize = 12 * BINS_PER_SEMITONE;
/// Probability the pitch *stays* in its bin frame to frame (given it stays
/// voiced). Must dominate so persistence is cheap — otherwise the (nearly free)
/// unvoiced self-loop out-races every voiced state and the tracker never commits.
/// The remaining `1 − SELF_STAY` is spread over neighbours by the kernel below.
const SELF_STAY: f32 = 0.8;
/// Gaussian width (cents) of the *neighbour* spread — how far a glide/vibrato can
/// move per frame within the non-self mass. An octave sits far in this tail, so a
/// one-frame octave jump is effectively impossible; real leaps route through the
/// unvoiced state instead (which reaches any pitch).
const TRANS_SIGMA_CENTS: f32 = 70.0;
/// Tiny floor on the neighbour spread so any within-window move has finite cost.
const TRANS_FLOOR: f32 = 1e-6;
/// Probability of leaving a pitch for the unvoiced state each frame (and vice
/// versa). Small: notes persist, but a real gap can switch voicing.
const VOICING_SWITCH: f32 = 0.02;
/// Emission floor so no state has log-prob −∞.
const EMIT_EPS: f32 = 1e-9;
/// Weight of the resonator bank's fast pitch when it is fused in as an extra HMM
/// candidate (scaled by the bank's strength). Comparable to a strong YIN
/// candidate so it pulls the tracker toward the low-latency estimate, but not so
/// large that a momentarily octave-wrong bank reading can overpower the HMM's
/// continuity (the transition cost of an octave jump still dominates).
const BANK_WEIGHT: f32 = 0.5;
/// Bank weight on an *onset* frame. A fresh attack means a new note, so the bank
/// (which sees it ~100 ms before YIN's long window catches up) is trusted to lead
/// even a large leap — paired with dropping the trellis bias in `process`. Off an
/// onset the smaller `BANK_WEIGHT` keeps octave glitches out; the onset signal is
/// what separates "new note" from "glitch".
const ATTACK_BANK_WEIGHT: f32 = 2.0;

fn freq_to_bin(frequency_hz: f32) -> f32 {
    let midi = 69.0 + 12.0 * (frequency_hz / 440.0).log2();
    (midi - MIN_MIDI) * BINS_PER_SEMITONE as f32
}

fn bin_to_freq(bin: usize) -> f32 {
    let midi = MIN_MIDI + bin as f32 / BINS_PER_SEMITONE as f32;
    440.0 * 2.0f32.powf((midi - 69.0) / 12.0)
}

/// Stateful probabilistic-YIN pitch tracker: one per audio stream. Feeds on
/// analysis windows and carries the HMM's Viterbi trellis frame to frame.
pub(crate) struct PitchTracker {
    prior:       ThresholdPrior,
    /// log-prob of the best path ending in each state (renormalized each frame);
    /// length `N_PITCH + 1`, last entry = unvoiced.
    delta:       Vec<f32>,
    /// log of the normalized pitch→pitch kernel, indexed `offset + TRANS_WINDOW`.
    log_kernel:  Vec<f32>,
    initialized: bool,
}

impl PitchTracker {
    pub(crate) fn new() -> Self {
        // Transition kernel over offsets −W..=W as a proper distribution summing to
        // 1, but *diagonal-dominant*: a fixed `SELF_STAY` on the diagonal (offset 0)
        // and the remaining `1 − SELF_STAY` shared among neighbours by a Gaussian.
        // Diagonal dominance is the whole trick — it makes "keep the same pitch"
        // cheap, so a voiced note can hold against the unvoiced self-loop, while the
        // octave (far in the Gaussian tail) stays prohibitively costly.
        let w = TRANS_WINDOW as isize;
        let mut kernel = vec![0.0f32; 2 * TRANS_WINDOW + 1];
        let mut neighbour_sum = 0.0f32;
        for off in -w..=w {
            if off == 0 {
                continue;
            }
            let cents = off as f32 * (100.0 / BINS_PER_SEMITONE as f32);
            let g = (-0.5 * (cents / TRANS_SIGMA_CENTS).powi(2)).exp() + TRANS_FLOOR;
            kernel[(off + w) as usize] = g;
            neighbour_sum += g;
        }
        for (i, k) in kernel.iter_mut().enumerate() {
            if i == TRANS_WINDOW {
                *k = SELF_STAY;
            } else {
                *k = (1.0 - SELF_STAY) * *k / neighbour_sum;
            }
        }
        let log_kernel = kernel.iter().map(|k| k.ln()).collect();

        Self {
            prior: ThresholdPrior::new(),
            delta: vec![0.0; N_PITCH + 1],
            log_kernel,
            initialized: false,
        }
    }

    /// Feed one analysis window; returns `(frequency_hz, voiced_probability)` when
    /// the decoded path is voiced this frame, else `None`.
    ///
    /// `bank` is the resonator bank's fast pitch `(frequency_hz, strength)` for this
    /// moment, or `None` when the bank isn't running / is quiet. When present it is
    /// fused in as one extra weighted candidate: the bank leads YIN's long window by
    /// ~100 ms, so this both speeds the tracker's onset response and lets the two
    /// sources agree on the octave inside the HMM (replacing the old panel-side
    /// octave-lock). Off an onset, a wrong-octave bank frame is out-voted by the
    /// HMM's continuity.
    ///
    /// `onset` marks a fresh attack (a new note starting). On such a frame the bank
    /// is trusted heavily (`ATTACK_BANK_WEIGHT`) and the trellis's continuity bias
    /// is dropped, so the new note — even a big leap — localizes to the fast
    /// evidence at once instead of being dragged toward the previous note or routed
    /// through the unvoiced state. This is what cuts note-start latency.
    pub(crate) fn process(
        &mut self,
        window: &[f32],
        sample_rate: f32,
        bank: Option<(f32, f32)>,
        onset: bool,
    ) -> Option<(f32, f32)> {
        let c = cmndf(window, sample_rate)?;
        let (mut candidates, mut voiced) = pyin_candidates(&c, &self.prior, sample_rate);
        if let Some((frequency_hz, strength)) = bank {
            if frequency_hz > 0.0 {
                let weight = if onset { ATTACK_BANK_WEIGHT } else { BANK_WEIGHT };
                let probability = weight * strength.clamp(0.0, 1.0);
                candidates.push(Candidate {
                    frequency_hz,
                    probability,
                });
                voiced = (voiced + probability).min(1.0);
            }
        }
        if onset {
            // Fresh attack: drop the trellis's continuity bias so the new note
            // localizes to the (attack-boosted) fast evidence this frame rather than
            // sticking to the previous note.
            self.initialized = false;
        }
        self.step(&candidates, voiced)
    }

    /// One HMM frame: build emissions from the candidates, advance the Viterbi
    /// trellis, and read off the current best state.
    fn step(&mut self, candidates: &[Candidate], voiced: f32) -> Option<(f32, f32)> {
        let uv = N_PITCH;

        // Emissions (linear). Each candidate deposits its mass around its bin
        // (centre + half to each neighbour); the unvoiced state gets the leftover.
        let mut emit = vec![EMIT_EPS; N_PITCH + 1];
        for cand in candidates {
            let bf = freq_to_bin(cand.frequency_hz);
            let center = bf.round() as isize;
            for off in -1..=1isize {
                let b = center + off;
                if b < 0 || b >= N_PITCH as isize {
                    continue;
                }
                let weight = if off == 0 { 1.0 } else { 0.5 };
                emit[b as usize] += cand.probability * weight;
            }
        }
        emit[uv] += 1.0 - voiced;
        let logb: Vec<f32> = emit.iter().map(|e| e.max(EMIT_EPS).ln()).collect();

        if !self.initialized {
            // Uniform prior over states → the first frame's path is just its
            // emission.
            self.delta.copy_from_slice(&logb);
            self.initialized = true;
        } else {
            let log_stay = (1.0 - VOICING_SWITCH).ln();
            let log_p2uv = VOICING_SWITCH.ln();
            let log_uv2uv = (1.0 - VOICING_SWITCH).ln();
            // Unvoiced re-enters pitch uniformly — this is the escape hatch that
            // lets a genuine octave leap (across a note gap) track.
            let log_uv2p = (VOICING_SWITCH / N_PITCH as f32).ln();

            let max_pitch_delta = self.delta[..N_PITCH]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);

            let mut next = vec![f32::NEG_INFINITY; N_PITCH + 1];
            for j in 0..N_PITCH {
                let lo = j.saturating_sub(TRANS_WINDOW);
                let hi = (j + TRANS_WINDOW).min(N_PITCH - 1);
                let mut best = f32::NEG_INFINITY;
                for i in lo..=hi {
                    let off = i as isize - j as isize + TRANS_WINDOW as isize;
                    let v = self.delta[i] + log_stay + self.log_kernel[off as usize];
                    if v > best {
                        best = v;
                    }
                }
                // …or arrive from unvoiced.
                let from_uv = self.delta[uv] + log_uv2p;
                if from_uv > best {
                    best = from_uv;
                }
                next[j] = best + logb[j];
            }
            // Unvoiced: stay unvoiced, or the best pitch decides to drop out.
            next[uv] = (self.delta[uv] + log_uv2uv).max(max_pitch_delta + log_p2uv) + logb[uv];

            self.delta = next;
        }

        // Renormalize (subtract the max) to keep the log-probs bounded over time.
        let m = self.delta.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if m.is_finite() {
            for d in &mut self.delta {
                *d -= m;
            }
        }

        // Greedy online decode: the current most-likely state.
        let best_state = self
            .delta
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        if best_state == uv {
            return None;
        }

        // Report the winning *candidate's* frequency (sub-cent) rather than the
        // 10-cent bin centre, so the tuner's cents stay sharp. Fall back to the bin
        // centre if no candidate sits near the decoded bin.
        let mut freq = bin_to_freq(best_state);
        let mut best_p = 0.0f32;
        for cand in candidates {
            let cb = freq_to_bin(cand.frequency_hz).round() as isize;
            if (cb - best_state as isize).abs() <= 2 && cand.probability > best_p {
                best_p = cand.probability;
                freq = cand.frequency_hz;
            }
        }
        Some((freq, voiced))
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    fn tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| (TAU * frequency_hz * i as f32 / sample_rate).sin())
            .collect()
    }

    /// Candidate extraction on a clean tone: the dominant hypothesis is the tone.
    #[test]
    fn candidates_find_the_tone() {
        let sr = 44_100.0;
        let prior = ThresholdPrior::new();
        let c = cmndf(&tone(440.0, sr, 6144), sr).unwrap();
        let (cands, voiced) = pyin_candidates(&c, &prior, sr);
        assert!(voiced > 0.5, "a clean tone should read voiced, got {voiced}");
        let top = cands
            .iter()
            .max_by(|a, b| a.probability.total_cmp(&b.probability))
            .unwrap();
        assert!(
            (top.frequency_hz - 440.0).abs() < 3.0,
            "dominant candidate {} should be ~440",
            top.frequency_hz
        );
    }

    /// The Beta weights integrate to 1 (a proper prior).
    #[test]
    fn prior_weights_sum_to_one() {
        let prior = ThresholdPrior::new();
        let sum: f32 = prior.weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4, "weights sum {sum}");
    }

    /// The HMM's whole point: a single octave-outlier frame in the middle of a
    /// steady note must NOT move the decoded pitch. Feed synthetic candidates
    /// directly to isolate the smoothing from the front end.
    #[test]
    fn hmm_rejects_a_lone_octave_outlier() {
        let mut t = PitchTracker::new();
        let a4 = [Candidate {
            frequency_hz: 440.0,
            probability:  0.9,
        }];
        // A realistic octave-error frame: the sub-octave momentarily wins the YIN
        // vote, but the true fundamental is still present as a weaker candidate
        // (its CMNDF dip never actually vanishes) — exactly what pYIN emits and the
        // HMM is meant to disambiguate by continuity.
        let glitch = [
            Candidate {
                frequency_hz: 220.0,
                probability:  0.6,
            },
            Candidate {
                frequency_hz: 440.0,
                probability:  0.3,
            },
        ];
        for _ in 0..8 {
            t.step(&a4, 0.9);
        }
        let out = t.step(&glitch, 0.9).unwrap();
        assert!(
            (out.0 - 440.0).abs() < 20.0,
            "octave outlier moved the pitch to {}",
            out.0
        );
    }

    /// …but a *sustained* octave change is tracked (via the unvoiced escape hatch
    /// and accumulated emission), not frozen forever.
    #[test]
    fn hmm_follows_a_sustained_octave_change() {
        let mut t = PitchTracker::new();
        let a4 = [Candidate {
            frequency_hz: 440.0,
            probability:  0.9,
        }];
        let a5 = [Candidate {
            frequency_hz: 880.0,
            probability:  0.9,
        }];
        for _ in 0..8 {
            t.step(&a4, 0.9);
        }
        let mut last = 440.0;
        for _ in 0..40 {
            if let Some((f, _)) = t.step(&a5, 0.9) {
                last = f;
            }
        }
        assert!(
            (last - 880.0).abs() < 20.0,
            "sustained octave change did not track, ended at {last}"
        );
    }

    /// No candidates + zero voiced probability decodes as unvoiced (silence).
    #[test]
    fn hmm_reports_unvoiced_on_silence() {
        let mut t = PitchTracker::new();
        for _ in 0..4 {
            t.step(&[], 0.9); // prime with something voiced-ish first
        }
        for _ in 0..8 {
            // strong unvoiced evidence, no pitch candidates
        }
        assert!(t.step(&[], 0.0).is_none(), "empty + unvoiced should be None");
    }

    /// End-to-end on real windows: a clean tone decodes to itself.
    #[test]
    fn process_tracks_a_clean_tone() {
        let sr = 44_100.0;
        let mut t = PitchTracker::new();
        let win = tone(330.0, sr, 6144); // ~E4
        let mut last = None;
        for _ in 0..6 {
            last = t.process(&win, sr, None, false);
        }
        let (f, _) = last.expect("clean tone should be voiced");
        assert!((f - 330.0).abs() < 3.0, "tracked {f}, expected ~330");
    }

    /// A fused bank candidate that agrees with the window keeps the tracker on pitch
    /// (and, being a strong low-latency vote, locks it promptly).
    #[test]
    fn process_with_agreeing_bank_tracks() {
        let sr = 44_100.0;
        let mut t = PitchTracker::new();
        let win = tone(440.0, sr, 6144);
        let mut last = None;
        for _ in 0..6 {
            last = t.process(&win, sr, Some((440.0, 0.9)), false);
        }
        let (f, _) = last.expect("voiced");
        assert!((f - 440.0).abs() < 3.0, "tracked {f}, expected ~440");
    }

    /// Off an onset, a momentarily octave-wrong bank reading must NOT drag the
    /// tracker off — the HMM's continuity out-votes the fused candidate. This is the
    /// fusion's safety net (the bank is octave-ambiguous on a bowed string).
    #[test]
    fn process_rejects_wrong_octave_bank() {
        let sr = 44_100.0;
        let mut t = PitchTracker::new();
        let win = tone(440.0, sr, 6144);
        for _ in 0..8 {
            t.process(&win, sr, None, false);
        }
        let out = t.process(&win, sr, Some((220.0, 0.9)), false).unwrap();
        assert!(
            (out.0 - 440.0).abs() < 20.0,
            "wrong-octave bank pulled pitch to {}",
            out.0
        );
    }

    /// On an *onset*, the attack-boosted bank leads a leap immediately: with the
    /// window still on the old note (A4) but the bank reporting the new note (A5)
    /// and the onset flag set, the tracker jumps this frame — the note-start latency
    /// win. (Contrast `process_rejects_wrong_octave_bank`: same bank/window, but no
    /// onset → continuity holds. The onset flag is what separates the two.)
    #[test]
    fn onset_lets_bank_lead_a_leap() {
        let sr = 44_100.0;
        let mut t = PitchTracker::new();
        let win = tone(440.0, sr, 6144); // window (and thus YIN) still says A4
        for _ in 0..8 {
            t.process(&win, sr, Some((440.0, 0.9)), false);
        }
        // Attack: bank already hears A5 while the window lags on A4.
        let out = t.process(&win, sr, Some((880.0, 0.9)), true).unwrap();
        assert!(
            (out.0 - 880.0).abs() < 20.0,
            "onset should let the bank lead the leap, got {}",
            out.0
        );
    }
}
