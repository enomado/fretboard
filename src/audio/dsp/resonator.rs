use resonators::{
    OnePoleBank,
    ResonatorConfig,
    heuristic_alpha,
};

use super::analysis_math::{
    NOTE_BUCKET_MAX_MIDI,
    NOTE_BUCKET_MIN_MIDI,
    SPIRAL_BINS_PER_SEMITONE,
    resonator_note_labels,
    splat_linear,
};
use super::swipe::{
    SalienceFrame,
    SwipeKernel,
};
use crate::audio::sample_rate::SampleRate;
use crate::audio::types::{
    AnalysisSettings,
    BankRange,
};
use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::{
    Hz,
    Midi,
    PNote,
};

// Offline measurement of the bank's contrast across `alpha`/`beta` on real music; test-only
// and `#[ignore]`d (needs a corpus directory). Verdict in `docs/resonator_contrast.md`.
#[cfg(test)]
mod contrast_probe;

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

// --- Display levels: a dB window under a HELD white point ---
//
// The display used to divide every column by its *own* max (`normalize_bars`) and raise
// it to `gamma`. That throws away the one thing that separates a partial from junk across
// frames — level: a reverb tail, a decay, a rest, a soft passage all get blown up to full
// scale, and between partials the bank's skirts sit at whatever the frame's max allows.
// Measured against truth (`resonator/contrast_probe.rs::truth_probe`, polyphonic MDB
// mixes, 8 songs, `docs/resonator_contrast.md`): at equal on-partial brightness the share
// of the display's light that lands on a real partial went 0.365 → 0.695 (voiced frames
// only: 0.538 → 0.701). Both halves pay: holding the white point ≈ +0.2, the dB window
// instead of a power law ≈ +0.13.
//
// Brightness = `clamp(1 + (L − white) / DISPLAY_RANGE_DB, 0, 1) ^ gamma`, `L` = 20·log10
// of the column value. `white` jumps up to a louder frame's max at once (no attack lag on
// a note) and decays at `DISPLAY_WHITE_DECAY_DB_PER_S` otherwise, so a quieter passage
// comes up to full scale over (level drop / decay) seconds instead of instantly.
//
// `gamma` keeps its meaning (the user's contrast slider: higher = darker mid-tones).
// `power` too: the column then holds magnitude², so the same dB window spans half the
// signal range — the same "more contrast" the toggle always meant.
//
// The decay is a RATE ON THE AUDIO CLOCK (samples fed since the last snapshot), not a
// factor per snapshot: the snapshot cadence is the user's `update_ms` slider, and a
// per-frame constant would hand it the display's time constant
// (`memory/display_settings_must_not_steer_the_detector.md`, the "такт" trigger).
//
// Display only: the detector scores the raw column before any of this.
const DISPLAY_RANGE_DB: f32 = 30.0;
const DISPLAY_WHITE_DECAY_DB_PER_S: f32 = 6.0;

/// The display's white point: the loudest recent column max, in dB of the column's own
/// quantity, decaying at [`DISPLAY_WHITE_DECAY_DB_PER_S`]. `None` until the first column
/// with any energy — before it there is no level to hold, which is a real state (a fresh
/// or just-rebuilt bank), not a placeholder.
#[derive(Clone, Copy, Debug, Default)]
struct HeldWhite {
    level_db: Option<f32>,
}

impl HeldWhite {
    /// Advance by `dt_seconds` of audio and take in this column's max (`None` = a column
    /// with no energy at all). Returns the white point to map this column under, or
    /// `None` while nothing has ever been heard.
    fn update(&mut self, column_max: Option<f32>, dt_seconds: f32) -> Option<f32> {
        let decayed = self
            .level_db
            .map(|level| level - DISPLAY_WHITE_DECAY_DB_PER_S * dt_seconds);
        let current = column_max.map(|max| 20.0 * max.log10());
        self.level_db = match (decayed, current) {
            (Some(held), Some(now)) => Some(held.max(now)),
            (held, now) => held.or(now),
        };
        self.level_db
    }
}

