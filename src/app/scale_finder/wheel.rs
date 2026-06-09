//! Левая колонка панели: 12-спицевое ХРОМАТИЧЕСКОЕ колесо (энергия chroma + ноты
//! топ-кандидата) сверху и кольцо КВИНТ метода D со стрелкой к центру тяжести снизу.

use eframe::egui::{
    self,
    Color32,
    FontId,
    Rect,
    Stroke,
    pos2,
    vec2,
};

use super::{
    COLOR_SPIRAL,
    Ranking,
};
use crate::core_types::pitch::PCNote;
use crate::core_types::scale_detect::PITCH_CLASS_COUNT;
use crate::ui::snail::{
    SPIRAL_PITCH_LABELS,
    pitch_class_angle,
    pitch_class_color,
};

/// Кольцо КВИНТ (метод D): 12 нот в квинтовом порядке + стрелка к центру тяжести
/// chroma. Длина стрелки = сила тонального центра, ближайшая нота = его тоника.
pub(super) fn draw_fifths_ring(painter: &egui::Painter, rect: Rect, ranking: &Ranking) {
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

    painter.circle_stroke(
        center,
        radius,
        Stroke::new(1.0_f32, Color32::from_rgb(59, 64, 72)),
    );

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
pub(super) fn draw_chroma_wheel(painter: &egui::Painter, rect: Rect, ranking: &Ranking) {
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
