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

use crate::core_types::scale::Scale;

/// Число pitch-классов в равномерной темперации.
pub const PITCH_CLASS_COUNT: usize = 12;

/// 12-мерный вектор энергии по pitch-классам, индекс = pitch-класс 0..=11 (C..B).
pub type Chroma = [f32; PITCH_CLASS_COUNT];

// --- Веса градуированного тонального профиля (величины ~ как у K–K) ---
const PROFILE_TONIC: f32 = 6.3;
const PROFILE_FIFTH: f32 = 4.8;
const PROFILE_THIRD: f32 = 4.4;
const PROFILE_SCALE_TONE: f32 = 3.3;
// Не ноль: Пирсон центрирует векторы, и «пол» делает чужие ноты отрицательным
// отклонением от среднего — это и есть штраф за энергию вне скейла.
const PROFILE_NON_SCALE: f32 = 2.2;

// --- Бас-окно для улики корня: ниже FULL — полный вес, выше ZERO — игнор ---
const BASS_FULL_MIDI: f32 = 36.0; // C2
const BASS_ZERO_MIDI: f32 = 60.0; // C4

// --- Смешивание улик корня ---
const ROOT_WEIGHT_BASS: f32 = 0.6;
const ROOT_WEIGHT_PERSISTENCE: f32 = 0.4;

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

/// Бас-вес: 1.0 ниже `BASS_FULL_MIDI`, линейно к 0 на `BASS_ZERO_MIDI`, 0 выше.
fn bass_weight(midi: f32) -> f32 {
    ((BASS_ZERO_MIDI - midi) / (BASS_ZERO_MIDI - BASS_FULL_MIDI)).clamp(0.0, 1.0)
}

/// Бас-взвешенная chroma — улика корня: низкие ноты тяготеют к тонике.
pub fn fold_bass_chroma(spectrum: &[f32], min_midi: usize, bins_per_semitone: usize) -> Chroma {
    fold_chroma_with(spectrum, min_midi, bins_per_semitone, bass_weight)
}

/// Устойчивость pitch-классов во времени: доля кадров истории, где данный класс
/// был «заметен» (энергия ≥ `prominence_ratio` от пика кадра). Тоника и опорные
/// тоны возвращаются из кадра в кадр → высокая устойчивость.
pub fn persistence(
    history: &[Vec<f32>],
    min_midi: usize,
    bins_per_semitone: usize,
    prominence_ratio: f32,
) -> Chroma {
    let mut present = [0.0f32; PITCH_CLASS_COUNT];
    let mut frames = 0.0f32;
    for row in history {
        let frame = fold_chroma(row, min_midi, bins_per_semitone);
        let peak = frame.iter().copied().fold(0.0, f32::max);
        if peak <= 0.0 {
            continue;
        }
        frames += 1.0;
        let threshold = peak * prominence_ratio;
        for pc in 0..PITCH_CLASS_COUNT {
            if frame[pc] >= threshold {
                present[pc] += 1.0;
            }
        }
    }
    if frames <= 0.0 {
        return present;
    }
    let inv = 1.0 / frames;
    for p in &mut present {
        *p *= inv;
    }
    present
}

/// Собрать улику корня на pitch-класс из баса и устойчивости, нормировав к пику 1.
/// `root_evidence[pc]` ≈ «насколько похоже, что тоника здесь».
pub fn root_evidence(bass: &Chroma, persistence: &Chroma) -> Chroma {
    let bass_peak = bass.iter().copied().fold(0.0, f32::max).max(1e-6);
    let mut out = [0.0f32; PITCH_CLASS_COUNT];
    for pc in 0..PITCH_CLASS_COUNT {
        out[pc] = ROOT_WEIGHT_BASS * (bass[pc] / bass_peak) + ROOT_WEIGHT_PERSISTENCE * persistence[pc];
    }
    let peak = out.iter().copied().fold(0.0, f32::max).max(1e-6);
    let inv = 1.0 / peak;
    for o in &mut out {
        *o *= inv;
    }
    out
}