/// Map a raw column to display brightness in place: the dB window under `white_db`, then
/// `gamma`. Cells below the window (and exact zeros — gated-off cells) go to 0.
fn map_display_levels(values: &mut [f32], white_db: f32, gamma: f32) {
    for v in values {
        *v = if *v > 0.0 {
            let db = 20.0 * v.log10();
            (1.0 + (db - white_db) / DISPLAY_RANGE_DB)
                .clamp(0.0, 1.0)
                .powf(gamma)
        } else {
            0.0
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResonatorViewSettings {
    bank:              BankRange,
    bins_per_semitone: usize,
    alpha_scale:       f32,
    beta_scale:        f32,
    gamma:             f32,
    power:             bool,
    // Эталон A4: меняется камертон → пересобираем банк (PartialEq ловит сдвиг).
    reference_hz:      Hz,
}

#[derive(Clone, Debug)]
pub(crate) struct ResonatorSnapshot {
    pub(crate) spectrum:    Vec<f32>,
    pub(crate) note_labels: Vec<String>,
    /// The bank's SWIPE′ salience curve for this column, for the consumer that decodes
    /// a *path* rather than a frame (`dsp::melody`). `None` for a column with no energy
    /// at all. The frame's own argmax is not carried: nothing in production reads a
    /// single-frame opinion any more (it used to ride out as `TunerReading::fast_pitch`
    /// with no reader) — the tests take it with `SalienceFrame::argmax` (`cfg(test)`).
    ///
    /// Deliberately not stored in `SharedState`: it is one frame's working material for
    /// `dsp::melody` — the `magnitudes` it carries for `swipe::refine_on_partials`
    /// double its weight, and none of that is a panel's business. The display layer a panel
    /// may draw is [`SalienceFrame::display_heat`] — built by the publisher from whichever
    /// frame *decoded the note* (bank or RT-SWIPE), not always this one, which is why the
    /// snapshot no longer carries a pre-baked heat of its own.
    pub(crate) salience:    Option<SalienceFrame>,
}

#[cfg(test)]
impl ResonatorSnapshot {
    /// The column's own single-frame opinion, `(fractional_midi, strength)` — the argmax of
    /// [`Self::salience`]. The tests' probe of the bank; see [`SalienceFrame::argmax`].
    pub(crate) fn fundamental(&self) -> Option<(f32, f32)> {
        self.salience.as_ref().and_then(SalienceFrame::argmax)
    }
}

#[derive(Debug)]
pub(crate) struct ResonatorAnalyzer {
    settings:    ResonatorViewSettings,
    sample_rate: SampleRate,
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

    // Display levels (see `DISPLAY_RANGE_DB`): the held white point, and the audio fed
    // since the last snapshot — the `dt` its decay runs on.
    display_white:          HeldWhite,
    samples_since_snapshot: usize,
}

impl ResonatorViewSettings {
    pub(crate) fn note_labels(&self, style: AccidentalStyle) -> Vec<String> {
        resonator_note_labels(self.bank, style)
    }

    /// Fractional MIDI of the bank's bin 0 — where every grid built on the bank starts.
    fn lo_midi(&self) -> Midi {
        Midi::from(self.bank.lo())
    }
}

impl Default for ResonatorViewSettings {
    fn default() -> Self {
        let note = |midi: usize| PNote::new(u8::try_from(midi).unwrap()).unwrap();
        Self {
            bank:              BankRange::new(note(RESONATOR_MIN_MIDI), note(RESONATOR_MAX_MIDI)),
            bins_per_semitone: RESONATOR_DEFAULT_BINS_PER_SEMITONE,
            alpha_scale:       1.0,
            beta_scale:        1.0,
            gamma:             0.72,
            power:             false,
            reference_hz:      Hz::A4_STANDARD,
        }
    }
}

impl From<&AnalysisSettings> for ResonatorViewSettings {
    fn from(s: &AnalysisSettings) -> Self {
        Self {
            bank:              s.resonator.bank_range(),
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
    pub(crate) fn new(sample_rate: SampleRate) -> Self {
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
            display_white: HeldWhite::default(),
            samples_since_snapshot: 0,
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
        // A new bank (new alpha, range, power…) puts its column on a new scale; a white
        // point held from the old one would black out or blow out the next seconds.
        self.display_white = HeldWhite::default();
        self.samples_since_snapshot = 0;
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
        self.samples_since_snapshot += samples.len();
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
        let dt = dn as f32 / self.sample_rate.hz();
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

    /// The bank's current column, scored for the detector and mapped for the display.
    ///
    /// `&mut`: the display's white point is held across snapshots and decays by the audio
    /// fed since the previous one (see `DISPLAY_RANGE_DB`).
    pub(crate) fn snapshot(&mut self, reassign: bool, style: AccidentalStyle) -> ResonatorSnapshot {
        let mut spectrum = raw_column(&self.bank, &self.settings, &self.detuning_hz, reassign);
        // Score BEFORE the display mapping: `gamma` is a display control (a
        // waterfall-contrast slider, `controls.rs`), and a display knob must not steer
        // the octave decision — which it did, for as long as the scoring came second.
        // SWIPE does its own warping (√), and it is not negotiable by the UI.
        // `power` is NOT covered by this: `raw_column` hands the scorer powers when it is
        // on — an open defect, see `docs/resonator_contrast.md` § Открытое.
        let (grid_bins_per_semitone, kernel) = if reassign {
            (OUTPUT_BINS_PER_SEMITONE as f32, &self.swipe_output)
        } else {
            (self.settings.bins_per_semitone as f32, &self.swipe_bank)
        };
        let salience =
            SalienceFrame::score(&spectrum, self.settings.lo_midi(), grid_bins_per_semitone, kernel);

        let dt_seconds = self.samples_since_snapshot as f32 / self.sample_rate.hz();
        self.samples_since_snapshot = 0;
        let max = spectrum.iter().copied().fold(0.0f32, f32::max);
        // No white point ⇒ nothing has ever been heard ⇒ the column is all zeros already.
        if let Some(white_db) = self.display_white.update((max > 0.0).then_some(max), dt_seconds) {
            map_display_levels(&mut spectrum, white_db, self.settings.gamma);
        }
        ResonatorSnapshot {
            spectrum,
            note_labels: self.settings.note_labels(style),
            salience,
        }
    }

    /// The raw (pre-display) column, for the probes that need the bank's own numbers
    /// rather than a picture of them — see [`raw_column`].
    #[cfg(test)]
    pub(crate) fn raw_spectrum(&self, reassign: bool) -> Vec<f32> {
        raw_column(&self.bank, &self.settings, &self.detuning_hz, reassign)
    }

    pub(crate) fn note_labels(&self, style: AccidentalStyle) -> Vec<String> {
        self.settings.note_labels(style)
    }
}

fn build_resonator_bank(sample_rate: SampleRate, settings: &ResonatorViewSettings) -> OnePoleBank {
    // Граница с крейтом `resonators`: он знает частоту дискретизации только как `f32`.
    let sample_rate = sample_rate.hz();
    let bin_count = settings.bank.semitones() * settings.bins_per_semitone + 1;
    let configs: Vec<ResonatorConfig> = (0..bin_count)
        .map(|i| {
            let midi = settings.lo_midi().0 + i as f32 / settings.bins_per_semitone as f32;
            let frequency = Midi(midi).to_hz(settings.reference_hz).0;
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

/// The bank's column **before** any display mapping — absolute magnitudes (or powers,
/// per `settings.power`), on the reassigned output grid when `reassign` is on and on the
/// bank's own grid otherwise. What the detector scores, and the only honest input for
/// anything that has to compare frames with each other (a white point, a sparsity
/// measure).
///
/// Reassignment: rather than reporting one magnitude per resonator at its nominal pitch
/// (which quantises every partial to a bin centre), we splat each bin's magnitude at
/// `f_bin + detuning` — its true frequency, recovered from the phase. This gives
/// sub-bin placement (super-resolution) on the spiral, and the coherence gate
/// turns the *consistency* of that reassignment into noise/image suppression:
/// only energy whose phase points at the bin's own tuning survives.
///
/// The bank runs at the user's `bins`/semitone; the output grid is the finer
/// `OUTPUT_BINS_PER_SEMITONE`, spanning the bank's own MIDI range so it stays
/// aligned with the (one-per-semitone) note labels.
fn raw_column(
    bank: &OnePoleBank,
    settings: &ResonatorViewSettings,
    detuning_hz: &[f32],
    reassign: bool,
) -> Vec<f32> {
    // Fallback (safety net): plain per-bin magnitude at the bin's nominal pitch,
    // at the bank's own resolution. This is the original, pre-reassignment path —
    // bit-for-bit what shipped before, so toggling it off is a clean rollback.
    if !reassign {
        return if settings.power {
            bank.powers()
        } else {
            bank.magnitudes()
        };
    }

    let out_len = settings.bank.semitones() * OUTPUT_BINS_PER_SEMITONE + 1;
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

        let midi = Hz(f_hat).to_midi(settings.reference_hz).0;
        let position = (midi - settings.lo_midi().0) * OUTPUT_BINS_PER_SEMITONE as f32;
        splat_linear(&mut spectrum, position, weight * gate);
    }
    spectrum
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    /// MIDI of an output-grid bin index, given the analyzer's range/resolution.
    fn bin_midi(an: &ResonatorAnalyzer, idx: usize) -> f32 {
        an.settings.lo_midi().0 + idx as f32 / OUTPUT_BINS_PER_SEMITONE as f32
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
        fn violin_tone(frequency_hz: f32, sample_rate: SampleRate, len: usize) -> Vec<f32> {
            let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
            (0..len)
                .map(|i| {
                    let t = i as f32 / sample_rate.hz();
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
            let sr = SampleRate(48_000);
            let hold = (sr.hz() * 0.6) as usize;
            let mut an = ResonatorAnalyzer::new(sr);
            let target_midi = Hz(to_hz).to_midi(Hz::A4_STANDARD).0;

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
                if let Some((midi, _strength)) = snap.fundamental()
                    && (midi - target_midi).abs() < 0.5
                {
                    return fed as f32 / sr.hz() * 1000.0;
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
        let sr = SampleRate(44_100);
        let mut an = ResonatorAnalyzer::new(sr);
        let target_midi = 69.1; // between bank bins at 69.0 and 69.2
        let f = Midi(target_midi).to_hz(Hz::A4_STANDARD).0;
        let sig: Vec<f32> = (0..sr.0 as usize)
            .map(|i| (TAU * f * i as f32 / sr.hz()).sin())
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
        let sr = SampleRate(44_100);
        let mut an = ResonatorAnalyzer::new(sr);
        let f = 440.0 * 2.0_f32.powf(0.1 / 12.0); // +0.1 semitone, sharp
        let sig: Vec<f32> = (0..sr.0 as usize)
            .map(|i| (TAU * f * i as f32 / sr.hz()).sin())
            .collect();
        an.process_samples(&sig, true);

        // bank bin index nearest A4 (440): (69 - lo) * bins_per_semitone
        let a4_bin = (69 - an.settings.bank.lo().as_u8() as usize) * an.settings.bins_per_semitone;
        let det = an.detuning_hz[a4_bin];
        let expected = f - 440.0; // ~2.5 Hz sharp
        assert!(det > 0.0, "sharp tone should give positive detuning, got {det}");
        assert!(
            (det - expected).abs() < 0.5,
            "detuning {det} should be near {expected} Hz"
        );
    }

    /// End-to-end through the production path (analyzer → snapshot → salience):
    /// a harmonic-rich tone like a bowed string must resolve to its *fundamental*
    /// (A4 = 69), not an overtone, and land in-range.
    #[test]
    fn fundamental_on_harmonic_tone_resolves_to_root() {
        let sr = SampleRate(44_100);
        let mut an = ResonatorAnalyzer::new(sr);
        let f0 = 440.0; // A4 = MIDI 69
        // Fundamental + 2nd + 3rd + 4th partials, fundamental strongest.
        let sig: Vec<f32> = (0..sr.0 as usize)
            .map(|i| {
                let t = i as f32 / sr.hz();
                (TAU * f0 * t).sin()
                    + 0.6 * (TAU * 2.0 * f0 * t).sin()
                    + 0.4 * (TAU * 3.0 * f0 * t).sin()
                    + 0.3 * (TAU * 4.0 * f0 * t).sin()
            })
            .collect();
        an.process_samples(&sig, true);
        let snap = an.snapshot(true, AccidentalStyle::Sharps);
        let (midi, strength) = snap.fundamental().expect("fundamental should be detected");
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
        let noise: Vec<f32> = (0..sr.0 as usize)
            .map(|i| ((i as f32 * 12.9898).sin() * 43758.547).fract() * 2.0 - 1.0)
            .collect();
        noisy.process_samples(&noise, true);
        let noise_strength = noisy
            .snapshot(true, AccidentalStyle::Sharps)
            .fundamental()
            .map(|(_, s)| s)
            .unwrap_or(0.0);
        assert!(
            strength > noise_strength,
            "a harmonic tone ({strength}) must out-score noise ({noise_strength})"
        );
    }

    /// A tone at `amplitude`, `seconds` long, at `sr`.
    fn tone(sr: SampleRate, hz: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(sr.hz() * seconds) as usize)
            .map(|i| amplitude * (TAU * hz * i as f32 / sr.hz()).sin())
            .collect()
    }

    /// Feed `samples` in `hop`-sized chunks, snapshotting after each; the last snapshot.
    fn feed(an: &mut ResonatorAnalyzer, samples: &[f32], hop: usize) -> ResonatorSnapshot {
        let mut last = None;
        for chunk in samples.chunks(hop) {
            an.process_samples(chunk, true);
            last = Some(an.snapshot(true, AccidentalStyle::Sharps));
        }
        last.unwrap()
    }

    fn peak(spectrum: &[f32]) -> f32 {
        spectrum.iter().copied().fold(0.0, f32::max)
    }

    /// REGRESSION (display levels): a passage 20 dB quieter than what came just before
    /// it must stay *dimmer* — the held white point decays at a rate, it does not snap to
    /// the new frame's max. Under the old per-frame `normalize_bars` the quiet tone's own
    /// max was full scale (peak = 1.0), which is how tails and soft passages washed out
    /// the picture (`docs/resonator_contrast.md`).
    ///
    /// Expected: 0.3 s after the drop the white point has fallen 1.8 dB of the 20, so
    /// the quiet tone sits ≈ 18 dB under white in a 30 dB window → (12/30)^0.72 ≈ 0.52.
    #[test]
    fn a_quiet_passage_after_a_loud_one_stays_dim() {
        let sr = SampleRate(48_000);
        let hop = 768; // 16 ms, the default `update_ms`
        let mut an = ResonatorAnalyzer::new(sr);
        let loud = feed(&mut an, &tone(sr, 440.0, 0.5, 0.6), hop);
        assert!(
            (peak(&loud.spectrum) - 1.0).abs() < 1e-3,
            "the loudest column defines white"
        );
        let quiet = feed(&mut an, &tone(sr, 440.0, 0.05, 0.3), hop);
        let p = peak(&quiet.spectrum);
        assert!(
            (0.4..0.65).contains(&p),
            "20 dB down, 0.3 s later: peak {p:.3} (≈ 0.52 expected, 1.0 = per-frame normalisation)"
        );
    }

    /// A louder column takes the white point at once — no attack lag, so a note never
    /// comes in clipped above white for a few frames.
    ///
    /// Asserted on a partial *below* the peak, not as "peak = 1.0": the clamp makes the
    /// peak 1.0 under a lagging white point too (that version of this test passed against
    /// a white point that never rose at all). So: A4 with its octave 6 dB down. Under a
    /// white point at this column's own max the octave sits at (1 − 6/30)^0.72 ≈ 0.85; a
    /// stale, lower white point clips it to 1.0 along with the fundamental. A pure tone
    /// cannot show this — reassignment collapses it into one or two cells.
    #[test]
    fn a_louder_note_takes_the_white_point_at_once() {
        let sr = SampleRate(48_000);
        let hop = 768;
        let a4_and_octave = |amplitude: f32, seconds: f32| -> Vec<f32> {
            tone(sr, 440.0, amplitude, seconds)
                .iter()
                .zip(tone(sr, 880.0, amplitude * 0.5, seconds))
                .map(|(a, b)| a + b)
                .collect()
        };
        let mut an = ResonatorAnalyzer::new(sr);
        feed(&mut an, &a4_and_octave(0.05, 0.6), hop);
        let loud = feed(&mut an, &a4_and_octave(0.5, 0.2), hop);
        let cell =
            |midi: f32| ((midi - an.settings.lo_midi().0) * OUTPUT_BINS_PER_SEMITONE as f32).round() as usize;
        let octave = peak(&loud.spectrum[cell(80.5)..=cell(81.5)]);
        assert!(
            (peak(&loud.spectrum) - 1.0).abs() < 1e-3,
            "the fundamental defines white"
        );
        assert!(
            (0.75..0.93).contains(&octave),
            "the octave, 6 dB down, reads {octave:.3} (≈ 0.85 expected; 1.0 = white lags the note)"
        );
    }

    /// REGRESSION (the "такт" trigger, `memory/display_settings_must_not_steer_the_detector.md`):
    /// the white point's decay runs on the AUDIO clock, so the snapshot cadence — the
    /// user's `update_ms` slider, 8..80 ms — must not change the picture. A per-snapshot
    /// decay factor would make the quiet tone ~10× brighter at 8 ms than at 80 ms.
    #[test]
    fn the_white_point_decay_ignores_the_snapshot_cadence() {
        let sr = SampleRate(48_000);
        let signal: Vec<f32> = tone(sr, 440.0, 0.5, 0.6)
            .into_iter()
            .chain(tone(sr, 440.0, 0.05, 0.96))
            .collect();
        let peaks: Vec<f32> = [384usize, 768, 3840] // 8, 16, 80 ms
            .iter()
            .map(|&hop| {
                let mut an = ResonatorAnalyzer::new(sr);
                peak(&feed(&mut an, &signal, hop).spectrum)
            })
            .collect();
        for p in &peaks {
            assert!(
                (p - peaks[1]).abs() < 0.02,
                "peaks at 8/16/80 ms cadence {peaks:?} must agree"
            );
        }
    }

    /// Empty / silent input must not panic and yields an all-zero spiral.
    #[test]
    fn silence_is_quiet() {
        let sr = SampleRate(44_100);
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
        let sr = SampleRate(44_100);
        let mut an = ResonatorAnalyzer::new(sr);
        let sig: Vec<f32> = (0..sr.0 as usize)
            .map(|i| (TAU * 440.0 * i as f32 / sr.hz()).sin())
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
