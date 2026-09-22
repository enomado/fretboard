//! Частота дискретизации аудиопотока — отдельный домен от частоты сигнала (`Hz`).
//!
//! Обе величины меряются в герцах, и обе раньше жили в голом `f32`/`u32` — так что
//! `RtSwipe::new(reference_hz, sample_rate)` собирался и молча считал ерунду.
//! Здесь единица — часть типа: «сэмплов в секунду», а не «колебаний в секунду».

use std::time::Duration;

/// Сэмплов в секунду.
///
/// Базовое представление — `u32`: его отдаёт cpal и пишет WAV (`hound`). Веб
/// (`AudioContext.sampleRate`) отдаёт `f32` — конструктор на той границе проверяет, что
/// дробной части нет. Внутреннее значение раскрывается `.0` на границах cpal/WAV/UI и
/// через [`Self::hz`] для DSP-формул.
///
/// `serde(transparent)`: в worker-протоколе поле выглядит как голое число.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SampleRate(pub u32);

impl SampleRate {
    /// Граница веба: `AudioContext.sampleRate` приходит `f32`. Спецификация Web Audio
    /// не обещает целого значения, но все реальные устройства отдают целые (44100,
    /// 48000); дробная частота значила бы, что `u32` здесь — неверная модель, и
    /// округлять её молча нельзя: сетка анализа разъехалась бы с потоком.
    #[cfg(target_arch = "wasm32")]
    pub fn from_web_audio(hz: f32) -> Self {
        assert!(
            hz.fract() == 0.0 && hz > 0.0,
            "AudioContext.sampleRate {hz} is not a positive whole number of Hz"
        );
        Self(hz as u32)
    }

    /// Частота как `f32` — для DSP-формул (`hz_per_bin = sr / n`, `t = i / sr`, …).
    pub fn hz(self) -> f32 {
        self.0 as f32
    }

    /// Сколько целых сэмплов помещается в `d` — пол точного произведения, в целых.
    /// Через `f32` пол врёт на единицу: 9 мс при 48 кГц — ровно 432 сэмпла, а
    /// `48000.0 * d.as_secs_f32()` даёт 431.99997 и усекается в 431.
    pub fn samples_in(self, d: Duration) -> usize {
        (self.0 as u128 * d.as_nanos() / 1_000_000_000) as usize
    }

    /// Сколько длятся `n` сэмплов — для темпирования потоков, которые кормят кольцо
    /// со скоростью устройства (`thread::sleep(sr.duration_of(block.len()))`).
    pub fn duration_of(self, n: usize) -> Duration {
        Duration::from_nanos(n as u64 * 1_000_000_000 / self.0 as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SampleRate;

    #[test]
    fn samples_in_is_exact_where_f32_would_round_down() {
        // Свидетель: тот же расчёт через f32 теряет сэмпл. Если он когда-нибудь станет
        // точным, пример перестанет что-либо доказывать — ищи другой.
        let nine_ms = Duration::from_millis(9);
        assert_eq!((48_000.0f32 * nine_ms.as_secs_f32()) as usize, 431);
        assert_eq!(SampleRate(48_000).samples_in(nine_ms), 432);
        assert_eq!(SampleRate(44_100).samples_in(Duration::from_secs(4)), 176_400);
    }

    #[test]
    fn duration_of_inverts_samples_in() {
        let sr = SampleRate(44_100);
        assert_eq!(sr.duration_of(441), Duration::from_millis(10));
        assert_eq!(sr.samples_in(sr.duration_of(22_050)), 22_050);
    }
}
