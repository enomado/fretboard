#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioInputKind {
    Microphone,
    System,
    Other,
}

#[derive(Clone, Debug)]
pub struct AudioInputOption {
    pub id:    String,
    pub label: String,
    pub kind:  AudioInputKind,
}

#[derive(Clone, Debug)]
pub struct AnalysisSettings {
    pub window_size:        usize,
    pub fft_size:           usize,
    pub min_frequency:      f32,
    pub max_frequency:      f32,
    pub spectrum_smoothing: usize,
    pub note_spread:        f32,
    pub spectrum_gamma:     f32,
    pub note_gamma:         f32,
    pub resonator_min_midi: usize,
    pub resonator_max_midi: usize,
    pub resonator_bins:     usize,
    pub resonator_alpha:    f32,
    pub resonator_beta:     f32,
    pub resonator_gamma:    f32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            window_size:        6144,
            fft_size:           16384,
            min_frequency:      16.0,
            max_frequency:      2_000.0,
            spectrum_smoothing: 1,
            note_spread:        0.35,
            spectrum_gamma:     0.58,
            note_gamma:         0.72,
            resonator_min_midi: 12,
            resonator_max_midi: 84,
            resonator_bins:     5,
            resonator_alpha:    1.0,
            resonator_beta:     1.0,
            resonator_gamma:    0.72,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TunerReading {
    pub frequency_hz:          f32,
    pub note_name:             String,
    pub cents:                 f32,
    pub clarity:               f32,
    pub spectrum:              Vec<f32>,
    pub waterfall:             Vec<Vec<f32>>,
    pub note_spectrum:         Vec<f32>,
    pub note_waterfall:        Vec<Vec<f32>>,
    pub spiral_spectrum:       Vec<f32>,
    pub spiral_waterfall:      Vec<Vec<f32>>,
    pub resonator_spectrum:    Vec<f32>,
    pub resonator_waterfall:   Vec<Vec<f32>>,
    pub resonator_note_labels: Vec<String>,
    pub note_labels:           Vec<String>,
}

#[derive(Clone, Debug)]
pub enum AudioStatus {
    Idle,
    Listening,
    Error(String),
}

pub struct AudioEngine;

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn status(&self) -> AudioStatus {
        AudioStatus::Error("Microphone tuner is not implemented for wasm yet".to_owned())
    }

    pub fn reading(&self) -> Option<TunerReading> {
        None
    }

    pub fn analysis_settings(&self) -> AnalysisSettings {
        AnalysisSettings::default()
    }

    pub fn set_analysis_settings(&self, _settings: AnalysisSettings) {
    }

    pub fn input_gain(&self) -> f32 {
        1.0
    }

    pub fn set_input_gain(&self, _gain: f32) {
    }

    pub fn input_gain_range(&self) -> (f32, f32) {
        (0.1, 12.0)
    }

    pub fn input_level(&self) -> f32 {
        0.0
    }

    pub fn input_waveform(&self) -> Vec<f32> {
        Vec::new()
    }

    pub fn monitor_enabled(&self) -> bool {
        false
    }

    pub fn set_monitor_enabled(&self, _enabled: bool) {
    }

    pub fn monitor_gain(&self) -> f32 {
        0.0
    }

    pub fn set_monitor_gain(&self, _gain: f32) {
    }

    pub fn current_input_sample_rate(&self) -> u32 {
        0
    }

    pub fn monitor_output_sample_rate(&self) -> Option<u32> {
        None
    }

    pub fn default_output_device_name(&self) -> Option<String> {
        None
    }

    pub fn available_inputs(&self) -> Vec<AudioInputOption> {
        Vec::new()
    }

    pub fn selected_input_id(&self) -> Option<String> {
        None
    }

    pub fn set_selected_input_id(&self, _input_id: Option<String>) {
    }

    pub fn play_test_note(&self, _midi: usize) {
    }
}
