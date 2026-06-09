//! «Определялка» тональности/скейла/лада на улитке — АНСАМБЛЬ из трёх методов.
//!
//! Три метода считаются одновременно, каждый ловит своё:
//!   A («set»)    — косинус chroma с плоской маской нот: «какой НАБОР нот звучит».
//!   B («tonal»)  — Пирсон chroma с тональным профилем: мажор/минор и гравитация
//!                  тоники (форма профиля заимствована у Краумхансла–Кесслер).
//!   C («root»)   — улика корня из баса и устойчивости во времени: какой именно
//!                  pitch-класс тоника (различает относительные лады, где A и B
//!                  бессильны — один набор нот).
//! Итог = взвешенное среднее трёх → softmax → проценты. Вердикт каждого метода
//! показывается отдельно, чтобы видеть, кто на чём настаивает.
//!
//! ИЗОЛЯЦИЯ: весь расчёт внутри `draw_scale_finder_card`. egui_tiles зовёт
//! `pane_ui` только для видимой вкладки — пока панель закрыта, ничего не считается.
//! Состояния в `App` детектор не держит.

use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    FontId,
    Frame,
    Margin,
    Rect,
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
use crate::audio::{
    AnalysisSettings,
    TunerReading,
};
use crate::core_types::pitch::PCNote;
use crate::core_types::scale_detect::{
    self,
    Chroma,
    FlatTemplate,
    MethodScores,
    MethodWeights,
    PITCH_CLASS_COUNT,
    TonalProfile,
};
use crate::ui::snail::{
    SPIRAL_PITCH_LABELS,
    pitch_class_angle,
    pitch_class_color,
};
use crate::ui::theme::PANEL_FILL;

const SOFTMAX_TEMPERATURE: f32 = 0.06;
const RANKING_ROWS: usize = 7;
/// Доля от пика кадра, выше которой pitch-класс считается «заметным» (метод C).
const PROMINENCE_RATIO: f32 = 0.5;

// Цвета методов в разбивке — отдельная палитра, чтобы не путать с цветом нот.
const COLOR_SET: Color32 = Color32::from_rgb(112, 204, 238); // A — голубой
const COLOR_PROFILE: Color32 = Color32::from_rgb(180, 150, 246); // B — фиолетовый
const COLOR_ROOT: Color32 = Color32::from_rgb(230, 180, 110); // C — янтарный

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
        format!("{} {}", SPIRAL_PITCH_LABELS[self.root_pc], self.kind.label())
    }
}

/// Результат разбора одного кадра.
struct Ranking {
    chroma:       Chroma,
    chroma_peak:  f32,
    candidates:   Vec<ScaleCandidate>, // отсортированы по убыванию blended
    set_pick:     (usize, ScaleKind),  // вердикт метода A
    profile_pick: (usize, ScaleKind),  // вердикт метода B
    root_pick_pc: usize,               // вердикт метода C (только корень)
}

fn label_of(pick: (usize, ScaleKind)) -> String {
    format!("{} {}", SPIRAL_PITCH_LABELS[pick.0], pick.1.label())
}

/// Argmax кандидата по произвольной проекции оценки (вердикт одного метода).
fn pick_by(candidates: &[ScaleCandidate], project: impl Fn(&ScaleCandidate) -> f32) -> (usize, ScaleKind) {
    let best = candidates
        .iter()
        .max_by(|a, b| project(a).total_cmp(&project(b)))
        .expect("candidate list is non-empty by construction");
    (best.root_pc, best.kind)
}

