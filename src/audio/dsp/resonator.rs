use resonators::{
    OnePoleBank,
    ResonatorConfig,
    heuristic_alpha,
    midi_to_hz,
};

use super::analysis_math::{
    NOTE_BUCKET_MAX_MIDI,
    NOTE_BUCKET_MIN_MIDI,
    SPIRAL_BINS_PER_SEMITONE,
    normalize_bars,
    resonator_note_labels,
    splat_linear,
};
use super::swipe::{
    SalienceFrame,
    SwipeKernel,
};
use crate::audio::types::AnalysisSettings;
use crate::core_types::note::AccidentalStyle;

const RESONATOR_MIN_MIDI: usize = NOTE_BUCKET_MIN_MIDI;
const RESONATOR_MAX_MIDI: usize = NOTE_BUCKET_MAX_MIDI;
const RESONATOR_DEFAULT_BINS_PER_SEMITONE: usize = 5;

// --- Instantaneous-frequency reassignment (super-resolution + image suppression) ---
//
// Each resonator's stored value `rr` (the EWMA of the heterodyned input) rotates
// at exactly +2π·(f_in − f_bin): its phase carries the *detuning* of the signal
// partial from the bin's tuning. Reading the phase twice, a known interval apart,
// and dividing the (wrapped) phase change by that interval recovers f_in − f_bin
// directly — instantaneous frequency without an FFT. We then splat the bin's
// magnitude at its *reassigned* frequency f_bin + detuning, so a slightly sharp
// note lands slightly sharp on the spiral instead of being quantised to the
// nearest bin centre (super-resolution).
//
// PHASE_WINDOW bounds the measurement interval so the wrapped phase change stays
// unambiguous (|Δφ| < π). At 128 samples / 44.1 kHz the no-alias ceiling is
// sr/(2·128) ≈ 172 Hz of detuning, far above the ±0.5-semitone band we trust
// (±122 Hz even at the top of the configurable range, 4186 Hz).
const PHASE_WINDOW: usize = 128;
// EWMA on the per-bin detuning estimate: smooths frame-to-frame jitter (and the
// residual wobble from the negative-frequency image of a real signal) while
// still tracking glides.
const DETUNING_SMOOTH: f32 = 0.3;
// Coherence gate. A bin legitimately tracking a nearby partial reassigns by only
// a fraction of a semitone (bins sit 1/bins_per_semitone apart). Energy whose
// phase points far from the bin's tuning is the negative-frequency image,
// neighbour leakage, or broadband noise — its reassignment is large and erratic,
// so a Gaussian falloff (σ) plus a hard cutoff suppresses it.
const GATE_SIGMA_SEMITONES: f32 = 0.5;
const GATE_MAX_SEMITONES: f32 = 2.0;
// Output spiral resolution. The bank stays at the (cheap) user `bins`/semitone;
// reassignment places its energy onto this finer display grid.
const OUTPUT_BINS_PER_SEMITONE: usize = SPIRAL_BINS_PER_SEMITONE;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResonatorViewSettings {
    min_midi:          usize,
    max_midi:          usize,
    bins_per_semitone: usize,
    alpha_scale:       f32,
    beta_scale:        f32,
    gamma:             f32,
    power:             bool,
    // Эталон A4: меняется камертон → пересобираем банк (PartialEq ловит сдвиг).
    reference_hz:      f32,
}

