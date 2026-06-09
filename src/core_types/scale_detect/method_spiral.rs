//! Метод D («spiral») — центр тяжести на круге КВИНТ (Spiral Array Элейн Чу,
//! pitch-class форма). Ноты раскладываются по кругу квинт — гармонически близкие
//! рядом. Центр тяжести звучащих нот указывает на тональный центр; ближайшая по
//! этому центру тональность и есть ответ. Берём 2D-круг (он замыкается, в отличие
//! от 3D-спирали Чу, которая для pitch-классов не сходится по высоте).

use super::{
    Chroma,
    PITCH_CLASS_COUNT,
};
use crate::core_types::scale::Scale;

const KEY_POINT_TONIC: f32 = 3.0;
const KEY_POINT_FIFTH: f32 = 2.0;
const KEY_POINT_THIRD: f32 = 1.5;
const KEY_POINT_TONE: f32 = 1.0;
/// Резкость перевода расстояния «центр↔тональность» в близость [0,1].
const SPIRAL_SHARPNESS: f32 = 2.2;

/// Угол pitch-класса на круге квинт. j = номер квинты от C: 7 — обратный сам себе
/// по модулю 12, поэтому j=(7·pc)%12 даёт порядок C,G,D,A,E,B,F#,…
fn fifths_angle(pc: usize) -> f32 {
    let j = (7 * pc) % PITCH_CLASS_COUNT;
    j as f32 * std::f32::consts::TAU / PITCH_CLASS_COUNT as f32
}

/// Точка pitch-класса на единичном круге квинт `[cos, sin]`.
pub fn fifths_point(pc: usize) -> [f32; 2] {
    let a = fifths_angle(pc);
    [a.cos(), a.sin()]
}

/// Центр тяжести chroma на круге квинт. Длина результата ~ сила тонального центра
/// (1 — вся энергия в одной ноте, 0 — размазано равномерно по квинтам).
pub fn center_of_effect(chroma: &Chroma) -> [f32; 2] {
    let mut x = 0.0;
    let mut y = 0.0;
    let mut sum = 0.0;
    for pc in 0..PITCH_CLASS_COUNT {
        let p = fifths_point(pc);
        x += chroma[pc] * p[0];
        y += chroma[pc] * p[1];
        sum += chroma[pc];
    }
    if sum <= f32::EPSILON {
        return [0.0, 0.0];
    }
    [x / sum, y / sum]
}

/// Точка-представитель скейла на круге квинт — взвешенный центроид его нот
/// (тоника/квинта/терция тяжелее, центр тяготеет к тонике).
pub fn key_point(scale: &Scale) -> [f32; 2] {
    let mut member = [false; PITCH_CLASS_COUNT];
    for pc in scale.notes() {
        member[pc.0 as usize % PITCH_CLASS_COUNT] = true;
    }
    let root = scale.root.0 as usize % PITCH_CLASS_COUNT;
    let fifth = (root + 7) % PITCH_CLASS_COUNT;
    let minor_third = (root + 3) % PITCH_CLASS_COUNT;
    let major_third = (root + 4) % PITCH_CLASS_COUNT;

    let mut x = 0.0;
    let mut y = 0.0;
    let mut sum = 0.0;
    for (pc, &is_member) in member.iter().enumerate() {
        if !is_member {
            continue;
        }
        let weight = if pc == root {
            KEY_POINT_TONIC
        } else if pc == fifth {
            KEY_POINT_FIFTH
        } else if pc == minor_third || pc == major_third {
            KEY_POINT_THIRD
        } else {
            KEY_POINT_TONE
        };
        let p = fifths_point(pc);
        x += weight * p[0];
        y += weight * p[1];
        sum += weight;
    }
    if sum <= f32::EPSILON {
        return [0.0, 0.0];
    }
    [x / sum, y / sum]
}

/// Евклидово расстояние в 2D.
pub fn dist2(a: &[f32; 2], b: &[f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// Близость центра тяжести к представителю тональности → [0,1], 1 = совпали.
pub fn spiral_proximity(ce: &[f32; 2], key: &[f32; 2]) -> f32 {
    (-dist2(ce, key) * SPIRAL_SHARPNESS).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::pitch::PCNote;
    use crate::core_types::scale_detect::method_profile::TonalProfile;

    #[test]
    fn center_of_effect_of_single_note_is_its_fifths_point() {
        let mut chroma = [0.0f32; PITCH_CLASS_COUNT];
        chroma[0] = 1.0; // только C
        let ce = center_of_effect(&chroma);
        let c = fifths_point(0);
        assert!((ce[0] - c[0]).abs() < 1e-5 && (ce[1] - c[1]).abs() < 1e-5);
    }

    #[test]
    fn spiral_centre_is_closer_to_c_major_than_to_distant_f_sharp() {
        // chroma из профиля C major: центр тяжести должен сидеть ближе к
        // представителю C major, чем к удалённому по квинтам F# major.
        let chroma = TonalProfile::from_scale(&Scale::major(PCNote(0))).weights;
        let ce = center_of_effect(&chroma);
        let near = spiral_proximity(&ce, &key_point(&Scale::major(PCNote(0))));
        let far = spiral_proximity(&ce, &key_point(&Scale::major(PCNote(6)))); // F#
        assert!(near > far, "C major proximity {near} должна бить F# {far}");
    }
}
