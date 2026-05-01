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