// --- Метод A: совпадение НАБОРА нот (плоская маска + тоника/квинта) ---
const FLAT_SCALE_TONE: f32 = 1.0;
const FLAT_ROOT: f32 = 2.0;
const FLAT_FIFTH: f32 = 1.3;

/// Плоский шаблон скейла: ноты=1, вне=0, тоника/квинта подняты. Меряется
/// косинусом — отвечает на вопрос «насколько chroma похожа на ЭТОТ набор нот».
pub struct FlatTemplate {
    pub weights: Chroma,
}

impl FlatTemplate {
    pub fn from_scale(scale: &Scale) -> Self {
        let mut weights = [0.0f32; PITCH_CLASS_COUNT];
        for pc in scale.notes() {
            weights[pc.0 as usize % PITCH_CLASS_COUNT] = FLAT_SCALE_TONE;
        }
        let root = scale.root.0 as usize % PITCH_CLASS_COUNT;
        weights[root] = FLAT_ROOT;
        let fifth = (root + 7) % PITCH_CLASS_COUNT;
        if weights[fifth] > 0.0 {
            weights[fifth] = weights[fifth].max(FLAT_FIFTH);
        }
        Self { weights }
    }
}

/// Косинусная близость двух chroma-векторов в [0, 1] (оба неотрицательны).
pub fn cosine(a: &Chroma, b: &Chroma) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..PITCH_CLASS_COUNT {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON { 0.0 } else { dot / denom }
}

/// Градуированный тональный профиль скейла в chroma-пространстве.
pub struct TonalProfile {
    pub weights: Chroma,
}

impl TonalProfile {
    pub fn from_scale(scale: &Scale) -> Self {
        let mut member = [false; PITCH_CLASS_COUNT];
        for pc in scale.notes() {
            member[pc.0 as usize % PITCH_CLASS_COUNT] = true;
        }

        let mut weights = [PROFILE_NON_SCALE; PITCH_CLASS_COUNT];
        for (pc, &is_member) in member.iter().enumerate() {
            if is_member {
                weights[pc] = PROFILE_SCALE_TONE;
            }
        }

        let root = scale.root.0 as usize % PITCH_CLASS_COUNT;
        weights[root] = PROFILE_TONIC;

        // Квинта (если входит — в локрийском уменьшённая, корень+6, и не поднимается).
        let fifth = (root + 7) % PITCH_CLASS_COUNT;
        if member[fifth] {
            weights[fifth] = PROFILE_FIFTH;
        }

        // Терция — какая в скейле (минорная корень+3 или мажорная корень+4).
        let minor_third = (root + 3) % PITCH_CLASS_COUNT;
        let major_third = (root + 4) % PITCH_CLASS_COUNT;
        if member[minor_third] {
            weights[minor_third] = PROFILE_THIRD;
        } else if member[major_third] {
            weights[major_third] = PROFILE_THIRD;
        }

        Self { weights }
    }
}

/// Корреляция Пирсона двух chroma-векторов в [-1, 1] (центрирует оба).
/// Нулевая дисперсия (плоский вектор/тишина) → 0.
pub fn pearson(a: &Chroma, b: &Chroma) -> f32 {
    let n = PITCH_CLASS_COUNT as f32;
    let mean_a = a.iter().sum::<f32>() / n;
    let mean_b = b.iter().sum::<f32>() / n;
    let mut cov = 0.0f32;
    let mut var_a = 0.0f32;
    let mut var_b = 0.0f32;
    for i in 0..PITCH_CLASS_COUNT {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    let denom = (var_a * var_b).sqrt();
    if denom <= f32::EPSILON { 0.0 } else { cov / denom }
}

/// Линейно растянуть корреляцию Пирсона из [-1, 1] в [0, 1] — чтобы метод B
/// смешивался в общей шкале с косинусом (A) и уликой корня (C).
pub fn unit_from_pearson(corr: f32) -> f32 {
    ((corr + 1.0) * 0.5).clamp(0.0, 1.0)
}

/// Веса ансамбля трёх методов. По умолчанию — почти поровну: каждый ловит своё
/// (A — набор нот, B — мажор/минор + тональную гравитацию, C — реальный корень).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MethodWeights {
    pub set:     f32, // метод A — косинус с плоской маской
    pub profile: f32, // метод B — Пирсон с тональным профилем
    pub root:    f32, // метод C — улика корня (бас + устойчивость)
}

