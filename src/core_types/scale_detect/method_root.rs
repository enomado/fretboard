//! Метод C («root») — улика КОРНЯ из двух источников, недоступных методам A/B
//! (которые видят лишь набор нот и потому слепы к относительным ладам):
//!   • БАС — низкие ноты тяготеют к тонике (бас-взвешенная chroma);
//!   • УСТОЙЧИВОСТЬ во времени — тоника возвращается из кадра в кадр.
//! Смесь этих улик на pitch-класс и говорит, какой класс похож на тонику.

use super::{
    Chroma,
    PITCH_CLASS_COUNT,
    fold_chroma,
    fold_chroma_with,
};

// --- Бас-окно для улики корня: ниже FULL — полный вес, выше ZERO — игнор ---
const BASS_FULL_MIDI: f32 = 36.0; // C2
const BASS_ZERO_MIDI: f32 = 60.0; // C4

// --- Смешивание улик корня ---
const ROOT_WEIGHT_BASS: f32 = 0.6;
const ROOT_WEIGHT_PERSISTENCE: f32 = 0.4;

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn root_evidence_favours_bass_note() {
        // Бас давит на G (pc 7), устойчивость ровная — корень-улика максимальна на G.
        let mut bass = [0.1f32; PITCH_CLASS_COUNT];
        bass[7] = 1.0;
        let flat_persistence = [0.5f32; PITCH_CLASS_COUNT];
        let evidence = root_evidence(&bass, &flat_persistence);
        let best = (0..PITCH_CLASS_COUNT).max_by(|a, b| evidence[*a].total_cmp(&evidence[*b]));
        assert_eq!(best, Some(7));
    }
}
