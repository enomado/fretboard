//! «Определялка» тональности/скейла/лада на улитке — АНСАМБЛЬ из четырёх методов.
//!
//! Четыре метода считаются одновременно, каждый ловит своё:
//!   A («set»)    — косинус chroma с плоской маской нот: «какой НАБОР нот звучит».
//!   B («tonal»)  — Пирсон chroma с тональным профилем: мажор/минор и гравитация
//!                  тоники (форма профиля заимствована у Краумхансла–Кесслер).
//!   C («root»)   — улика корня из баса и устойчивости во времени: какой именно
//!                  pitch-класс тоника (различает относительные лады, где A и B
//!                  бессильны — один набор нот).
//!   D («spiral») — центр тяжести chroma на круге квинт: тональный центр стрелкой.
//! Итог = взвешенное среднее → softmax → проценты. Вердикт каждого метода
//! показывается отдельно, чтобы видеть, кто на чём настаивает.
//!
//! Файлы панели: [`controls`] — слайдеры баланса методов и окна; [`wheel`] — левая
//! колонка (хроматическое колесо + кольцо квинт метода D); [`panel`] — правая
//! колонка (вердикты методов + слитый рейтинг). Здесь — ранжирование (`rank`),
//! каркас карточки и общие для подфайлов типы/константы.
//!
//! ИЗОЛЯЦИЯ: весь расчёт внутри `draw_scale_finder_card`. egui_tiles зовёт
//! `pane_ui` только для видимой вкладки — пока панель закрыта, ничего не считается.
//! Состояния в `App` детектор не держит.

mod controls;
mod panel;
pub(crate) mod solver;
mod wheel;

use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use super::{
    ALL_SCALES,
    App,
    ScaleKind,
    pill,
};
use crate::core_types::pitch::PCNote;
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
use crate::core_types::scale_detect::{
    Chroma,
    MethodScores,
    PITCH_CLASS_COUNT,
    ScaleFinderConfig,
    mean_chroma,
    softmax_with_temperature,
};
use crate::ui::theme::PANEL_FILL;

const SOFTMAX_TEMPERATURE: f32 = 0.06;
const RANKING_ROWS: usize = 5;
/// Доля от пика кадра, выше которой pitch-класс считается «заметным» (метод C).
const PROMINENCE_RATIO: f32 = 0.5;

// Цвета методов — отдельная палитра, чтобы не путать с цветом нот.
const COLOR_SET: Color32 = Color32::from_rgb(112, 204, 238); // A — голубой
const COLOR_PROFILE: Color32 = Color32::from_rgb(180, 150, 246); // B — фиолетовый
const COLOR_ROOT: Color32 = Color32::from_rgb(230, 180, 110); // C — янтарный
const COLOR_SPIRAL: Color32 = Color32::from_rgb(124, 214, 160); // D — зелёный

/// Один кандидат `(корень × скейл/лад)` с оценками всех трёх методов.
struct ScaleCandidate {
    root_pc:     usize,
    kind:        ScaleKind,
    scores:      MethodScores,
    blended:     f32,
    probability: f32,
}

impl ScaleCandidate {
    fn label(&self) -> String {
        format!(
            "{} {}",
            crate::ui::snail::SPIRAL_PITCH_LABELS[self.root_pc],
            self.kind.label()
        )
    }
}

/// Результат разбора одного кадра.
struct Ranking {
    chroma:      Chroma,
    chroma_peak: f32,
    root_ev:     Chroma,              // улика корня по pitch-классам (метод C)
    ce:          [f32; 2],            // центр тяжести на круге квинт (метод D)
    candidates:  Vec<ScaleCandidate>, // отсортированы по убыванию blended
}