impl Default for MethodWeights {
    fn default() -> Self {
        Self {
            set:     0.34,
            profile: 0.33,
            root:    0.33,
        }
    }
}

/// Оценки трёх методов для одного кандидата, каждая нормирована в [0, 1].
#[derive(Clone, Copy)]
pub struct MethodScores {
    pub set:     f32,
    pub profile: f32,
    pub root:    f32,
}

impl MethodScores {
    /// Взвешенное среднее трёх методов — итоговая оценка кандидата в [0, 1].
    pub fn blended(&self, weights: MethodWeights) -> f32 {
        let total = (weights.set + weights.profile + weights.root).max(1e-6);
        (weights.set * self.set + weights.profile * self.profile + weights.root * self.root) / total
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
    use crate::core_types::pitch::PCNote;

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
    fn bass_fold_keeps_low_notes_and_drops_high_ones() {
        let mut spectrum = vec![0.0f32; 400];
        spectrum[0] = 1.0; // MIDI 12 — глубокий бас, полный вес
        spectrum[60 * 5 - 12 * 5] = 1.0; // MIDI 60 (C4) — на верхней границе, вес ~0
        let bass = fold_bass_chroma(&spectrum, 12, 5);
        // MIDI 60 (C4) — снова C, но с нулевым бас-весом, вклад только от MIDI 12.
        assert!((bass[0] - 1.0).abs() < 0.05);
    }

    #[test]
    fn pearson_of_identical_is_one() {
        let v = [6.3, 2.2, 3.3, 2.2, 3.3, 3.3, 2.2, 4.8, 2.2, 3.3, 2.2, 3.3];
        assert!((pearson(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn c_major_chroma_correlates_higher_with_c_major_than_relative_minor() {
        // chroma = профиль C major. Корреляция с C major = 1, с относительным
        // A minor строго ниже: пики профилей стоят на разных тониках.
        let c_major = TonalProfile::from_scale(&Scale::major(PCNote(0)));
        let a_minor = TonalProfile::from_scale(&Scale::minor(PCNote(9)));
        let chroma = c_major.weights;

        let corr_c = pearson(&chroma, &c_major.weights);
        let corr_a = pearson(&chroma, &a_minor.weights);

        assert!((corr_c - 1.0).abs() < 1e-5);
        assert!(corr_c > corr_a, "C major {corr_c} должен бить A minor {corr_a}");
    }

    #[test]
    fn root_evidence_favours_bass_note() {
        // Бас давит на G (pc 7), устойчивость ровная — корень-улика максимальна на G.
        let mut bass = [0.1f32; PITCH_CLASS_COUNT];
        bass[7] = 1.0;
        let flat_persistence = [0.5f32; PITCH_CLASS_COUNT];
        let evidence = root_evidence(&bass, &flat_persistence);
        let best = (0..PITCH_CLASS_COUNT).max_by(|a, b| evidence[*a].total_cmp(&evidence[*b]));
        assert_eq!(best, Some(7));
    }

    #[test]
    fn softmax_sums_to_one_and_peaks_on_max() {
        let probs = softmax_with_temperature(&[0.9, 0.6, 0.6, 0.3], 0.06);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4);
        assert!(probs[0] > probs[1]);
    }
}