#[derive(Clone, Debug)]
pub(crate) struct ResonatorSnapshot {
    pub(crate) spectrum:      Vec<f32>,
    pub(crate) note_labels:   Vec<String>,
    /// Fast played-note prior for this snapshot: `(fractional_midi, strength)` of
    /// the harmonic fundamental, or `None` when the bank is quiet. Rides to the UI
    /// on `TunerReading::fast_pitch`.
    ///
    /// This is the frame's **own** opinion, weighing nothing but itself. It stays that way
    /// on purpose even though [`Self::salience`] now supports a better one: `dsp::melody`
    /// borrows pYIN's octave to *check* the bank, and a bank reading that had already been
    /// smoothed against its own past would be a worse witness, not a better one.
    pub(crate) fundamental:   Option<(f32, f32)>,
    /// The evidence behind [`Self::fundamental`] — the whole salience curve, for the
    /// consumer that decodes a *path* rather than a frame. `None` for a column with no
    /// energy at all.
    ///
    /// Deliberately not stored in `SharedState`: it is one frame's working material for
    /// `dsp::melody` — the `magnitudes` it carries for [`SalienceFrame::refine_on_partials`]
    /// double its weight, and none of that is a panel's business. What a panel may draw is
    /// [`Self::salience_heat`], which is the curve alone.
    pub(crate) salience:      Option<SalienceFrame>,
    /// [`Self::salience`] as a **display layer**: the curve alone, zeroed outside the
    /// tracked domain and normalized exactly like [`Self::spectrum`], so the pitch roll can
    /// swap one for the other and be comparing like with like.
    ///
    /// Separate from `salience` rather than derived at the panel because the normalization
    /// is `dsp`'s (`normalize_bars`, and the same `gamma` the column it replaces gets) and
    /// `audio::dsp` is private — a panel re-deriving it would be that contrast rule
    /// implemented twice and drifting.
    pub(crate) salience_heat: Option<Vec<f32>>,
}

/// [`ResonatorSnapshot::salience_heat`] from a scored frame: zero outside the domain, *then*
/// normalize — see [`SalienceFrame::curve_over_tracked`] for why that order is not a detail.
fn salience_heat(salience: Option<&SalienceFrame>, gamma: f32) -> Option<Vec<f32>> {
    salience.map(|frame| {
        let mut curve = frame.curve_over_tracked();
        normalize_bars(&mut curve, gamma);
        curve
    })
}

#[derive(Debug)]
pub(crate) struct ResonatorAnalyzer {
    settings:    ResonatorViewSettings,
    sample_rate: f32,
    bank:        OnePoleBank,

    // Instantaneous-frequency tracking state, one slot per bank bin.
    // `prev_phase` holds the phase at the last measurement; `detuning_hz` is the
    // smoothed (f_in − f_bin) estimate; `pending` counts samples fed since the
    // last measurement; `have_phase` gates the first (no-baseline) interval.
    prev_phase:  Vec<f32>,
    detuning_hz: Vec<f32>,
    pending:     usize,
    have_phase:  bool,

    // SWIPE′ kernels — fixed vectors, so they are built once per grid rather than per
    // frame (see `dsp::swipe`). Two, because the two snapshot paths score on two
    // different grids: the reassigned path on the fixed `OUTPUT_BINS_PER_SEMITONE`, the
    // rollback path on the bank's own user-adjustable resolution.
    swipe_output: SwipeKernel,
    swipe_bank:   SwipeKernel,
}

impl ResonatorViewSettings {
    pub(crate) fn note_labels(&self, style: AccidentalStyle) -> Vec<String> {
        resonator_note_labels(self.min_midi, self.max_midi, style)
    }
}

impl Default for ResonatorViewSettings {
    fn default() -> Self {
        Self {
            min_midi:          RESONATOR_MIN_MIDI,
            max_midi:          RESONATOR_MAX_MIDI,
            bins_per_semitone: RESONATOR_DEFAULT_BINS_PER_SEMITONE,
            alpha_scale:       1.0,
            beta_scale:        1.0,
            gamma:             0.72,
            power:             false,
            reference_hz:      440.0,
        }
    }
}

impl From<&AnalysisSettings> for ResonatorViewSettings {
    fn from(s: &AnalysisSettings) -> Self {
        Self {
            // Leave the typed-MIDI config behind here: the view-model uses these
            // purely as bin-bucket offsets/iteration bounds (raw `usize` domain).
            min_midi:          s.resonator.min_midi.as_u8() as usize,
            max_midi:          s.resonator.max_midi.as_u8() as usize,
            bins_per_semitone: s.resonator.bins,
            alpha_scale:       s.resonator.alpha,
            beta_scale:        s.resonator.beta,
            gamma:             s.resonator.gamma,
            power:             s.resonator.power,
            reference_hz:      s.concert_pitch_hz,
        }
    }
}

