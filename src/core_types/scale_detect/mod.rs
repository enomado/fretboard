//! Snail-вероятность тональности/скейла/лада — двухэтапно: КОРЕНЬ, затем ЛАД.
//!
//! Источник — резонаторный банк (точнее FFT-бинов). Спектр сворачивается в
//! 12-мерный вектор по pitch-классам (chroma): октавы наматываются на один угол,
//! ровно как на «улитке».
//!
//! Этап «лад»: для кандидата `(корень × скейл)` строим ГРАДУИРОВАННЫЙ тональный
//! профиль (форма заимствована у профилей Краумхансла–Кесслер: тоника > терция/
//! квинта > прочие ступени > «пол» вне скейла) и меряем корреляцию Пирсона с
//! наблюдаемой chroma. Пирсон вычитает среднее, поэтому энергия в НЕ-скейловых
//! нотах уходит в минус относительно среднего и честно штрафует.
//!
//! Этап «корень»: набор pitch-классов сам по себе НЕ различает относительные лады
//! (C major = A minor = D dorian — одни и те же 7 нот). Корень вытаскиваем из
//! дополнительных улик: энергии в БАСУ (низкие ноты тяготеют к тонике) и
//! УСТОЙЧИВОСТИ во времени (тоника возвращается из кадра в кадр). Эти улики
//! поднимают score кандидатов на правильном корне.
//!
//! Каждый из четырёх методов ансамбля живёт в своём файле:
//!   A — [`method_set`]     косинус с плоской маской нот;
//!   B — [`method_profile`] Пирсон с тональным профилем;
//!   C — [`method_root`]    улика корня из баса и устойчивости;
//!   D — [`method_spiral`]  центр тяжести на круге квинт.
//! Здесь, в `mod.rs`, остаётся общая инфраструктура: chroma-тип, свёртка спектра в
//! chroma, веса/оценки ансамбля и softmax — то, чем пользуются все методы.

pub mod method_profile;
pub mod method_root;
pub mod method_set;
pub mod method_spiral;

/// Число pitch-классов в равномерной темперации.
pub const PITCH_CLASS_COUNT: usize = 12;

/// 12-мерный вектор энергии по pitch-классам, индекс = pitch-класс 0..=11 (C..B).
pub type Chroma = [f32; PITCH_CLASS_COUNT];

/// Усреднённый спектр по истории кадров плюс текущий кадр (поэлементное среднее).
/// Тональность/скейл — медленная величина: интегрировать по фразе правильнее, чем
/// читать один дёрганый кадр. Кадры несогласованной длины молча пропускаются.
pub fn mean_spectrum(history: &[Vec<f32>], current: &[f32]) -> Vec<f32> {
    let len = current.len();
    if len == 0 {
        return Vec::new();
    }

    let mut acc = current.to_vec();
    let mut count = 1usize;
    for row in history {
        if row.len() != len {
            continue;
        }
        for (a, v) in acc.iter_mut().zip(row.iter()) {
            *a += *v;
        }
        count += 1;
    }
    let inv = 1.0 / count as f32;
    for a in &mut acc {
        *a *= inv;
    }
    acc
}

/// Дробный MIDI бина резонаторного банка. Контракт банка (`resonator.rs`):
/// бин `i` сидит на `min_midi + i / bins_per_semitone`.
fn bin_midi(index: usize, min_midi: usize, bins_per_semitone: usize) -> f32 {
    min_midi as f32 + index as f32 / bins_per_semitone as f32
}

/// Свернуть спектр банка в chroma по ближайшему pitch-классу, домножая энергию
/// каждого бина на `weight(midi)`. Энергия между нотами падает на ближайшую ноту.
fn fold_chroma_with<F: Fn(f32) -> f32>(
    spectrum: &[f32],
    min_midi: usize,
    bins_per_semitone: usize,
    weight: F,
) -> Chroma {
    let mut chroma = [0.0f32; PITCH_CLASS_COUNT];
    if bins_per_semitone == 0 {
        return chroma;
    }
    for (i, &energy) in spectrum.iter().enumerate() {
        let midi = bin_midi(i, min_midi, bins_per_semitone);
        let pc = (midi.round() as i64).rem_euclid(PITCH_CLASS_COUNT as i64) as usize;
        chroma[pc] += energy * weight(midi);
    }
    chroma
}

