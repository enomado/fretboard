//! Метод A («set») — совпадение НАБОРА нот: плоский шаблон скейла (ноты=1, вне=0,
//! тоника/квинта подняты) мерится КОСИНУСОМ с наблюдаемой chroma. Отвечает на
//! вопрос «насколько chroma похожа на ЭТОТ набор нот», но относительные лады
//! (один набор нот) не различает — этим заняты методы C и D.

use super::{
    Chroma,
    PITCH_CLASS_COUNT,
};
use crate::core_types::scale::Scale;

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
