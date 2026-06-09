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
    power:             bool,
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
    bank:        ResonatorBank,
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
        }
    }
}

impl From<&AnalysisSettings> for ResonatorViewSettings {
    fn from(s: &AnalysisSettings) -> Self {
        Self {
            min_midi:          s.resonator.min_midi,
            max_midi:          s.resonator.max_midi,
            bins_per_semitone: s.resonator.bins,
            alpha_scale:       s.resonator.alpha,
            beta_scale:        s.resonator.beta,
            gamma:             s.resonator.gamma,
            power:             s.resonator.power,
        }
    }
}

impl ResonatorAnalyzer {
    pub(super) fn new(sample_rate: f32) -> Self {
        let settings = ResonatorViewSettings::default();
        let bank = build_resonator_bank(sample_rate, &settings);
        Self {
            settings,
            sample_rate,
            bank,
        }
    }

    pub(super) fn sync_settings(&mut self, requested: ResonatorViewSettings) -> bool {
        if requested == self.settings {
            return false;
        }
        self.settings = requested;
        self.bank = build_resonator_bank(self.sample_rate, &self.settings);
        true
    }

    pub(super) fn process_samples(&mut self, samples: &[f32]) {
        self.bank.process_samples(samples);
    }

    pub(super) fn snapshot(&self) -> ResonatorSnapshot {
        resonator_snapshot(&self.bank, &self.settings)
    }

    pub(super) fn note_labels(&self) -> Vec<String> {
        self.settings.note_labels()
    }
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
    let mut spectrum = if settings.power {
        bank.powers()
    } else {
        bank.magnitudes()
    };
    normalize_bars(&mut spectrum, settings.gamma);
    ResonatorSnapshot {
        spectrum,
        note_labels: settings.note_labels(),
    }
}
