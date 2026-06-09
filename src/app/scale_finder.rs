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
    ScaleFinderConfig,
    TonalProfile,
};
use crate::ui::snail::{
    SPIRAL_PITCH_LABELS,
    pitch_class_angle,
    pitch_class_color,
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
        format!("{} {}", SPIRAL_PITCH_LABELS[self.root_pc], self.kind.label())
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
fn top_indices(candidates: &[ScaleCandidate], project: impl Fn(&ScaleCandidate) -> f32, n: usize) -> Vec<usize> {
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

/// Свернуть резонаторный снимок в chroma и проранжировать 12×N кандидатов всеми
/// тремя методами. `None` — нет резонаторных бинов или полная тишина.
fn rank(reading: &TunerReading, settings: &AnalysisSettings, config: ScaleFinderConfig) -> Option<Ranking> {
    if reading.resonator_spectrum.is_empty() {
        return None;
    }

    // Контракт банка (`resonator.rs`): бин i = MIDI `min_midi + i/bins_per_semitone`.
    let min_midi = settings.resonator.min_midi.as_u8() as usize;
    let bins = settings.resonator.bins;

    // Ширина окна: интегрируем только последние `window_frames` кадров истории
    // (хвост — новейшие кадры лежат в конце). Узкое окно — отзывчиво, широкое —
    // стабильно. И chroma, и устойчивость метода C считаются по этому же окну.
    let window = config.window_frames.max(1);
    let history = &reading.resonator_waterfall;
    let recent = &history[history.len().saturating_sub(window)..];

    let mean = scale_detect::mean_spectrum(recent, &reading.resonator_spectrum);
    let chroma = scale_detect::fold_chroma(&mean, min_midi, bins);
    let chroma_peak = chroma.iter().copied().fold(0.0, f32::max);
    if chroma_peak <= 0.0 {
        return None;
    }

    // Метод C готовится один раз: бас + устойчивость во времени → улика корня.
    let bass = scale_detect::fold_bass_chroma(&mean, min_midi, bins);
    let persist = scale_detect::persistence(recent, min_midi, bins, PROMINENCE_RATIO);
    let root_ev = scale_detect::root_evidence(&bass, &persist);
    // Метод D готовится один раз: центр тяжести chroma на круге квинт.
    let ce = scale_detect::center_of_effect(&chroma);
    let weights = config.weights;

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
            let spiral = scale_detect::spiral_proximity(&ce, &scale_detect::key_point(&scale));
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

    let probabilities = scale_detect::softmax_with_temperature(&blended_scores, SOFTMAX_TEMPERATURE);
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
        let reading = self.audio.reading();
        let settings = self.audio.analysis_settings();
        // Веса берём из конфига App. Слайдеры ниже меняют их для следующего кадра
        // (egui перерисовывается непрерывно — лаг в один кадр незаметен).
        let ranking = reading
            .as_ref()
            .and_then(|reading| rank(reading, &settings, self.scale_finder));
        let frame_ms = settings.resonator.update_ms;

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
                self.draw_scale_finder_controls(ui, frame_ms);
                ui.add_space(10.0);
                draw_scale_finder_body(ui, ranking.as_ref(), selected_root_pc, selected_kind);
            });
    }

    /// Слайдеры прямо в панели (конфиг живёт с панелью — панель самодостаточна):
    /// баланс трёх методов + ширина окна интеграции. `blended()` нормирует на
    /// сумму весов, поэтому каждый вес — относительный вклад, сумма к 1 не обязана.
    fn draw_scale_finder_controls(&mut self, ui: &mut Ui, frame_ms: u64) {
        let config = &mut self.scale_finder;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Method mix")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            weight_slider(ui, "notes", COLOR_SET, &mut config.weights.set);
            weight_slider(ui, "tonal", COLOR_PROFILE, &mut config.weights.profile);
            weight_slider(ui, "root", COLOR_ROOT, &mut config.weights.root);
            weight_slider(ui, "spiral", COLOR_SPIRAL, &mut config.weights.spiral);
            if ui.button("Reset").clicked() {
                config.weights = MethodWeights::default();
            }
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Window")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            ui.add_sized(
                [200.0, 18.0],
                egui::Slider::new(&mut config.window_frames, 1..=120)
                    .clamping(egui::SliderClamping::Always)
                    .trailing_fill(true)
                    .show_value(false),
            );
            // Кадры → секунды по интервалу обновления банка (resonator.update_ms).
            let seconds = config.window_frames as f32 * frame_ms as f32 / 1000.0;
            ui.label(
                RichText::new(format!("{} fr · ~{:.1}s", config.window_frames, seconds))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
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
    let fifths_rect = Rect::from_min_max(
        pos2(wheel_rect.left(), snail_rect.bottom() + 6.0),
        wheel_rect.max,
    );

    draw_chroma_wheel(&painter, snail_rect, ranking);
    draw_fifths_ring(&painter, fifths_rect, ranking);
    draw_method_panel(&painter, list_rect, ranking, selected_root_pc, selected_kind);
}