impl ResonatorAnalyzer {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let settings = ResonatorViewSettings::default();
        let settings_bins = settings.bins_per_semitone;
        let bank = build_resonator_bank(sample_rate, &settings);
        let n = bank.len();
        Self {
            settings,
            sample_rate,
            bank,
            prev_phase: vec![0.0; n],
            detuning_hz: vec![0.0; n],
            pending: 0,
            have_phase: false,
            swipe_output: SwipeKernel::new(OUTPUT_BINS_PER_SEMITONE as f32),
            swipe_bank: SwipeKernel::new(settings_bins as f32),
        }
    }

    pub(crate) fn sync_settings(&mut self, requested: ResonatorViewSettings) -> bool {
        if requested == self.settings {
            return false;
        }
        // The bank's resolution is user-adjustable, and the kernel is cut to a grid.
        if requested.bins_per_semitone != self.settings.bins_per_semitone {
            self.swipe_bank = SwipeKernel::new(requested.bins_per_semitone as f32);
        }
        self.settings = requested;
        self.bank = build_resonator_bank(self.sample_rate, &self.settings);
        // The bin set changed → the old phase/detuning slots no longer map to
        // anything. Resize and restart tracking from a clean baseline.
        let n = self.bank.len();
        self.prev_phase = vec![0.0; n];
        self.detuning_hz = vec![0.0; n];
        self.pending = 0;
        self.have_phase = false;
        true
    }

    /// Feed audio into the bank. When `reassign` is on, the buffer is sliced so
    /// the per-bin phase is sampled at a fixed `PHASE_WINDOW` cadence regardless
    /// of the host's callback chunk size — a fixed, bounded interval is what keeps
    /// the wrapped phase difference unambiguous (see `PHASE_WINDOW`).
    ///
    /// When `reassign` is off the snapshot reads only the bank's magnitudes, so
    /// the entire instantaneous-frequency measurement is dead weight: we feed the
    /// bank in one shot and skip it. The tracking state is reset so re-enabling
    /// reassignment restarts from a clean phase baseline rather than differencing
    /// across the gap where measurement was suspended.
    pub(crate) fn process_samples(&mut self, samples: &[f32], reassign: bool) {
        if !reassign {
            self.bank.process_samples(samples);
            self.pending = 0;
            self.have_phase = false;
            return;
        }
        let mut offset = 0;
        while offset < samples.len() {
            let take = (PHASE_WINDOW - self.pending).min(samples.len() - offset);
            self.bank.process_samples(&samples[offset..offset + take]);
            self.pending += take;
            offset += take;
            if self.pending >= PHASE_WINDOW {
                self.measure_detuning(self.pending);
                self.pending = 0;
            }
        }
    }

    /// Update each bin's detuning estimate from the phase advanced over the last
    /// `dn` samples. `rr`'s phase rotates at +2π·(f_in − f_bin), so the wrapped
    /// phase change over Δt = dn/sr divided by 2π·Δt is the detuning in Hz.
    fn measure_detuning(&mut self, dn: usize) {
        use std::f32::consts::{
            PI,
            TAU,
        };
        let dt = dn as f32 / self.sample_rate;
        let two_pi_dt = TAU * dt;
        // Disjoint-field borrow: `prev`/`det` mutate the tracking vecs while
        // `self.bank.phase(i)` reads a different field — `i` is still needed to
        // address the bank, so we enumerate rather than range-loop.
        let have_phase = self.have_phase;
        for (i, (prev, det)) in self
            .prev_phase
            .iter_mut()
            .zip(self.detuning_hz.iter_mut())
            .enumerate()
        {
            let phase = self.bank.phase(i);
            if have_phase {
                // Wrap the difference into (−π, π]; an unwrapped jump would alias
                // into a bogus detuning. rem_euclid keeps it branchless.
                let delta = (phase - *prev + PI).rem_euclid(TAU) - PI;
                let detuning = delta / two_pi_dt;
                *det += DETUNING_SMOOTH * (detuning - *det);
            }
            *prev = phase;
        }
        self.have_phase = true;
    }

    pub(crate) fn snapshot(&self, reassign: bool, style: AccidentalStyle) -> ResonatorSnapshot {
        resonator_snapshot(
            &self.bank,
            &self.settings,
            &self.detuning_hz,
            reassign,
            style,
            &self.swipe_bank,
            &self.swipe_output,
        )
    }

    pub(crate) fn note_labels(&self, style: AccidentalStyle) -> Vec<String> {
        self.settings.note_labels(style)
    }
}

