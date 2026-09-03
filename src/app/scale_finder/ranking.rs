//! Ранжирование кандидатов `(корень × скейл/лад)`: считает все четыре метода на
//! окне chroma-кадров, сливает их оценки в одну и раскладывает по вероятностям.
//! Здесь же — палитра методов и общие для подфайлов типы результата.

use eframe::egui::Color32;

use super::super::{
    ALL_SCALES,
    ScaleKind,
};
use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::PCNote;
use crate::core_types::scale_detect::chroma::{
    Chroma,
    PITCH_CLASS_COUNT,
    mean_chroma,
};
use crate::core_types::scale_detect::ensemble::{
    MethodScores,
    ScaleFinderConfig,
    softmax_with_temperature,
};
use crate::core_types::scale_detect::method_profile::{
    TonalProfile,
    pearson,
    unit_from_pearson,
};
use crate::core_types::scale_detect::method_root::{
    persistence_chroma,
    root_evidence,
};
use crate::core_types::scale_detect::method_set::{
    FlatTemplate,
    cosine,
};
use crate::core_types::scale_detect::method_spiral::{
    center_of_effect,
    key_point,
    spiral_proximity,
};

const SOFTMAX_TEMPERATURE: f32 = 0.06;
pub(super) const RANKING_ROWS: usize = 5;
/// Доля от пика кадра, выше которой pitch-класс считается «заметным» (метод C).
const PROMINENCE_RATIO: f32 = 0.5;

// Цвета методов — отдельная палитра, чтобы не путать с цветом нот.
pub(super) const COLOR_SET: Color32 = Color32::from_rgb(112, 204, 238); // A — голубой
pub(super) const COLOR_PROFILE: Color32 = Color32::from_rgb(180, 150, 246); // B — фиолетовый
pub(super) const COLOR_ROOT: Color32 = Color32::from_rgb(230, 180, 110); // C — янтарный
pub(super) const COLOR_SPIRAL: Color32 = Color32::from_rgb(124, 214, 160); // D — зелёный

/// Один кандидат `(корень × скейл/лад)` с оценками всех трёх методов.
pub(super) struct ScaleCandidate {
    pub(super) root_pc:     usize,
    pub(super) kind:        ScaleKind,
    pub(super) scores:      MethodScores,
    pub(super) blended:     f32,
    pub(super) probability: f32,
}

impl ScaleCandidate {
    pub(super) fn label(&self, style: AccidentalStyle) -> String {
        format!("{} {}", style.pitch_class_name(self.root_pc), self.kind.label())
    }
}

/// Результат разбора одного кадра.
pub(super) struct Ranking {
    pub(super) chroma:      Chroma,
    pub(super) chroma_peak: f32,
    pub(super) root_ev:     Chroma,   // улика корня по pitch-классам (метод C)
    pub(super) ce:          [f32; 2], // центр тяжести на круге квинт (метод D)
    pub(super) candidates:  Vec<ScaleCandidate>, // отсортированы по убыванию blended
}

/// Индексы топ-`n` кандидатов по произвольной проекции оценки (своя для каждого
/// метода). Полный список мал (12×N), сортировки копии хватает.
pub(super) fn top_indices(
    candidates: &[ScaleCandidate],
    project: impl Fn(&ScaleCandidate) -> f32,
    n: usize,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| project(&candidates[b]).total_cmp(&project(&candidates[a])));
    order.truncate(n);
    order
}

/// Цвет метода, который сильнее всех поддержал этого кандидата (доминирующий вклад).
pub(super) fn dominant_method_color(scores: &MethodScores) -> Color32 {
    let mut best = (COLOR_SET, scores.set);
    if scores.profile > best.1 {
        best = (COLOR_PROFILE, scores.profile);
    }
    if scores.root > best.1 {
        best = (COLOR_ROOT, scores.root);
    }
    if scores.spiral > best.1 {
        best = (COLOR_SPIRAL, scores.spiral);
    }
    best.0
}

/// Проранжировать 12×N кандидатов всеми четырьмя методами по окну chroma-кадров,
/// уже свёрнутых решалкой. `None` — окно пустое или полная тишина.
pub(super) fn rank(
    chroma_frames: &[Chroma],
    bass_frames: &[Chroma],
    config: ScaleFinderConfig,
) -> Option<Ranking> {
    if chroma_frames.is_empty() {
        return None;
    }

    // chroma и бас — средние по окну; устойчивость — по тем же кадрам окна.
    let chroma = mean_chroma(chroma_frames);
    let chroma_peak = chroma.iter().copied().fold(0.0, f32::max);
    if chroma_peak <= 0.0 {
        return None;
    }

    // Метод C готовится один раз: бас + устойчивость во времени → улика корня.
    let bass = mean_chroma(bass_frames);
    let persist = persistence_chroma(chroma_frames, PROMINENCE_RATIO);
    let root_ev = root_evidence(&bass, &persist);
    // Метод D готовится один раз: центр тяжести chroma на круге квинт.
    let ce = center_of_effect(&chroma);
    let weights = config.weights;

    let mut candidates = Vec::with_capacity(PITCH_CLASS_COUNT * ALL_SCALES.len());
    let mut blended_scores = Vec::with_capacity(PITCH_CLASS_COUNT * ALL_SCALES.len());
    for (root_pc, &root_ev_pc) in root_ev.iter().enumerate() {
        let root = PCNote(root_pc as u8);
        for &kind in &ALL_SCALES {
            let scale = kind.to_scale(root);
            let set = cosine(&chroma, &FlatTemplate::from_scale(&scale).weights);
            let profile = unit_from_pearson(pearson(&chroma, &TonalProfile::from_scale(&scale).weights));
            let spiral = spiral_proximity(&ce, &key_point(&scale));
            let scores = MethodScores {
                set,
                profile,
                root: root_ev_pc,
                spiral,
            };
            let blended = scores.blended(weights);
            blended_scores.push(blended);
            candidates.push(ScaleCandidate {
                root_pc,
                kind,
                scores,
                blended,
                probability: 0.0,
            });
        }
    }

    let probabilities = softmax_with_temperature(&blended_scores, SOFTMAX_TEMPERATURE);
    for (candidate, probability) in candidates.iter_mut().zip(probabilities) {
        candidate.probability = probability;
    }

    candidates.sort_by(|a, b| {
        b.blended
            .partial_cmp(&a.blended)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Some(Ranking {
        chroma,
        chroma_peak,
        root_ev,
        ce,
        candidates,
    })
}