/// Кольцо КВИНТ (метод D): 12 нот в квинтовом порядке + стрелка к центру тяжести
/// chroma. Длина стрелки = сила тонального центра, ближайшая нота = его тоника.
fn draw_fifths_ring(painter: &egui::Painter, rect: Rect, ranking: &Ranking) {
    use std::f32::consts::FRAC_PI_2;

    let square = rect.width().min(rect.height());
    let chart = Rect::from_center_size(rect.center(), vec2(square, square));
    let center = chart.center();
    let radius = square * 0.32;

    painter.text(
        pos2(center.x, chart.top()),
        egui::Align2::CENTER_TOP,
        "circle of fifths · center of effect",
        FontId::proportional(10.0),
        Color32::from_rgb(150, 156, 164),
    );

    painter.circle_stroke(center, radius, Stroke::new(1.0_f32, Color32::from_rgb(59, 64, 72)));

    for pc in 0..PITCH_CLASS_COUNT {
        // Угол на круге квинт, C сверху: совпадает с координатами `fifths_point`,
        // повёрнутыми на -π/2 (там pc=C даёт направление +x, тут хотим вверх).
        let j = (7 * pc) % PITCH_CLASS_COUNT;
        let angle = -FRAC_PI_2 + j as f32 * std::f32::consts::TAU / PITCH_CLASS_COUNT as f32;
        let dir = vec2(angle.cos(), angle.sin());
        painter.text(
            center + dir * (radius + 12.0),
            egui::Align2::CENTER_CENTER,
            SPIRAL_PITCH_LABELS[pc],
            FontId::proportional(11.0),
            pitch_class_color(pc),
        );
    }

    // Центр тяжести: его направление в координатах круга квинт повёрнуто на -π/2.
    let ce = ranking.ce;
    let magnitude = (ce[0] * ce[0] + ce[1] * ce[1]).sqrt().min(1.0);
    if magnitude > 0.01 {
        let ce_angle = ce[1].atan2(ce[0]) - FRAC_PI_2;
        let dir = vec2(ce_angle.cos(), ce_angle.sin());
        let tip = center + dir * radius * magnitude;
        painter.line_segment([center, tip], Stroke::new(2.0_f32, COLOR_SPIRAL));
        painter.circle_filled(tip, 4.0, COLOR_SPIRAL);
    }
    painter.circle_filled(center, 2.0, Color32::from_rgb(120, 126, 134));
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
        "Three methods — each ranks on its own signal",
        FontId::proportional(12.0),
        Color32::from_rgb(178, 183, 190),
    );

    // Топ-вердикт каждого метода в его собственном цвете (см. цвета слайдеров).
    let set_top = top_indices(&ranking.candidates, |c| c.scores.set, 1)[0];
    let profile_top = top_indices(&ranking.candidates, |c| c.scores.profile, 1)[0];
    let spiral_top = top_indices(&ranking.candidates, |c| c.scores.spiral, 1)[0];
    let set_c = &ranking.candidates[set_top];
    let profile_c = &ranking.candidates[profile_top];
    let spiral_c = &ranking.candidates[spiral_top];

    // Метод C ранжирует КОРНИ, не скейлы — показываем три лучших корня-ноты.
    let mut roots: Vec<usize> = (0..PITCH_CLASS_COUNT).collect();
    roots.sort_by(|&a, &b| ranking.root_ev[b].total_cmp(&ranking.root_ev[a]));
    let root_value = format!(
        "{} · {} · {}",
        SPIRAL_PITCH_LABELS[roots[0]], SPIRAL_PITCH_LABELS[roots[1]], SPIRAL_PITCH_LABELS[roots[2]]
    );

    let mut y = rect.top() + 20.0;
    let verdict_h = 21.0;
    draw_verdict_row(painter, rect, y, COLOR_SET, "NOTES", &set_c.label(), set_c.scores.set);
    y += verdict_h;
    draw_verdict_row(painter, rect, y, COLOR_PROFILE, "TONAL", &profile_c.label(), profile_c.scores.profile);
    y += verdict_h;
    draw_verdict_row(painter, rect, y, COLOR_ROOT, "ROOT", &root_value, ranking.root_ev[roots[0]]);
    y += verdict_h;
    draw_verdict_row(painter, rect, y, COLOR_SPIRAL, "SPIRAL", &spiral_c.label(), spiral_c.scores.spiral);
    y += verdict_h;

    // --- Слитый результат ---
    y += 10.0;
    painter.text(
        pos2(rect.left(), y),
        egui::Align2::LEFT_TOP,
        "Blended result — left edge marks the leading method",
        FontId::proportional(12.0),
        Color32::from_rgb(150, 156, 164),
    );
    y += 18.0;

    let list_bottom = rect.bottom() - 22.0;
    let shown = ranking.candidates.len().min(RANKING_ROWS);
    if shown == 0 {
        return;
    }
    let row_h = ((list_bottom - y) / shown as f32).clamp(24.0, 40.0);

    for (i, candidate) in ranking.candidates.iter().take(shown).enumerate() {
        let ry = y + i as f32 * row_h;
        let row = Rect::from_min_max(pos2(rect.left() + 6.0, ry), pos2(rect.right(), ry + row_h - 4.0));
        let pc_color = pitch_class_color(candidate.root_pc);

        // Фон-полоска слитой вероятности (цвет ноты-корня).
        let bar_w = row.width() * candidate.probability.clamp(0.0, 1.0);
        if bar_w > 1.0 {
            let bar = Rect::from_min_max(row.min, pos2(row.left() + bar_w, row.max.y));
            painter.rect_filled(
                bar,
                6.0,
                Color32::from_rgba_unmultiplied(pc_color.r(), pc_color.g(), pc_color.b(), 38),
            );
        }
        if i == 0 {
            painter.rect_stroke(
                row,
                6.0,
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(pc_color.r(), pc_color.g(), pc_color.b(), 150),
                ),
                egui::StrokeKind::Inside,
            );
        }

        // Цветная метка слева — какой метод сильнее всего тянул этого кандидата.
        let tab = Rect::from_min_max(pos2(rect.left(), ry + 2.0), pos2(rect.left() + 3.0, ry + row_h - 6.0));
        painter.rect_filled(tab, 1.5, dominant_method_color(&candidate.scores));

        painter.text(
            pos2(row.left() + 8.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            candidate.label(),
            FontId::proportional(13.0),
            Color32::from_rgb(222, 215, 203),
        );
        painter.text(
            pos2(row.right() - 10.0, row.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.0}%", candidate.probability * 100.0),
            FontId::monospace(12.0),
            pc_color,
        );
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

/// Строка вердикта одного метода: цветной чип с именем + его лучший выбор + оценка.
fn draw_verdict_row(painter: &egui::Painter, rect: Rect, y: f32, color: Color32, name: &str, value: &str, score: f32) {
    let chip = Rect::from_min_size(pos2(rect.left(), y), vec2(52.0, 16.0));
    painter.rect_filled(
        chip,
        4.0,
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40),
    );
    painter.rect_stroke(chip, 4.0, Stroke::new(1.0_f32, color), egui::StrokeKind::Inside);
    painter.text(chip.center(), egui::Align2::CENTER_CENTER, name, FontId::proportional(10.0), color);

    painter.text(
        pos2(rect.left() + 60.0, y + 8.0),
        egui::Align2::LEFT_CENTER,
        value,
        FontId::proportional(12.0),
        Color32::from_rgb(220, 213, 201),
    );
    painter.text(
        pos2(rect.right(), y + 8.0),
        egui::Align2::RIGHT_CENTER,
        format!("{:.0}%", score * 100.0),
        FontId::monospace(11.0),
        color,
    );
}
