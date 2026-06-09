//! Waterfall (spectrogram) rendering: a stack of history rows painted top→bottom,
//! newest at the bottom, each cell tinted by magnitude and faded by age. Three
//! flavours share the same `draw_waterfall` core:
//!   - plain grid,
//!   - note-labelled (one label column per cell),
//!   - pitch-labelled (labels spaced by a fractional bins-per-label stride).
//! All functions are pure: they paint into a borrowed `Painter` over a `Rect`.

use eframe::egui::{
    self,
    Color32,
    FontId,
    Painter,
    Rect,
    Stroke,
    pos2,
};

/// Paint the magnitude grid. Rows are history (top = oldest), columns are bins.
pub fn draw_waterfall(painter: &Painter, rect: Rect, waterfall: &[Vec<f32>]) {
    if waterfall.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waterfall will fill as audio arrives",
            FontId::proportional(12.0),
            Color32::from_rgb(128, 133, 139),
        );
        return;
    }

    let rows = waterfall.len().max(1);
    let cols = waterfall[0].len().max(1);
    let cell_h = rect.height() / rows as f32;
    let cell_w = rect.width() / cols as f32;

    for (row_index, row) in waterfall.iter().enumerate() {
        for (col_index, value) in row.iter().enumerate() {
            let min = pos2(
                rect.left() + col_index as f32 * cell_w,
                rect.top() + row_index as f32 * cell_h,
            );
            // +0.5 overdraw stops hairline gaps between cells at fractional sizes.
            let max = pos2(min.x + cell_w + 0.5, min.y + cell_h + 0.5);
            painter.rect_filled(
                Rect::from_min_max(min, max),
                0.0,
                waterfall_color(*value, row_index as f32 / rows as f32),
            );
        }
    }
}

/// Waterfall with one label per cell (FFT note grid). Every `label_stride`-th
/// label is drawn; the active note gets a highlight column and a brighter label.
pub fn draw_note_waterfall(
    painter: &Painter,
    rect: Rect,
    waterfall: &[Vec<f32>],
    labels: &[String],
    active_note: Option<&str>,
) {
    draw_waterfall(painter, rect, waterfall);

    if labels.is_empty() {
        return;
    }

    let label_stride = 6usize;
    let cell_w = rect.width() / labels.len() as f32;
    let active_index = active_note.and_then(|note| labels.iter().position(|label| label == note));

    if let Some(index) = active_index {
        let x0 = rect.left() + index as f32 * cell_w;
        let x1 = x0 + cell_w;
        let center_x = (x0 + x1) * 0.5;
        painter.rect_filled(
            Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom())),
            0.0,
            Color32::from_rgba_unmultiplied(214, 200, 182, 24),
        );
        painter.line_segment(
            [pos2(center_x, rect.top()), pos2(center_x, rect.bottom())],
            Stroke::new(2.0_f32, Color32::from_rgb(214, 200, 182)),
        );
    }

    for index in (0..labels.len()).step_by(label_stride) {
        if Some(index) == active_index {
            continue;
        }

        let x = rect.left() + (index as f32 + 0.5) * cell_w;
        painter.text(
            pos2(x, rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            labels[index].as_str(),
            FontId::proportional(10.0),
            Color32::from_rgb(128, 133, 139),
        );
    }

    if let Some(index) = active_index {
        let x = rect.left() + (index as f32 + 0.5) * cell_w;
        painter.text(
            pos2(x, rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            labels[index].as_str(),
            FontId::proportional(10.0),
            Color32::from_rgb(228, 220, 208),
        );
    }
}

/// Waterfall where bins outnumber labels: labels sit `bins_per_label` bins apart
/// (the resonator bank packs several bins per semitone). Used by the resonator
/// bank and full-screen resonator waterfall.
pub fn draw_pitch_labeled_waterfall(
    painter: &Painter,
    rect: Rect,
    waterfall: &[Vec<f32>],
    labels: &[String],
    bins_per_label: f32,
    active_note: Option<&str>,
) {
    draw_waterfall(painter, rect, waterfall);

    if labels.is_empty() {
        return;
    }

    let total_bins = waterfall.first().map_or(0.0, |row| row.len() as f32).max(1.0);
    let bin_width = rect.width() / total_bins;
    let label_stride = 6usize;
    let active_index = active_note.and_then(|note| labels.iter().position(|label| label == note));

    if let Some(index) = active_index {
        let center_bin = index as f32 * bins_per_label;
        let x0 = rect.left() + (center_bin - bins_per_label * 0.5).max(0.0) * bin_width;
        let x1 = rect.left() + (center_bin + bins_per_label * 0.5).min(total_bins) * bin_width;
        let center_x = rect.left() + center_bin * bin_width;
        painter.rect_filled(
            Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom())),
            0.0,
            Color32::from_rgba_unmultiplied(214, 200, 182, 24),
        );
        painter.line_segment(
            [pos2(center_x, rect.top()), pos2(center_x, rect.bottom())],
            Stroke::new(2.0_f32, Color32::from_rgb(214, 200, 182)),
        );
    }

    for index in (0..labels.len()).step_by(label_stride) {
        if Some(index) == active_index {
            continue;
        }

        let x = rect.left() + index as f32 * bins_per_label * bin_width;
        painter.text(
            pos2(x, rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            labels[index].as_str(),
            FontId::proportional(10.0),
            Color32::from_rgb(128, 133, 139),
        );
    }

    if let Some(index) = active_index {
        let x = rect.left() + index as f32 * bins_per_label * bin_width;
        painter.text(
            pos2(x, rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            labels[index].as_str(),
            FontId::proportional(10.0),
            Color32::from_rgb(228, 220, 208),
        );
    }
}

/// Cell colour: warmer/brighter with magnitude, dimmed with age so older rows
/// recede. `age` is 0 (oldest) .. 1 (newest) — newer rows keep more saturation.
fn waterfall_color(value: f32, age: f32) -> Color32 {
    let intensity = value.clamp(0.0, 1.0);
    let fade = (0.35 + age * 0.65).clamp(0.0, 1.0);
    let r = (34.0 + intensity * 138.0 * fade).round() as u8;
    let g = (42.0 + intensity * 120.0 * fade).round() as u8;
    let b = (52.0 + intensity * 92.0 * fade).round() as u8;
    Color32::from_rgb(r, g, b)
}