/// Свернуть резонаторный снимок в chroma и проранжировать 12×N кандидатов всеми
/// тремя методами. `None` — нет резонаторных бинов или полная тишина.
fn rank(reading: &TunerReading, settings: &AnalysisSettings, weights: MethodWeights) -> Option<Ranking> {
    if reading.resonator_spectrum.is_empty() {
        return None;
    }

    // Контракт банка (`resonator.rs`): бин i = MIDI `min_midi + i/bins_per_semitone`.
    let min_midi = settings.resonator.min_midi.as_u8() as usize;
    let bins = settings.resonator.bins;

    let mean = scale_detect::mean_spectrum(&reading.resonator_waterfall, &reading.resonator_spectrum);
    let chroma = scale_detect::fold_chroma(&mean, min_midi, bins);
    let chroma_peak = chroma.iter().copied().fold(0.0, f32::max);
    if chroma_peak <= 0.0 {
        return None;
    }

    // Метод C готовится один раз: бас + устойчивость во времени → улика корня.
    let bass = scale_detect::fold_bass_chroma(&mean, min_midi, bins);
    let persist = scale_detect::persistence(&reading.resonator_waterfall, min_midi, bins, PROMINENCE_RATIO);
    let root_ev = scale_detect::root_evidence(&bass, &persist);

    let mut candidates = Vec::with_capacity(PITCH_CLASS_COUNT * ALL_SCALES.len());
    let mut blended_scores = Vec::with_capacity(PITCH_CLASS_COUNT * ALL_SCALES.len());
    for root_pc in 0..PITCH_CLASS_COUNT {
        let root = PCNote(root_pc as u8);
        for &kind in &ALL_SCALES {
            let scale = kind.to_scale(root);
            let set = scale_detect::cosine(&chroma, &FlatTemplate::from_scale(&scale).weights);
            let profile = scale_detect::unit_from_pearson(scale_detect::pearson(
                &chroma,
                &TonalProfile::from_scale(&scale).weights,
            ));
            let scores = MethodScores {
                set,
                profile,
                root: root_ev[root_pc],
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

    let probabilities = scale_detect::softmax_with_temperature(&blended_scores, SOFTMAX_TEMPERATURE);
    for (candidate, probability) in candidates.iter_mut().zip(probabilities) {
        candidate.probability = probability;
    }

    // Вердикты отдельных методов берём ДО сортировки — каждый argmax по своему сигналу.
    let set_pick = pick_by(&candidates, |c| c.scores.set);
    let profile_pick = pick_by(&candidates, |c| c.scores.profile);
    let root_pick_pc = (0..PITCH_CLASS_COUNT)
        .max_by(|a, b| root_ev[*a].total_cmp(&root_ev[*b]))
        .unwrap();

    candidates.sort_by(|a, b| {
        b.blended
            .partial_cmp(&a.blended)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Some(Ranking {
        chroma,
        chroma_peak,
        candidates,
        set_pick,
        profile_pick,
        root_pick_pc,
    })
}

impl App {
    pub(super) fn draw_scale_finder_card(&mut self, ui: &mut Ui) {
        let reading = self.audio.reading();
        let settings = self.audio.analysis_settings();
        // Веса берём из конфига App. Слайдеры ниже меняют их для следующего кадра
        // (egui перерисовывается непрерывно — лаг в один кадр незаметен).
        let ranking = reading
            .as_ref()
            .and_then(|reading| rank(reading, &settings, self.scale_finder_weights));

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
                            RichText::new("3 methods on the snail: notes · tonal profile · root")
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
                self.draw_method_weight_sliders(ui);
                ui.add_space(10.0);
                draw_scale_finder_body(ui, ranking.as_ref(), selected_root_pc, selected_kind);
            });
    }

    /// Три слайдера баланса методов прямо в панели (конфиг живёт с панелью —
    /// панель остаётся самодостаточной). `blended()` нормирует на сумму весов,
    /// поэтому каждый слайдер — относительный вклад, сумма к 1 не обязана.
    fn draw_method_weight_sliders(&mut self, ui: &mut Ui) {
        let weights = &mut self.scale_finder_weights;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Method mix")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            weight_slider(ui, "notes", COLOR_SET, &mut weights.set);
            weight_slider(ui, "tonal", COLOR_PROFILE, &mut weights.profile);
            weight_slider(ui, "root", COLOR_ROOT, &mut weights.root);
            if ui.button("Reset").clicked() {
                *weights = MethodWeights::default();
            }
        });
    }
}

/// Один помеченный слайдер веса метода (0..1) с цветной меткой и числом.
fn weight_slider(ui: &mut Ui, label: &str, color: Color32, value: &mut f32) {
    ui.label(RichText::new(label).color(color).strong().size(12.0));
    ui.add_sized(
        [110.0, 18.0],
        egui::Slider::new(value, 0.0..=1.0)
            .clamping(egui::SliderClamping::Always)
            .trailing_fill(true)
            .show_value(false),
    );
    ui.label(
        RichText::new(format!("{value:.2}"))
            .color(Color32::from_rgb(226, 216, 201))
            .monospace(),
    );
}

fn draw_scale_finder_body(
    ui: &mut Ui,
    ranking: Option<&Ranking>,
    selected_root_pc: usize,
    selected_kind: ScaleKind,
) {
    let available = ui.available_size_before_wrap();
    let desired = vec2(available.x, available.y.max(380.0));
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
    let wheel_side = inner.height().min(inner.width() * 0.46);
    let wheel_rect = Rect::from_min_size(inner.min, vec2(wheel_side, inner.height()));
    let list_rect = Rect::from_min_max(pos2(wheel_rect.right() + pad, inner.top()), inner.max);

    draw_chroma_wheel(&painter, wheel_rect, ranking);
    draw_method_panel(&painter, list_rect, ranking, selected_root_pc, selected_kind);
}

/// 12-спицевое колесо pitch-классов: размер точки = энергия chroma, кольца-маркеры
/// = ноты топ-кандидата, жирная спица = его тоника.
fn draw_chroma_wheel(painter: &egui::Painter, rect: Rect, ranking: &Ranking) {
    let square = rect.width().min(rect.height());
    let chart = Rect::from_center_size(rect.center(), vec2(square, square));
    let center = chart.center();
    let radius = square * 0.34;
    let peak = ranking.chroma_peak.max(1e-6);

    let top = &ranking.candidates[0];
    let top_scale = top.kind.to_scale(PCNote(top.root_pc as u8));
    let mut member = [false; PITCH_CLASS_COUNT];
    for pc in top_scale.notes() {
        member[pc.0 as usize % PITCH_CLASS_COUNT] = true;
    }

    painter.circle_stroke(
        center,
        radius,
        Stroke::new(1.0_f32, Color32::from_rgb(59, 64, 72)),
    );

    let root_angle = pitch_class_angle(top.root_pc);
    let root_dir = vec2(root_angle.cos(), root_angle.sin());
    painter.line_segment(
        [center, center + root_dir * radius],
        Stroke::new(1.8_f32, pitch_class_color(top.root_pc)),
    );

    for pc in 0..PITCH_CLASS_COUNT {
        let angle = pitch_class_angle(pc);
        let dir = vec2(angle.cos(), angle.sin());
        let color = pitch_class_color(pc);
        let spoke_end = center + dir * radius;

        painter.line_segment(
            [center + dir * (radius * 0.16), spoke_end],
            Stroke::new(1.0_f32, Color32::from_rgb(48, 52, 59)),
        );
        painter.text(
            center + dir * (radius + 18.0),
            egui::Align2::CENTER_CENTER,
            SPIRAL_PITCH_LABELS[pc],
            FontId::proportional(15.0),
            color,
        );

        if member[pc] {
            let is_root = pc == top.root_pc;
            painter.circle_stroke(
                spoke_end,
                if is_root { 8.0 } else { 5.5 },
                Stroke::new(if is_root { 2.2_f32 } else { 1.3_f32 }, color),
            );
        }

        let intensity = (ranking.chroma[pc] / peak).clamp(0.0, 1.0);
        if intensity > 0.02 {
            painter.circle_filled(
                spoke_end,
                2.0 + intensity * 9.0,
                Color32::from_rgba_unmultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    (40.0 + intensity * 150.0) as u8,
                ),
            );
            painter.circle_filled(spoke_end, 1.5 + intensity * 2.5, color);
        }
    }

    painter.text(
        pos2(center.x, chart.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        top.label(),
        FontId::proportional(13.0),
        Color32::from_rgb(214, 206, 192),
    );
}

/// Правая колонка: вердикты трёх методов + слитый рейтинг с разбивкой по методам.
fn draw_method_panel(
    painter: &egui::Painter,
    rect: Rect,
    ranking: &Ranking,
    selected_root_pc: usize,
    selected_kind: ScaleKind,
) {
    // --- Три вердикта в строке ---
    painter.text(
        pos2(rect.left(), rect.top()),
        egui::Align2::LEFT_TOP,
        "Three methods",
        FontId::proportional(13.0),
        Color32::from_rgb(178, 183, 190),
    );

    let verdicts = [
        (COLOR_SET, "notes", label_of(ranking.set_pick)),
        (COLOR_PROFILE, "tonal", label_of(ranking.profile_pick)),
        (
            COLOR_ROOT,
            "root",
            SPIRAL_PITCH_LABELS[ranking.root_pick_pc].to_string(),
        ),
    ];
    let mut vy = rect.top() + 20.0;
    for (color, name, value) in &verdicts {
        painter.circle_filled(pos2(rect.left() + 4.0, vy + 6.0), 3.5, *color);
        painter.text(
            pos2(rect.left() + 14.0, vy),
            egui::Align2::LEFT_TOP,
            format!("{name}: {value}"),
            FontId::proportional(12.0),
            Color32::from_rgb(206, 200, 189),
        );
        vy += 17.0;
    }

    // --- Слитый рейтинг ---
    let list_top = vy + 8.0;
    let list_bottom = rect.bottom() - 24.0; // место под строку выбора
    painter.text(
        pos2(rect.left(), list_top - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "Blended ranking",
        FontId::proportional(12.0),
        Color32::from_rgb(150, 156, 164),
    );

    let shown = ranking.candidates.len().min(RANKING_ROWS);
    if shown == 0 {
        return;
    }
    let row_h = ((list_bottom - list_top) / shown as f32).clamp(28.0, 48.0);

    for (i, candidate) in ranking.candidates.iter().take(shown).enumerate() {
        let y = list_top + i as f32 * row_h;
        let row = Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), y + row_h - 4.0));
        let color = pitch_class_color(candidate.root_pc);

        // Фон-полоска слитой вероятности.
        let bar_w = row.width() * candidate.probability.clamp(0.0, 1.0);
        if bar_w > 1.0 {
            let bar = Rect::from_min_max(row.min, pos2(row.left() + bar_w, row.max.y));
            painter.rect_filled(
                bar,
                6.0,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38),
            );
        }
        if i == 0 {
            painter.rect_stroke(
                row,
                6.0,
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 150),
                ),
                egui::StrokeKind::Inside,
            );
        }

        // Имя + итоговый процент (верхняя строка ряда).
        painter.text(
            pos2(row.left() + 10.0, row.top() + 3.0),
            egui::Align2::LEFT_TOP,
            candidate.label(),
            FontId::proportional(13.0),
            Color32::from_rgb(222, 215, 203),
        );
        painter.text(
            pos2(row.right() - 10.0, row.top() + 3.0),
            egui::Align2::RIGHT_TOP,
            format!("{:.0}%", candidate.probability * 100.0),
            FontId::monospace(12.0),
            color,
        );

        // Разбивка по трём методам (нижняя строка ряда): три мини-бара S/P/R.
        draw_method_breakdown(painter, row, &candidate.scores);
    }

    // Где в рейтинге ручной выбор скейла.
    if let Some(index) = ranking
        .candidates
        .iter()
        .position(|c| c.root_pc == selected_root_pc && c.kind == selected_kind)
    {
        let candidate = &ranking.candidates[index];
        painter.text(
            pos2(rect.left(), rect.bottom() - 10.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "selected {} {} — #{} · {:.0}%",
                SPIRAL_PITCH_LABELS[selected_root_pc],
                selected_kind.label(),
                index + 1,
                candidate.probability * 100.0
            ),
            FontId::proportional(12.0),
            Color32::from_rgb(150, 156, 164),
        );
    }
}

/// Три мини-бара S/P/R в нижней части ряда — вклад каждого метода в этот кандидат.
fn draw_method_breakdown(painter: &egui::Painter, row: Rect, scores: &MethodScores) {
    let segments = [
        (COLOR_SET, scores.set),
        (COLOR_PROFILE, scores.profile),
        (COLOR_ROOT, scores.root),
    ];
    let seg_w = 30.0;
    let gap = 6.0;
    let total_w = seg_w * 3.0 + gap * 2.0;
    let bar_h = 4.0;
    let y = row.bottom() - bar_h - 3.0;
    let mut x = row.right() - total_w;

    for (color, value) in &segments {
        let track = Rect::from_min_size(pos2(x, y), vec2(seg_w, bar_h));
        painter.rect_filled(track, 2.0, Color32::from_rgb(52, 56, 63));
        let fill_w = seg_w * value.clamp(0.0, 1.0);
        if fill_w > 0.5 {
            painter.rect_filled(Rect::from_min_size(pos2(x, y), vec2(fill_w, bar_h)), 2.0, *color);
        }
        x += seg_w + gap;
    }
}