/// Индексы топ-`n` кандидатов по произвольной проекции оценки (своя для каждого
/// метода). Полный список мал (12×N), сортировки копии хватает.
fn top_indices(
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
fn dominant_method_color(scores: &MethodScores) -> Color32 {
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
fn rank(chroma_frames: &[Chroma], bass_frames: &[Chroma], config: ScaleFinderConfig) -> Option<Ranking> {
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
    for root_pc in 0..PITCH_CLASS_COUNT {
        let root = PCNote(root_pc as u8);
        for &kind in &ALL_SCALES {
            let scale = kind.to_scale(root);
            let set = cosine(&chroma, &FlatTemplate::from_scale(&scale).weights);
            let profile = unit_from_pearson(pearson(&chroma, &TonalProfile::from_scale(&scale).weights));
            let spiral = spiral_proximity(&ce, &key_point(&scale));
            let scores = MethodScores {
                set,
                profile,
                root: root_ev[root_pc],
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

impl App {
    pub(super) fn draw_scale_finder_card(&mut self, ui: &mut Ui) {
        // Эта панель — потребитель резонаторного банка: пока она видима, держим
        // его «нужным». Закрылась → запросы прекратились → банк паркуется.
        self.audio.request_resonator();

        let now = std::time::Instant::now();
        let reading = self.audio.reading();
        let settings = self.audio.analysis_settings();

        // Тик решалки — только пока панель видима (этот метод зовётся лишь для
        // активной вкладки): закрыта → не тикается, окно стынет, ничего не считается.
        if let Some(reading) = &reading {
            self.scale_solver.tick(now, reading, &settings);
        }
        let (chroma_frames, bass_frames) = self.scale_solver.window(now, self.scale_finder.window_seconds);
        let ranking = rank(&chroma_frames, &bass_frames, self.scale_finder);
        let captured_secs = self.scale_solver.captured_secs(now);

        let selected_root_pc = PCNote::from_natural(self.root_note).0 as usize;
        let selected_kind = self.scale_kind;

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Scale finder")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            RichText::new("4 methods on the snail: notes · tonal · root · spiral")
                                .color(Color32::from_rgb(152, 158, 165)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match ranking.as_ref().and_then(|r| r.candidates.first()) {
                            Some(top) => {
                                pill(
                                    ui,
                                    &format!("{:.0}%  {}", top.probability * 100.0, top.label()),
                                    Color32::from_rgb(214, 206, 192),
                                    Color32::from_rgb(64, 68, 73),
                                )
                            }
                            None => {
                                pill(
                                    ui,
                                    "waiting for input",
                                    Color32::from_rgb(184, 188, 196),
                                    Color32::from_rgb(56, 61, 68),
                                )
                            }
                        }
                    });
                });

                ui.add_space(10.0);
                self.draw_scale_finder_controls(ui, captured_secs);
                ui.add_space(10.0);
                draw_scale_finder_body(ui, ranking.as_ref(), selected_root_pc, selected_kind);
            });
    }
}

fn draw_scale_finder_body(
    ui: &mut Ui,
    ranking: Option<&Ranking>,
    selected_root_pc: usize,
    selected_kind: ScaleKind,
) {
    use eframe::egui::{
        FontId,
        Rect,
    };

    let available = ui.available_size_before_wrap();
    let desired = vec2(available.x, available.y.max(420.0));
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
    painter.rect_stroke(
        rect,
        18.0,
        Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
        egui::StrokeKind::Inside,
    );

    let Some(ranking) = ranking else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Play a sustained phrase — the snail will rank key, scale and mode",
            FontId::proportional(13.0),
            Color32::from_rgb(139, 143, 149),
        );
        return;
    };

    let pad = 16.0;
    let inner = rect.shrink(pad);
    let wheel_w = inner.width() * 0.42;
    let wheel_rect = Rect::from_min_size(inner.min, vec2(wheel_w, inner.height()));
    let list_rect = Rect::from_min_max(pos2(wheel_rect.right() + pad, inner.top()), inner.max);

    // Левая колонка делится на хроматическую улитку (chroma по полутонам) и кольцо
    // КВИНТ (метод D): на нём центр тяжести показан стрелкой к тональному центру.
    let snail_h = wheel_rect.height() * 0.58;
    let snail_rect = Rect::from_min_size(wheel_rect.min, vec2(wheel_rect.width(), snail_h));
    let fifths_rect = Rect::from_min_max(pos2(wheel_rect.left(), snail_rect.bottom() + 6.0), wheel_rect.max);

    wheel::draw_chroma_wheel(&painter, snail_rect, ranking);
    wheel::draw_fifths_ring(&painter, fifths_rect, ranking);
    panel::draw_method_panel(&painter, list_rect, ranking, selected_root_pc, selected_kind);
}