fn build_resonator_bank(sample_rate: f32, settings: &ResonatorViewSettings) -> OnePoleBank {
    let bin_count = (settings.max_midi - settings.min_midi) * settings.bins_per_semitone + 1;
    let configs: Vec<ResonatorConfig> = (0..bin_count)
        .map(|i| {
            let midi = settings.min_midi as f32 + i as f32 / settings.bins_per_semitone as f32;
            let frequency = midi_to_hz(midi, settings.reference_hz);
            // Floor guards only alpha > 0 (a zero coefficient is a dead resonator);
            // it must sit well below base_alpha * min(slider). The old floor of 1e-4
            // silently swallowed the bottom of the slider range for bass bins: at
            // C0 (~16 Hz) base alpha ≈ 3e-4, so any scale below ~0.33 was clamped
            // flat. At 1e-6 the slider stays effective down to scale 0.001 for
            // every bin above ~90 Hz (tau ≈ 23 s at the floor — extreme but stable:
            // 1 − alpha < 1 keeps the pole inside the unit circle).
            let alpha = (heuristic_alpha(frequency, sample_rate) * settings.alpha_scale).clamp(1e-6, 1.0);
            let beta = (heuristic_alpha(frequency, sample_rate) * settings.beta_scale).clamp(1e-6, 1.0);
            ResonatorConfig::new(frequency, alpha, beta)
        })
        .collect();
    OnePoleBank::new(&configs, sample_rate)
}

