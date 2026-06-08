use resonators::{
    ResonatorBank,
    ResonatorConfig,
    heuristic_alpha,
    midi_to_hz,
};

use super::analysis_math::{
    NOTE_BUCKET_MAX_MIDI,
    NOTE_BUCKET_MIN_MIDI,
    normalize_bars,
    resonator_note_labels,
};
use crate::audio::types::AnalysisSettings;

const RESONATOR_MIN_MIDI: usize = NOTE_BUCKET_MIN_MIDI;
const RESONATOR_MAX_MIDI: usize = NOTE_BUCKET_MAX_MIDI;
const RESONATOR_DEFAULT_BINS_PER_SEMITONE: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ResonatorViewSettings {
    min_midi:          usize,
    max_midi:          usize,
    bins_per_semitone: usize,
    alpha_scale:       f32,
    beta_scale:        f32,
    gamma:             f32,
}

#[derive(Clone, Debug)]
pub(super) struct ResonatorSnapshot {
    pub(super) spectrum:    Vec<f32>,
    pub(super) note_labels: Vec<String>,
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
        }
    }
}

impl From<&AnalysisSettings> for ResonatorViewSettings {
    fn from(s: &AnalysisSettings) -> Self {
        Self {
            min_midi:          s.resonator_min_midi,
            max_midi:          s.resonator_max_midi,
            bins_per_semitone: s.resonator_bins,
            alpha_scale:       s.resonator_alpha,
            beta_scale:        s.resonator_beta,
            gamma:             s.resonator_gamma,
        }
    }
}

pub(super) fn resonator_snapshot_for_window(
    window: &[f32],
    sample_rate: f32,
    settings: &ResonatorViewSettings,
) -> ResonatorSnapshot {
    let mut bank = build_resonator_bank(sample_rate, settings);
    bank.process_samples(window);
    resonator_snapshot(&bank, settings)
}

fn build_resonator_bank(sample_rate: f32, settings: &ResonatorViewSettings) -> ResonatorBank {
    let bin_count = (settings.max_midi - settings.min_midi) * settings.bins_per_semitone + 1;
    let configs: Vec<ResonatorConfig> = (0..bin_count)
        .map(|i| {
            let midi = settings.min_midi as f32 + i as f32 / settings.bins_per_semitone as f32;
            let frequency = midi_to_hz(midi, 440.0);
            let alpha = (heuristic_alpha(frequency, sample_rate) * settings.alpha_scale).clamp(0.0001, 1.0);
            let beta = (heuristic_alpha(frequency, sample_rate) * settings.beta_scale).clamp(0.0001, 1.0);
            ResonatorConfig::new(frequency, alpha, beta)
        })
        .collect();
    ResonatorBank::new(&configs, sample_rate)
}

fn resonator_snapshot(bank: &ResonatorBank, settings: &ResonatorViewSettings) -> ResonatorSnapshot {
    let mut spectrum = bank.magnitudes();
    normalize_bars(&mut spectrum, settings.gamma);
    ResonatorSnapshot {
        spectrum,
        note_labels: settings.note_labels(),
    }
}
