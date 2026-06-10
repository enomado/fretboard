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
use crate::audio::types::AnalysisSettings;

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
pub(super) struct ResonatorViewSettings {
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
pub(super) struct ResonatorSnapshot {
    pub(super) spectrum:    Vec<f32>,
    pub(super) note_labels: Vec<String>,
}

#[derive(Debug)]
pub(super) struct ResonatorAnalyzer {
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
}

impl ResonatorViewSettings {
    pub(super) fn note_labels(&self) -> Vec<String> {
        resonator_note_labels(self.min_midi, self.max_midi)
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
    pub(super) fn new(sample_rate: f32) -> Self {
        let settings = ResonatorViewSettings::default();
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
        }
    }

    pub(super) fn sync_settings(&mut self, requested: ResonatorViewSettings) -> bool {
        if requested == self.settings {
            return false;
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
    pub(super) fn process_samples(&mut self, samples: &[f32], reassign: bool) {
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

    pub(super) fn snapshot(&self, reassign: bool) -> ResonatorSnapshot {
        resonator_snapshot(&self.bank, &self.settings, &self.detuning_hz, reassign)
    }

    pub(super) fn note_labels(&self) -> Vec<String> {
        self.settings.note_labels()
    }
}

fn build_resonator_bank(sample_rate: f32, settings: &ResonatorViewSettings) -> OnePoleBank {
    let bin_count = (settings.max_midi - settings.min_midi) * settings.bins_per_semitone + 1;
    let configs: Vec<ResonatorConfig> = (0..bin_count)
        .map(|i| {
            let midi = settings.min_midi as f32 + i as f32 / settings.bins_per_semitone as f32;
            let frequency = midi_to_hz(midi, settings.reference_hz);
            let alpha = (heuristic_alpha(frequency, sample_rate) * settings.alpha_scale).clamp(0.0001, 1.0);
            let beta = (heuristic_alpha(frequency, sample_rate) * settings.beta_scale).clamp(0.0001, 1.0);
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
        normalize_bars(&mut spectrum, settings.gamma);
        return ResonatorSnapshot {
            spectrum,
            note_labels: settings.note_labels(),
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

    normalize_bars(&mut spectrum, settings.gamma);
    ResonatorSnapshot {
        spectrum,
        note_labels: settings.note_labels(),
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

        let snap = an.snapshot(true);
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

    /// Empty / silent input must not panic and yields an all-zero spiral.
    #[test]
    fn silence_is_quiet() {
        let sr = 44100.0;
        let mut an = ResonatorAnalyzer::new(sr);
        an.process_samples(&vec![0.0; 4096], true);
        let snap = an.snapshot(true);
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
        let nominal = an.snapshot(false);
        let reassigned = an.snapshot(true);
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
