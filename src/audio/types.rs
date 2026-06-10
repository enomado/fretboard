use crate::core_types::pitch::PNote;

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
pub struct ResonatorReading {
    pub spectrum:    Vec<f32>,
    pub waterfall:   Vec<Vec<f32>>,
    pub note_labels: Vec<String>,
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnalysisSettings {
    pub window_size:        usize,
    pub fft_size:           usize,
    pub min_frequency:      f32,
    pub max_frequency:      f32,
    /// Эталон A4 (камертон / concert pitch) в Гц. Задаёт стандарт строя:
    /// 440 — ISO, 442/443 — многие европейские оркестры, 430 — классика,
    /// 415 — барокко. Влияет на маппинг частота↔нота во всём движке.
    ///
    /// `serde(default)` — мягкая миграция: в RON прошлых версий поля нет, и без
    /// дефолта весь снимок настроек сбросился бы. Берём 440.0 (а НЕ `f32`-ноль).
    #[serde(default = "default_concert_pitch_hz")]
    pub concert_pitch_hz:   f32,
    pub spectrum_smoothing: usize,
    pub note_spread:        f32,
    pub spectrum_gamma:     f32,
    pub note_gamma:         f32,
    pub resonator:          ResonatorSettings,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResonatorSettings {
    pub min_midi:  PNote,
    pub max_midi:  PNote,
    pub bins:      usize,
    pub alpha:     f32,
    pub beta:      f32,
    pub gamma:     f32,
    pub history:   usize,
    pub update_ms: u64,
    pub power:     bool,
    /// Δφ instantaneous-frequency reassignment on the spiral: each bin's energy
    /// is placed at its measured true frequency (super-resolution) and a
    /// coherence gate suppresses the negative-frequency image / noise. Turning it
    /// off falls back to plain per-bin magnitude at the bin's nominal pitch — the
    /// original behaviour, kept as a safety net.
    ///
    /// `serde(default)` — soft migration: configs saved before this field existed
    /// load with reassignment on (the intended default), not `bool`'s `false`.
    #[serde(default = "default_reassign")]
    pub reassign:  bool,
}

fn default_reassign() -> bool {
    true
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

fn default_concert_pitch_hz() -> f32 {
    440.0
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            window_size:        6144,
            fft_size:           16384,
            min_frequency:      16.0,
            max_frequency:      2_000.0,
            concert_pitch_hz:   440.0,
            spectrum_smoothing: 1,
            note_spread:        0.35,
            spectrum_gamma:     0.58,
            note_gamma:         0.72,
            resonator:          ResonatorSettings::default(),
        }
    }
}

impl Default for ResonatorSettings {
    fn default() -> Self {
        Self {
            min_midi:  PNote::new(12).unwrap(),
            max_midi:  PNote::new(84).unwrap(),
            bins:      5,
            alpha:     1.0,
            beta:      1.0,
            gamma:     0.72,
            history:   52,
            update_ms: 16,
            power:     false,
            reassign:  true,
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
        // Камертон: нижняя граница 400 Гц (чуть ниже барочного 415 — под
        // верди-строй и исторические низкие диапазоны), верхняя 466.16 (A#4).
        // Покрывает все академические стандарты (430/440/442/443/444) и не
        // даёт уехать в бессмыслицу.
        self.concert_pitch_hz = self.concert_pitch_hz.clamp(400.0, 466.0);
        self.spectrum_smoothing = self.spectrum_smoothing.min(4);
        self.note_spread = self.note_spread.clamp(0.15, 0.8);
        self.spectrum_gamma = self.spectrum_gamma.clamp(0.35, 1.2);
        self.note_gamma = self.note_gamma.clamp(0.35, 1.2);
        self.resonator = self.resonator.sanitized();
        self
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ResonatorSettings {
    pub(crate) fn sanitized(mut self) -> Self {
        // Clamp in raw-MIDI space (the newtype has no arithmetic), keeping the
        // invariant max ≥ min + 6, then rebuild the validated `PNote`s. The
        // bounds stay inside 0..=127, so `new` can't fail here.
        let min = self.min_midi.as_u8().clamp(12, 84);
        let mut max = self.max_midi.as_u8().clamp(24, 108);
        if max <= min + 6 {
            max = (min + 6).clamp(24, 108);
        }
        self.min_midi = PNote::new(min).unwrap();
        self.max_midi = PNote::new(max).unwrap();
        self.bins = self.bins.clamp(1, 12);
        self.alpha = self.alpha.clamp(0.05, 12.0);
        self.beta = self.beta.clamp(0.05, 12.0);
        self.gamma = self.gamma.clamp(0.15, 2.4);
        self.history = self.history.clamp(8, 240);
        self.update_ms = self.update_ms.clamp(8, 80);
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
            concert_pitch_hz:   900.0,
            spectrum_smoothing: 12,
            note_spread:        0.01,
            spectrum_gamma:     0.01,
            note_gamma:         9.0,
            resonator:          ResonatorSettings {
                min_midi:  PNote::new(10).unwrap(),
                max_midi:  PNote::new(11).unwrap(),
                bins:      99,
                alpha:     0.01,
                beta:      9.0,
                gamma:     9.0,
                history:   999,
                update_ms: 1,
                power:     false,
                reassign:  true,
            },
        }
        .sanitized();

        assert!(settings.window_size >= MIN_WINDOW_SIZE);
        assert!(settings.fft_size >= settings.window_size.next_power_of_two());
        assert!(settings.max_frequency > settings.min_frequency);
        assert!((400.0..=466.0).contains(&settings.concert_pitch_hz));
        assert!(settings.spectrum_smoothing <= 4);
        assert!((0.15..=0.8).contains(&settings.note_spread));
        assert!(settings.resonator.max_midi > settings.resonator.min_midi);
        assert!((1..=12).contains(&settings.resonator.bins));
        assert!((0.05..=12.0).contains(&settings.resonator.alpha));
        assert!((0.05..=12.0).contains(&settings.resonator.beta));
        assert!((0.15..=2.4).contains(&settings.resonator.gamma));
        assert!((8..=240).contains(&settings.resonator.history));
        assert!((8..=80).contains(&settings.resonator.update_ms));
    }

    #[test]
    fn missing_concert_pitch_in_old_ron_defaults_to_a440() {
        // Снимок до появления камертона: поля concert_pitch_hz нет. Мягкая
        // миграция должна подставить 440.0, а не уронить разбор всего снимка.
        let ron = ron::ser::to_string(&AnalysisSettings::default()).unwrap();
        let legacy = ron.replace(",concert_pitch_hz:440.0", "");
        assert!(
            !legacy.contains("concert_pitch_hz"),
            "field must be absent for the test to be meaningful"
        );

        let restored: AnalysisSettings = ron::from_str(&legacy).unwrap();
        assert_eq!(restored.concert_pitch_hz, 440.0);
    }
}
