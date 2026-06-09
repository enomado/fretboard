//! Правая колонка панели: вердикты четырёх методов (каждый в своём цвете) и слитый
//! рейтинг кандидатов с разбивкой по доминирующему методу.

use eframe::egui::{
    self,
    Color32,
    FontId,
    Rect,
    Stroke,
    pos2,
    vec2,
};

use super::super::ScaleKind;
use super::{
    COLOR_PROFILE,
    COLOR_ROOT,
    COLOR_SET,
    COLOR_SPIRAL,
    RANKING_ROWS,
    Ranking,
    dominant_method_color,
    top_indices,
};
use crate::core_types::scale_detect::PITCH_CLASS_COUNT;
use crate::ui::snail::{
    SPIRAL_PITCH_LABELS,
    pitch_class_color,
};

/// Правая колонка: вердикты трёх методов + слитый рейтинг с разбивкой по методам.
pub(super) fn draw_method_panel(
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
    draw_verdict_row(
        painter,
        rect,
        y,
        COLOR_SET,
        "NOTES",
        &set_c.label(),
        set_c.scores.set,
    );
    y += verdict_h;
    draw_verdict_row(
        painter,
        rect,
        y,
        COLOR_PROFILE,
        "TONAL",
        &profile_c.label(),
        profile_c.scores.profile,
    );
    y += verdict_h;
    draw_verdict_row(
        painter,
        rect,
        y,
        COLOR_ROOT,
        "ROOT",
        &root_value,
        ranking.root_ev[roots[0]],
    );
    y += verdict_h;
    draw_verdict_row(
        painter,
        rect,
        y,
        COLOR_SPIRAL,
        "SPIRAL",
        &spiral_c.label(),
        spiral_c.scores.spiral,
    );
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
        let tab = Rect::from_min_max(
            pos2(rect.left(), ry + 2.0),
            pos2(rect.left() + 3.0, ry + row_h - 6.0),
        );
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
fn draw_verdict_row(
    painter: &egui::Painter,
    rect: Rect,
    y: f32,
    color: Color32,
    name: &str,
    value: &str,
    score: f32,
) {
    let chip = Rect::from_min_size(pos2(rect.left(), y), vec2(52.0, 16.0));
    painter.rect_filled(
        chip,
        4.0,
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40),
    );
    painter.rect_stroke(chip, 4.0, Stroke::new(1.0_f32, color), egui::StrokeKind::Inside);
    painter.text(
        chip.center(),
        egui::Align2::CENTER_CENTER,
        name,
        FontId::proportional(10.0),
        color,
    );

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