/// Build the display spiral by reassigning each bin's energy to its measured
/// instantaneous frequency.
///
/// Rather than reporting one magnitude per resonator at its nominal pitch (which
/// quantises every partial to a bin centre), we splat each bin's magnitude at
/// `f_bin + detuning` — its true frequency, recovered from the phase. This gives
/// sub-bin placement (super-resolution) on the spiral, and the coherence gate
/// turns the *consistency* of that reassignment into noise/image suppression:
/// only energy whose phase points at the bin's own tuning survives.
///
/// The bank runs at the user's `bins`/semitone; the output grid is the finer
/// `OUTPUT_BINS_PER_SEMITONE`, spanning the bank's own MIDI range so it stays
/// aligned with the (one-per-semitone) note labels.
fn resonator_snapshot(
    bank: &OnePoleBank,
    settings: &ResonatorViewSettings,
    detuning_hz: &[f32],
    reassign: bool,
    style: AccidentalStyle,
    swipe_bank: &SwipeKernel,
    swipe_output: &SwipeKernel,
) -> ResonatorSnapshot {
    // Fallback (safety net): plain per-bin magnitude at the bin's nominal pitch,
    // at the bank's own resolution. This is the original, pre-reassignment path —
    // bit-for-bit what shipped before, so toggling it off is a clean rollback.
    if !reassign {
        let mut spectrum = if settings.power {
            bank.powers()
        } else {
            bank.magnitudes()
        };
        // Score BEFORE `normalize_bars`: `gamma` and `power` are display controls (a
        // waterfall-contrast slider, `controls.rs`), and a display knob must not steer
        // the octave decision — which it did, for as long as this line came second.
        // SWIPE does its own warping (√), and it is not negotiable by the UI.
        let salience = SalienceFrame::score(
            &spectrum,
            settings.min_midi as f32,
            settings.bins_per_semitone as f32,
            swipe_bank,
        );
        let fundamental = salience.as_ref().and_then(|s| s.argmax());
        normalize_bars(&mut spectrum, settings.gamma);
        return ResonatorSnapshot {
            spectrum,
            note_labels: settings.note_labels(style),
            fundamental,
            salience_heat: salience_heat(salience.as_ref(), settings.gamma),
            salience,
        };
    }

    let semitone_span = settings.max_midi - settings.min_midi;
    let out_len = semitone_span * OUTPUT_BINS_PER_SEMITONE + 1;
    let mut spectrum = vec![0.0f32; out_len];

    for (i, &detuning) in detuning_hz.iter().enumerate() {
        let weight = if settings.power {
            bank.power(i)
        } else {
            bank.magnitude(i)
        };
        if weight <= 0.0 {
            continue;
        }
        let f_bin = bank.freq(i);
        let f_hat = f_bin + detuning;
        // A reassignment that flips sign / lands below DC is meaningless — drop it.
        if f_hat <= 0.0 {
            continue;
        }

        // Reassignment distance in semitones drives the coherence gate.
        let ds = 12.0 * (f_hat / f_bin).log2();
        if ds.abs() > GATE_MAX_SEMITONES {
            continue;
        }
        let gate = (-0.5 * (ds / GATE_SIGMA_SEMITONES).powi(2)).exp();

        let midi = 69.0 + 12.0 * (f_hat / settings.reference_hz).log2();
        let position = (midi - settings.min_midi as f32) * OUTPUT_BINS_PER_SEMITONE as f32;
        splat_linear(&mut spectrum, position, weight * gate);
    }

    // Same rule as the rollback path above: the detector reads the bank's own column,
    // the display gets its `gamma` afterwards.
    let salience = SalienceFrame::score(
        &spectrum,
        settings.min_midi as f32,
        OUTPUT_BINS_PER_SEMITONE as f32,
        swipe_output,
    );
    let fundamental = salience.as_ref().and_then(|s| s.argmax());
    normalize_bars(&mut spectrum, settings.gamma);
    ResonatorSnapshot {
        spectrum,
        note_labels: settings.note_labels(style),
        fundamental,
        salience_heat: salience_heat(salience.as_ref(), settings.gamma),
        salience,
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    /// MIDI of an output-grid bin index, given the analyzer's range/resolution.
    fn bin_midi(an: &ResonatorAnalyzer, idx: usize) -> f32 {
        an.settings.min_midi as f32 + idx as f32 / OUTPUT_BINS_PER_SEMITONE as f32
    }

    fn peak_index(spectrum: &[f32]) -> usize {
        spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap()
    }

    /// REGRESSION: the bank must follow a note change inside the perceptual
    /// threshold for pitch feedback (~30–40 ms, see `docs/pitch_detection_survey.md`).
    ///
    /// This is the property the whole melody path is built on: the bank is the fast
    /// source, pYIN only pins its octave (`dsp::melody`). pYIN on the shared 6144
    /// window is pinned at 128 ms — if the bank ever stops being dramatically faster,
    /// riding it buys nothing and the design should be revisited rather than kept out
    /// of habit.
    #[test]
    fn bank_latency_probe() {
        fn violin_tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
            let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
            (0..len)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    partials
                        .iter()
                        .enumerate()
                        .map(|(h, a)| a * (TAU * frequency_hz * (h + 1) as f32 * t).sin())
                        .sum::<f32>()
                        / 3.0
                })
                .collect()
        }

        fn probe(from_hz: f32, to_hz: f32) -> f32 {
            let sr = 48_000.0f32;
            let hold = (sr * 0.6) as usize;
            let mut an = ResonatorAnalyzer::new(sr);
            let target_midi = 69.0 + 12.0 * (to_hz / 440.0).log2();

            // Prime on the old note, then feed the new one in audio-callback-sized
            // chunks, sampling the fundamental at the bank's own 16 ms publish rate.
            an.process_samples(&violin_tone(from_hz, sr, hold), true);
            let new = violin_tone(to_hz, sr, hold);
            let chunk = 128usize;
            let mut fed = 0usize;
            while fed < new.len() {
                let take = chunk.min(new.len() - fed);
                an.process_samples(&new[fed..fed + take], true);
                fed += take;
                let snap = an.snapshot(true, AccidentalStyle::Sharps);
                if let Some((midi, _strength)) = snap.fundamental {
                    if (midi - target_midi).abs() < 0.5 {
                        return fed as f32 / sr * 1000.0;
                    }
                }
            }
            f32::INFINITY
        }

        // Comfortably under pYIN's 128 ms, with headroom over the ~40 ms perceptual
        // threshold so this fails on a real regression, not on measurement noise.
        const BUDGET_MS: f32 = 40.0;
        // Two cases pay more, and Phase 1.11 (`dsp::swipe`) is why. SWIPE′ asks whether a
        // harmonic series *explains* the column, valleys included — so while the old note
        // rings on in the bank, its partials sit in the new candidate's valleys and vote
        // against it. They are not wrong: it really is still sounding. That is the same
        // mechanism that took the phantom octave from 57% of frames to 0% on a real bowed
        // G, so this is the price of the fix rather than a defect in it.
        //
        // The two costs are independent, which is why there are two numbers:
        //   * GEOMETRY — the octave is the worst interval, because every partial of A4
        //     feeds A5's valleys. 56 ms measured, against 13 ms for a fifth from the very
        //     same note.
        //   * REGISTER — A4->E5 and G3->D4 are the same fifth (identical kernel geometry)
        //     yet measure 13 ms and 80 ms. The bank is constant-Q: its ring-down is a
        //     roughly fixed number of *cycles*, so it lasts longer in seconds down low.
        //
        // Both are knowingly past the ~40 ms perceptual threshold — the accepted trade,
        // recorded in the plan (Phase 1.11). These are budgets, not targets: shrinking
        // them means the bank's ring-down (a time/frequency trade in `heuristic_alpha`),
        // never a weaker kernel.
        const OCTAVE_BUDGET_MS: f32 = 75.0;
        const LOW_REGISTER_BUDGET_MS: f32 = 100.0;
        println!("\n=== resonator bank note-change latency (pYIN = 128 ms) ===");
        let cases = [
            ("A4->B4 (2nd)  ", 440.0f32, 493.88f32, BUDGET_MS),
            ("A4->E5 (5th)  ", 440.0, 659.25, BUDGET_MS),
            ("A4->A5 (8ve)  ", 440.0, 880.0, OCTAVE_BUDGET_MS),
            ("E5->A4 (down) ", 659.25, 440.0, BUDGET_MS),
            ("G3->D4 (violin G->D)", 196.0, 293.66, LOW_REGISTER_BUDGET_MS),
        ];
        // Measure every case before asserting any: a probe that dies on the first
        // failure hides the shape of a regression behind its first symptom, and the
        // shape is the diagnosis.
        let measured: Vec<(&str, f32, f32)> = cases
            .iter()
            .map(|&(name, from, to, budget)| (name, probe(from, to), budget))
            .collect();
        for (name, ms, budget) in &measured {
            println!("{name} -> {ms:>7.1} ms  (budget {budget:.0})");
        }
        for (name, ms, budget) in &measured {
            assert!(
                ms <= budget,
                "{name}: bank took {ms:.1} ms, budget {budget:.0} ms — the melody \
                 line rides the bank precisely because it is this fast"
            );
        }
    }

    /// A tone parked *between* bank bins must land at its true frequency, closer
    /// than the nearest physical resonator could place it. Bank bins sit 0.2
    /// semitone apart (5/semitone), so a tone at +0.1 semitone is 0.1 from the
    /// nearest bin centre; reassignment should beat that comfortably.
    #[test]
    fn reassignment_lands_between_bins() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        let target_midi = 69.1; // between bank bins at 69.0 and 69.2
        let f = 440.0 * 2.0_f32.powf((target_midi - 69.0) / 12.0);
        let sig: Vec<f32> = (0..sr as usize)
            .map(|i| (TAU * f * i as f32 / sr).sin())
            .collect();
        an.process_samples(&sig, true);

        let snap = an.snapshot(true, AccidentalStyle::Sharps);
        let peak_midi = bin_midi(&an, peak_index(&snap.spectrum));
        assert!(
            (peak_midi - target_midi).abs() < 0.06,
            "peak {peak_midi} should reassign to ~{target_midi} (better than 0.1 nominal)"
        );
    }

    /// The measured detuning recovers the sign and rough magnitude of the offset
    /// for the bin nearest the tone (sharp → positive Hz).
    #[test]
    fn detuning_recovers_sharp_offset() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        let f = 440.0 * 2.0_f32.powf(0.1 / 12.0); // +0.1 semitone, sharp
        let sig: Vec<f32> = (0..sr as usize)
            .map(|i| (TAU * f * i as f32 / sr).sin())
            .collect();
        an.process_samples(&sig, true);

        // bank bin index nearest A4 (440): (69 - min_midi) * bins_per_semitone
        let a4_bin = (69 - an.settings.min_midi) * an.settings.bins_per_semitone;
        let det = an.detuning_hz[a4_bin];
        let expected = f - 440.0; // ~2.5 Hz sharp
        assert!(det > 0.0, "sharp tone should give positive detuning, got {det}");
        assert!(
            (det - expected).abs() < 0.5,
            "detuning {det} should be near {expected} Hz"
        );
    }

    /// End-to-end through the production path (analyzer → snapshot → fundamental):
    /// a harmonic-rich tone like a bowed string must resolve to its *fundamental*
    /// (A4 = 69), not an overtone, and land in-range. This is what feeds the staff's
    /// `TunerReading::fast_pitch`.
    #[test]
    fn fundamental_on_harmonic_tone_resolves_to_root() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        let f0 = 440.0; // A4 = MIDI 69
        // Fundamental + 2nd + 3rd + 4th partials, fundamental strongest.
        let sig: Vec<f32> = (0..sr as usize)
            .map(|i| {
                let t = i as f32 / sr;
                (TAU * f0 * t).sin()
                    + 0.6 * (TAU * 2.0 * f0 * t).sin()
                    + 0.4 * (TAU * 3.0 * f0 * t).sin()
                    + 0.3 * (TAU * 4.0 * f0 * t).sin()
            })
            .collect();
        an.process_samples(&sig, true);
        let snap = an.snapshot(true, AccidentalStyle::Sharps);
        let (midi, strength) = snap.fundamental.expect("fundamental should be detected");
        assert!((midi - 69.0).abs() < 0.2, "expected ~A4 (69), got {midi}");
        // `strength` changed meaning with `dsp::swipe`: it used to be the fundamental
        // bin's own magnitude against `FUNDAMENTAL_FLOOR` — a quantity that reads ~0 for
        // a perfectly clear open G, which is the defect SWIPE′ exists to fix. It is now
        // SWIPE's pitch strength: how well a harmonic series at this f0 explains the
        // whole spectrum.
        //
        // Its *absolute* scale does not carry over from the paper, and no threshold is
        // asserted here on purpose. SWIPE normalizes by the norm of the entire spectrum;
        // Camacho's is a narrow FFT band, ours is 480 resonators spanning C0..C8 whose
        // skirts spread energy across far more bins, so the same tone scores lower here
        // than the paper's 0..1 intuition suggests (measured: ~0.15 for this tone). Any
        // fixed number here would be invented rather than derived.
        //
        // What *is* meaningful is the comparison, so assert that instead: a harmonic tone
        // must out-score noise through the same bank, which is the only claim the quantity
        // actually supports.
        let mut noisy = ResonatorAnalyzer::new(sr);
        // Deterministic pseudo-noise — no rand dependency, and a fixed sequence keeps the
        // test reproducible.
        let noise: Vec<f32> = (0..sr as usize)
            .map(|i| ((i as f32 * 12.9898).sin() * 43758.547).fract() * 2.0 - 1.0)
            .collect();
        noisy.process_samples(&noise, true);
        let noise_strength = noisy
            .snapshot(true, AccidentalStyle::Sharps)
            .fundamental
            .map(|(_, s)| s)
            .unwrap_or(0.0);
        assert!(
            strength > noise_strength,
            "a harmonic tone ({strength}) must out-score noise ({noise_strength})"
        );
    }

    /// Empty / silent input must not panic and yields an all-zero spiral.
    #[test]
    fn silence_is_quiet() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        an.process_samples(&vec![0.0; 4096], true);
        let snap = an.snapshot(true, AccidentalStyle::Sharps);
        assert!(snap.spectrum.iter().all(|&v| v == 0.0));
    }

    /// The fallback (`reassign = false`) returns the bank's own resolution and
    /// places A4 at its nominal bin, while the reassigned path uses the finer
    /// output grid. Both must light up A4.
    #[test]
    fn fallback_path_uses_bank_resolution() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        let sig: Vec<f32> = (0..sr as usize)
            .map(|i| (TAU * 440.0 * i as f32 / sr).sin())
            .collect();
        an.process_samples(&sig, true);

        let span = RESONATOR_MAX_MIDI - RESONATOR_MIN_MIDI;
        let nominal = an.snapshot(false, AccidentalStyle::Sharps);
        let reassigned = an.snapshot(true, AccidentalStyle::Sharps);
        assert_eq!(
            nominal.spectrum.len(),
            span * RESONATOR_DEFAULT_BINS_PER_SEMITONE + 1
        );
        assert_eq!(reassigned.spectrum.len(), span * OUTPUT_BINS_PER_SEMITONE + 1);

        // nominal A4 peak sits on the bank grid (5/semitone) at midi 69.
        let nom_peak = RESONATOR_MIN_MIDI as f32
            + peak_index(&nominal.spectrum) as f32 / RESONATOR_DEFAULT_BINS_PER_SEMITONE as f32;
        assert!(
            (nom_peak - 69.0).abs() < 0.21,
            "nominal peak {nom_peak} should be ~A4"
        );
    }
}
