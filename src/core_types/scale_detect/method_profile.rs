//! Метод B («tonal»/profile) — ГРАДУИРОВАННЫЙ тональный профиль скейла (тоника >
//! терция/квинта > прочие ступени > «пол» вне скейла; форма заимствована у
//! Краумхансла–Кесслер) мерится корреляцией ПИРСОНА с наблюдаемой chroma. Пирсон
//! центрирует векторы, поэтому энергия вне скейла уходит в минус и штрафуется,
//! а мажор/минор и гравитация тоники различаются.

use super::{
    Chroma,
    PITCH_CLASS_COUNT,
};
use crate::core_types::scale::Scale;

// --- Веса градуированного тонального профиля (величины ~ как у K–K) ---
const PROFILE_TONIC: f32 = 6.3;
const PROFILE_FIFTH: f32 = 4.8;
const PROFILE_THIRD: f32 = 4.4;
const PROFILE_SCALE_TONE: f32 = 3.3;
// Не ноль: Пирсон центрирует векторы, и «пол» делает чужие ноты отрицательным
// отклонением от среднего — это и есть штраф за энергию вне скейла.
const PROFILE_NON_SCALE: f32 = 2.2;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::pitch::PCNote;

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
}