/// Полная chroma (все ноты с равным весом) — основа для подбора лада.
pub fn fold_chroma(spectrum: &[f32], min_midi: usize, bins_per_semitone: usize) -> Chroma {
    fold_chroma_with(spectrum, min_midi, bins_per_semitone, |_| 1.0)
}

/// Веса ансамбля методов. По умолчанию набор/профиль ведут, корень/спираль
/// уточняют тонику (A — набор нот, B — мажор/минор + гравитация, C — бас+устойчивость,
/// D — центр тяжести на круге квинт).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MethodWeights {
    pub set:     f32, // метод A — косинус с плоской маской
    pub profile: f32, // метод B — Пирсон с тональным профилем
    pub root:    f32, // метод C — улика корня (бас + устойчивость)
    #[serde(default)]
    pub spiral:  f32, // метод D — центр тяжести на круге квинт
}

impl Default for MethodWeights {
    fn default() -> Self {
        Self {
            set:     0.3,
            profile: 0.3,
            root:    0.2,
            spiral:  0.2,
        }
    }
}

/// Оценки методов для одного кандидата, каждая нормирована в [0, 1].
#[derive(Clone, Copy)]
pub struct MethodScores {
    pub set:     f32,
    pub profile: f32,
    pub root:    f32,
    pub spiral:  f32,
}

impl MethodScores {
    /// Взвешенное среднее методов — итоговая оценка кандидата в [0, 1].
    pub fn blended(&self, weights: MethodWeights) -> f32 {
        let total = (weights.set + weights.profile + weights.root + weights.spiral).max(1e-6);
        (weights.set * self.set
            + weights.profile * self.profile
            + weights.root * self.root
            + weights.spiral * self.spiral)
            / total
    }
}

/// Конфиг панели Scale Finder: баланс трёх методов + ширина окна интеграции —
/// сколько последних кадров истории сворачивать в chroma. Узкое окно отзывчиво,
/// но дёргано; широкое стабильно, но инертно (тональность — медленная величина).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ScaleFinderConfig {
    pub weights:       MethodWeights,
    pub window_frames: usize,
}

impl Default for ScaleFinderConfig {
    fn default() -> Self {
        Self {
            weights:       MethodWeights::default(),
            window_frames: 32,
        }
    }
}

/// Softmax с температурой: переводит близко лежащие оценки кандидатов в
/// распределение вероятностей. Меньшая `temperature` — острее пик на лидере.
pub fn softmax_with_temperature(scores: &[f32], temperature: f32) -> Vec<f32> {
    if scores.is_empty() {
        return Vec::new();
    }
    let t = temperature.max(1e-4);
    let max = scores.iter().copied().fold(f32::MIN, f32::max);
    let exps: Vec<f32> = scores.iter().map(|s| ((s - max) / t).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        return vec![0.0; scores.len()];
    }
    let inv = 1.0 / sum;
    exps.iter().map(|e| e * inv).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_lands_energy_on_nearest_pitch_class() {
        // 5 бинов/полутон, старт C0 (MIDI 12). Бин 0 = C, бин 35 = G.
        let mut spectrum = vec![0.0f32; 40];
        spectrum[0] = 1.0;
        spectrum[35] = 0.5;
        let chroma = fold_chroma(&spectrum, 12, 5);
        assert!((chroma[0] - 1.0).abs() < 0.01);
        assert!((chroma[7] - 0.5).abs() < 0.01);
        assert_eq!(chroma[1], 0.0);
    }

    #[test]
    fn softmax_sums_to_one_and_peaks_on_max() {
        let probs = softmax_with_temperature(&[0.9, 0.6, 0.6, 0.3], 0.06);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4);
        assert!(probs[0] > probs[1]);
    }
}
