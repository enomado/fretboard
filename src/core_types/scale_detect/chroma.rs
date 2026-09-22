//! Вход ансамбля: 12-мерный вектор энергии по pitch-классам и свёртка спектра
//! банка в него. Октавы наматываются на один угол — ровно как на «улитке», —
//! поэтому дальше все методы работают с 12 числами, а не со спектром.

use crate::core_types::pitch::{
    Midi,
    PNote,
};

/// Число pitch-классов в равномерной темперации.
pub const PITCH_CLASS_COUNT: usize = 12;

/// 12-мерный вектор энергии по pitch-классам, индекс = pitch-класс 0..=11 (C..B).
pub type Chroma = [f32; PITCH_CLASS_COUNT];

/// Поэлементное среднее набора chroma-кадров (окна интеграции). Тональность —
/// медленная величина: усреднять по окну правильнее, чем читать дёрганый кадр.
pub fn mean_chroma(frames: &[Chroma]) -> Chroma {
    let mut acc = [0.0f32; PITCH_CLASS_COUNT];
    if frames.is_empty() {
        return acc;
    }
    for frame in frames {
        for (a, v) in acc.iter_mut().zip(frame.iter()) {
            *a += *v;
        }
    }
    let inv = 1.0 / frames.len() as f32;
    for a in &mut acc {
        *a *= inv;
    }
    acc
}

/// Дробный MIDI бина резонаторного банка. Контракт банка (`resonator.rs`):
/// бин `i` сидит на `min_midi + i / bins_per_semitone`, где `min_midi` — нижняя нота
/// банка (бин 0).
fn bin_midi(index: usize, min_midi: PNote, bins_per_semitone: usize) -> f32 {
    Midi::from(min_midi).0 + index as f32 / bins_per_semitone as f32
}

/// Свернуть спектр банка в chroma по ближайшему pitch-классу, домножая энергию
/// каждого бина на `weight(midi)`. Энергия между нотами падает на ближайшую ноту.
pub(super) fn fold_chroma_with<F: Fn(f32) -> f32>(
    spectrum: &[f32],
    min_midi: PNote,
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
pub fn fold_chroma(spectrum: &[f32], min_midi: PNote, bins_per_semitone: usize) -> Chroma {
    fold_chroma_with(spectrum, min_midi, bins_per_semitone, |_| 1.0)
}

#[cfg(test)]
mod tests {
    use super::fold_chroma;
    use crate::core_types::pitch::PNote;

    #[test]
    fn fold_lands_energy_on_nearest_pitch_class() {
        // 5 бинов/полутон, старт C0 (MIDI 12). Бин 0 = C, бин 35 = G.
        let mut spectrum = vec![0.0f32; 40];
        spectrum[0] = 1.0;
        spectrum[35] = 0.5;
        let chroma = fold_chroma(&spectrum, PNote::new(12).unwrap(), 5);
        assert!((chroma[0] - 1.0).abs() < 0.01);
        assert!((chroma[7] - 0.5).abs() < 0.01);
        assert_eq!(chroma[1], 0.0);
    }
}
