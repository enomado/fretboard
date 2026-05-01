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

#[cfg(not(target_arch = "wasm32"))]
const MIN_WINDOW_SIZE: usize = 2048;
#[cfg(not(target_arch = "wasm32"))]
const MAX_WINDOW_SIZE: usize = 16384;
#[cfg(not(target_arch = "wasm32"))]
const MIN_FFT_SIZE: usize = 4096;
#[cfg(not(target_arch = "wasm32"))]
const MAX_FFT_SIZE: usize = 32768;
#[cfg(not(target_arch = "wasm32"))]
const LOWEST_TRACKED_FREQUENCY: f32 = 16.0;

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

#[cfg(not(target_arch = "wasm32"))]
impl AnalysisSettings {
    pub(crate) fn sanitized(mut self) -> Self {
        self.window_size = self.window_size.clamp(MIN_WINDOW_SIZE, MAX_WINDOW_SIZE);
        let min_fft_for_window = self
            .window_size
            .next_power_of_two()
            .clamp(MIN_FFT_SIZE, MAX_FFT_SIZE);
        self.fft_size = self
            .fft_size
            .max(MIN_FFT_SIZE)
            .next_power_of_two()
            .clamp(min_fft_for_window, MAX_FFT_SIZE);
        self.min_frequency = self.min_frequency.clamp(LOWEST_TRACKED_FREQUENCY, 1_200.0);
        self.max_frequency = self.max_frequency.clamp(120.0, 4_000.0);
        if self.max_frequency <= self.min_frequency + 40.0 {
            self.max_frequency = (self.min_frequency + 40.0).clamp(120.0, 4_000.0);
        }
        self.spectrum_smoothing = self.spectrum_smoothing.min(4);
        self.note_spread = self.note_spread.clamp(0.15, 0.8);
        self.spectrum_gamma = self.spectrum_gamma.clamp(0.35, 1.2);
        self.note_gamma = self.note_gamma.clamp(0.35, 1.2);
        self.resonator_min_midi = self.resonator_min_midi.clamp(12, 84);
        self.resonator_max_midi = self.resonator_max_midi.clamp(24, 108);
        if self.resonator_max_midi <= self.resonator_min_midi + 6 {
            self.resonator_max_midi = (self.resonator_min_midi + 6).clamp(24, 108);
        }
        self.resonator_bins = self.resonator_bins.clamp(1, 12);
        self.resonator_alpha = self.resonator_alpha.clamp(0.2, 4.0);
        self.resonator_beta = self.resonator_beta.clamp(0.2, 4.0);
        self.resonator_gamma = self.resonator_gamma.clamp(0.35, 1.2);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_settings_are_sanitized() {
        let settings = AnalysisSettings {
            window_size:        500,
            fft_size:           1_000,
            min_frequency:      900.0,
            max_frequency:      920.0,
            spectrum_smoothing: 12,
            note_spread:        0.01,
            spectrum_gamma:     0.01,
            note_gamma:         9.0,
            resonator_min_midi: 10,
            resonator_max_midi: 11,
            resonator_bins:     99,
            resonator_alpha:    0.01,
            resonator_beta:     9.0,
            resonator_gamma:    9.0,
        }
        .sanitized();

        assert!(settings.window_size >= MIN_WINDOW_SIZE);
        assert!(settings.fft_size >= settings.window_size.next_power_of_two());
        assert!(settings.max_frequency > settings.min_frequency);
        assert!(settings.spectrum_smoothing <= 4);
        assert!((0.15..=0.8).contains(&settings.note_spread));
        assert!(settings.resonator_max_midi > settings.resonator_min_midi);
        assert!((1..=12).contains(&settings.resonator_bins));
    }
}
