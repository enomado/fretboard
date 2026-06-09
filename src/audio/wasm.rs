use super::types::{
    AnalysisSettings,
    AudioInputOption,
    AudioStatus,
    ResonatorReading,
    TunerReading,
};

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

    pub fn resonator_reading(&self) -> Option<ResonatorReading> {
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

    pub fn play_test_note(&self, _midi: crate::core_types::pitch::PNote) {
    }

    /// Гейт резонатора — нет банка в wasm, no-op (паритет API с native).
    pub fn request_resonator(&self) {
    }
}
