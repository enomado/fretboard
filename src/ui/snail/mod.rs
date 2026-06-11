//! The "snail": a logarithmic pitch spiral. Octaves wrap onto the same angle so
//! a pitch class always points the same direction; the radius grows by semitone.
//! Drawing is pure — it takes a `SpiralChart` snapshot plus the analysis settings
//! that shape contrast, and paints into the given `Ui`.

use eframe::egui::{
    self,
    Color32,
    FontId,
    Rect,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use crate::audio::AnalysisSettings;

/// Labels for the 12 pitch-class spokes, starting at C at the top (-π/2).
pub const SPIRAL_PITCH_LABELS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

/// One frame's worth of data to render on the spiral, plus the cosmetic strings.
/// Borrows everything — the caller owns the audio reading it is sliced from.
pub struct SpiralChart<'a> {
    pub title:             &'a str,
    pub subtitle:          &'a str,
    pub spectrum:          Option<&'a [f32]>,
    pub waterfall:         &'a [Vec<f32>],
    pub note_labels:       &'a [String],
    pub active_note:       Option<&'a str>,
    pub waiting_message:   &'a str,
    pub empty_message:     &'a str,
    pub active_note_label: &'a str,
}

/// Default min-height floor below which the desktop spiral viz gets cramped
/// (matches bank/waterfall). Mobile passes a smaller floor so the snail fits
/// the leftover screen height instead of overflowing past the bottom edge.
pub const SPIRAL_MIN_HEIGHT: f32 = 376.0;

pub fn draw_spiral_chart(ui: &mut Ui, chart: SpiralChart<'_>, settings: &AnalysisSettings) {
    // Desktop tiles are "rubbery": no upper bound, so a divider drag stretches
    // the snail to fill the pane.
    draw_spiral_chart_sized(ui, chart, settings, SPIRAL_MIN_HEIGHT, f32::INFINITY);
}

/// Draw the spiral, clamping its allocated height to `[min_height, max_height]`.
///
/// The snail is a circle, so on a width-constrained surface (a phone) the height
/// must be capped to the available width — otherwise filling the leftover screen
/// height (which can be unbounded in some host uis) blows the circle far past the
/// bottom edge. Desktop passes `INFINITY` for `max_height` to stay rubbery.
pub fn draw_spiral_chart_sized(
    ui: &mut Ui,
    chart: SpiralChart<'_>,
    settings: &AnalysisSettings,
    min_height: f32,
    max_height: f32,
) {
    let available_size = ui.available_size_before_wrap();
    // clamp() requires min <= max; guard the (degenerate) very-narrow case.
    let height = available_size.y.clamp(min_height, max_height.max(min_height));
    let desired_size = vec2(available_size.x, height);
    let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
    painter.rect_stroke(
        rect,
        18.0,
        Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
        egui::StrokeKind::Inside,
    );

    painter.text(
        pos2(rect.left() + 14.0, rect.top() + 12.0),
        egui::Align2::LEFT_TOP,
        chart.title,
        FontId::proportional(15.0),
        Color32::from_rgb(201, 195, 184),
    );
    painter.text(
        pos2(rect.right() - 14.0, rect.top() + 12.0),
        egui::Align2::RIGHT_TOP,
        chart.subtitle,
        FontId::proportional(12.0),
        Color32::from_rgb(152, 158, 165),
    );

    let viz_rect = Rect::from_min_max(
        pos2(rect.left() + 20.0, rect.top() + 44.0),
        pos2(rect.right() - 20.0, rect.bottom() - 20.0),
    );

    let Some(spectrum) = chart.spectrum else {
        painter.text(
            viz_rect.center(),
            egui::Align2::CENTER_CENTER,
            chart.waiting_message,
            FontId::proportional(13.0),
            Color32::from_rgb(139, 143, 149),
        );
        return;
    };

    if spectrum.is_empty() || chart.note_labels.is_empty() {
        painter.text(
            viz_rect.center(),
            egui::Align2::CENTER_CENTER,
            chart.empty_message,
            FontId::proportional(13.0),
            Color32::from_rgb(139, 143, 149),
        );
        return;
    }

    let square = viz_rect.width().min(viz_rect.height());
    let chart_rect = Rect::from_center_size(viz_rect.center(), vec2(square, square));
    let center = chart_rect.center();
    let inner_radius = square * 0.12;
    let outer_radius = square * 0.47;
    let semitone_count = chart.note_labels.len().max(1);
    let spiral_bin_count = spectrum.len().max(1);
    // The bins→semitone divisor must come from the length of the data actually
    // being drawn, not from a single per-frame count. A waterfall row can have
    // been captured at a different resolution than the live spectrum: toggling
    // Δφ reassign switches the resonator spiral between 5 and 8 bins/semitone
    // while the history keeps the older rows. A shared divisor would reinterpret
    // those stale rows at the wrong scale and splay them onto bogus radii.
    let live_bins_per_semitone = bins_per_semitone(spectrum.len(), semitone_count);
    let pitch_class_offset = chart
        .note_labels
        .first()
        .and_then(|label| note_label_pitch_class(label))
        .unwrap_or(0);
    let radius_step = if semitone_count > 1 {
        (outer_radius - inner_radius) / (semitone_count - 1) as f32
    } else {
        0.0
    };
    let active_index = chart
        .active_note
        .and_then(|note| chart.note_labels.iter().position(|label| label == note));

    painter.circle_filled(
        center,
        inner_radius * 0.82,
        Color32::from_rgba_unmultiplied(70, 106, 148, 26),
    );

    for ring in 0..=semitone_count.saturating_sub(1) / 12 {
        let radius = inner_radius + ring as f32 * radius_step * 12.0;
        painter.circle_stroke(
            center,
            radius.min(outer_radius),
            Stroke::new(1.0_f32, Color32::from_rgb(59, 64, 72)),
        );
    }

    for (pitch_class, pitch_label) in SPIRAL_PITCH_LABELS.iter().enumerate() {
        let angle = pitch_class_angle(pitch_class);
        let direction = vec2(angle.cos(), angle.sin());
        let label_pos = center + direction * (outer_radius + 20.0);
        let spoke_color = pitch_class_color(pitch_class);
        let spoke_stroke = if Some(pitch_class) == active_index.map(|index| (pitch_class_offset + index) % 12)
        {
            Stroke::new(1.6_f32, spoke_color)
        } else {
            Stroke::new(1.0_f32, Color32::from_rgb(55, 60, 67))
        };

        painter.line_segment(
            [
                center + direction * inner_radius * 0.58,
                center + direction * outer_radius,
            ],
            spoke_stroke,
        );
        painter.text(
            label_pos,
            egui::Align2::CENTER_CENTER,
            *pitch_label,
            FontId::proportional(18.0),
            spoke_color,
        );
    }

    let spiral_points: Vec<_> = (0..spiral_bin_count)
        .map(|index| {
            spiral_point_fractional(
                center,
                inner_radius,
                radius_step,
                index as f32 / live_bins_per_semitone,
                pitch_class_offset as f32,
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        spiral_points,
        Stroke::new(1.1_f32, Color32::from_rgb(76, 82, 90)),
    ));

    for (history_index, row) in chart.waterfall.iter().enumerate() {
        let age = history_index as f32 / chart.waterfall.len().max(1) as f32;
        // Per-row divisor: this row may pre-date a resolution change, so its bins
        // are scaled by its own length — not the live spectrum's.
        let row_bins_per_semitone = bins_per_semitone(row.len(), semitone_count);
        let strengths = spiral_contrast_strengths(row, settings);
        for (note_index, intensity) in strengths.iter().copied().enumerate() {
            if intensity <= 0.0 {
                continue;
            }

            let semitone_position = note_index as f32 / row_bins_per_semitone;
            let pitch_class = (pitch_class_offset + semitone_position.floor() as usize) % 12;
            let position = spiral_point_fractional(
                center,
                inner_radius,
                radius_step,
                semitone_position,
                pitch_class_offset as f32,
            );
            let glow = 1.8 + intensity * 6.0 * (0.45 + age * 0.40);
            painter.circle_filled(
                position,
                glow,
                spiral_note_color(pitch_class, intensity, 10 + (age * 28.0) as u8),
            );
        }
    }

    for (note_index, intensity) in spiral_contrast_strengths(spectrum, settings)
        .into_iter()
        .enumerate()
    {
        if intensity <= 0.0 {
            continue;
        }

        let semitone_position = note_index as f32 / live_bins_per_semitone;
        let pitch_class = (pitch_class_offset + semitone_position.floor() as usize) % 12;
        let position = spiral_point_fractional(
            center,
            inner_radius,
            radius_step,
            semitone_position,
            pitch_class_offset as f32,
        );
        let glow_radius = 3.0 + intensity * 8.0;
        let core_radius = 1.4 + intensity * 3.2;
        let color = pitch_class_color(pitch_class);

        painter.circle_filled(
            position,
            glow_radius,
            spiral_note_color(pitch_class, intensity, 28 + (intensity * 96.0) as u8),
        );
        painter.circle_filled(position, core_radius, color);
    }

    if let Some(active_index) = active_index {
        let active_position = spiral_point_fractional(
            center,
            inner_radius,
            radius_step,
            active_index as f32,
            pitch_class_offset as f32,
        );
        let active_color = pitch_class_color((pitch_class_offset + active_index) % 12);
        painter.circle_stroke(active_position, 11.0, Stroke::new(2.0_f32, active_color));
        painter.circle_stroke(
            active_position,
            17.0,
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(active_color.r(), active_color.g(), active_color.b(), 100),
            ),
        );
        painter.text(
            pos2(rect.left() + 14.0, rect.bottom() - 14.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{} {}", chart.active_note_label, chart.note_labels[active_index]),
            FontId::proportional(12.0),
            Color32::from_rgb(214, 206, 192),
        );
    }
}

/// Angle of a pitch-class spoke. C sits at the top (-π/2) and the circle runs
/// clockwise through the 12 classes.
pub(crate) fn pitch_class_angle(pitch_class: usize) -> f32 {
    -std::f32::consts::FRAC_PI_2 + pitch_class as f32 * std::f32::consts::TAU / 12.0
}

/// Map a note label like "G#4"/"Bb5"/"C" to its pitch class 0..=11. Trailing
/// octave digits (and a leading-octave minus) are stripped before matching.
fn note_label_pitch_class(label: &str) -> Option<usize> {
    let note = label.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-');
    match note {
        "C" => Some(0),
        "C#" | "Db" => Some(1),
        "D" => Some(2),
        "D#" | "Eb" => Some(3),
        "E" => Some(4),
        "F" => Some(5),
        "F#" | "Gb" => Some(6),
        "G" => Some(7),
        "G#" | "Ab" => Some(8),
        "A" => Some(9),
        "A#" | "Bb" => Some(10),
        "B" => Some(11),
        _ => None,
    }
}

/// How many spectrum bins span one semitone, for a row of `bin_count` samples
/// laid across `semitone_count` note labels (one label per semitone). The grid
/// is `(semitone_count - 1)` semitones wide and carries `bin_count` samples, so
/// the density is `(bin_count - 1) / (semitone_count - 1)`.
///
/// Derived per drawn row rather than shared frame-wide: the resonator spiral
/// changes resolution (5 vs 8 bins/semitone) when Δφ reassign toggles, and the
/// waterfall history outlives that switch — each row must be scaled by its own
/// length or its energy lands on the wrong radius. Floored at 1.0 so a degenerate
/// row can never invert the bin→semitone mapping.
fn bins_per_semitone(bin_count: usize, semitone_count: usize) -> f32 {
    if semitone_count > 1 {
        (bin_count.max(1).saturating_sub(1) as f32 / (semitone_count - 1) as f32).max(1.0)
    } else {
        1.0
    }
}

/// Position on the spiral for a fractional semitone offset. `semitone_position`
/// is in semitones from the innermost label; `pitch_class_offset` rotates the
/// whole spiral so the first label lands on its true pitch-class angle.
fn spiral_point_fractional(
    center: egui::Pos2,
    inner_radius: f32,
    radius_step: f32,
    semitone_position: f32,
    pitch_class_offset: f32,
) -> egui::Pos2 {
    let angle = -std::f32::consts::FRAC_PI_2
        + (semitone_position + pitch_class_offset) * std::f32::consts::TAU / 12.0;
    let radius = inner_radius + semitone_position * radius_step;
    center + vec2(angle.cos(), angle.sin()) * radius
}

pub(crate) fn pitch_class_color(pitch_class: usize) -> Color32 {
    match pitch_class % 12 {
        0 => Color32::from_rgb(92, 230, 105),
        1 => Color32::from_rgb(104, 222, 170),
        2 => Color32::from_rgb(112, 204, 238),
        3 => Color32::from_rgb(122, 173, 255),
        4 => Color32::from_rgb(127, 138, 255),
        5 => Color32::from_rgb(164, 116, 246),
        6 => Color32::from_rgb(212, 98, 219),
        7 => Color32::from_rgb(236, 93, 168),
        8 => Color32::from_rgb(232, 110, 121),
        9 => Color32::from_rgb(239, 167, 102),
        10 => Color32::from_rgb(230, 203, 94),
        _ => Color32::from_rgb(156, 218, 115),
    }
}

fn spiral_note_color(pitch_class: usize, intensity: f32, alpha: u8) -> Color32 {
    let base = pitch_class_color(pitch_class);
    let glow = (40.0 + intensity * 120.0).round() as u8;
    Color32::from_rgba_unmultiplied(
        base.r().saturating_add(glow / 4),
        base.g().saturating_add(glow / 4),
        base.b().saturating_add(glow / 5),
        alpha,
    )
}

/// Adaptive thresholding so the spiral lights up local peaks rather than a wash
/// of noise. `note_gamma`/`note_spread` drive the emphasis curve and how sharply
/// neighbouring bins are suppressed.
fn spiral_contrast_strengths(values: &[f32], settings: &AnalysisSettings) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }

    let peak = values.iter().copied().fold(0.0, f32::max);
    if peak <= 0.0 {
        return vec![0.0; values.len()];
    }

    let mean = values.iter().copied().sum::<f32>() / values.len() as f32;
    let gamma_norm = normalize_setting(settings.note_gamma, 0.35, 1.2);
    let spread_norm = normalize_setting(settings.note_spread, 0.15, 0.8);
    let threshold_floor = lerp(0.025, 0.11, gamma_norm);
    let threshold_ceiling = lerp(0.22, 0.36, gamma_norm);
    let threshold =
        (mean * lerp(1.15, 1.95, spread_norm) + threshold_floor).clamp(threshold_floor, threshold_ceiling);
    let scale = (1.0 - threshold).max(0.001);
    let mut strengths = vec![0.0; values.len()];

    for index in 0..values.len() {
        let intensity = values[index].clamp(0.0, 1.0);
        let normalized = ((intensity - threshold) / scale).clamp(0.0, 1.0);
        if normalized <= 0.0 {
            continue;
        }

        let left = values[index.saturating_sub(1)].clamp(0.0, 1.0);
        let right = values[(index + 1).min(values.len() - 1)].clamp(0.0, 1.0);
        let neighbor = left.max(right);
        let is_local_peak = intensity >= left && intensity >= right;
        let neighbor_guard = lerp(0.96, 0.84, spread_norm);
        let ridge = ((intensity - neighbor * neighbor_guard) / scale).clamp(0.0, 1.0);
        let focus = if is_local_peak {
            lerp(0.48, 0.78, 1.0 - spread_norm) + ridge * lerp(0.28, 0.62, 1.0 - spread_norm)
        } else {
            ridge * lerp(0.04, 0.18, spread_norm)
        };
        let emphasis = lerp(1.55, 2.65, gamma_norm);
        let emphasized = normalized.powf(emphasis) * focus;

        if emphasized > lerp(0.012, 0.05, gamma_norm) {
            strengths[index] = emphasized;
        }
    }

    strengths
}

fn normalize_setting(value: f32, min: f32, max: f32) -> f32 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        bins_per_semitone,
        note_label_pitch_class,
    };

    #[test]
    fn note_label_pitch_class_reads_sharp_flat_and_natural_notes() {
        assert_eq!(note_label_pitch_class("C2"), Some(0));
        assert_eq!(note_label_pitch_class("F3"), Some(5));
        assert_eq!(note_label_pitch_class("G#4"), Some(8));
        assert_eq!(note_label_pitch_class("Bb5"), Some(10));
    }

    /// The divisor is recovered from the row's own length, so the two resolutions
    /// the resonator spiral switches between (5 and 8 bins/semitone, toggled by
    /// Δφ reassign) each report their true density across the same note labels.
    #[test]
    fn bins_per_semitone_reads_each_rows_own_resolution() {
        let semitone_count = 13; // 12 semitones between the innermost/outermost label
        assert_eq!(bins_per_semitone(12 * 5 + 1, semitone_count), 5.0);
        assert_eq!(bins_per_semitone(12 * 8 + 1, semitone_count), 8.0);
    }

    /// The crux of the toggle bug: a stale history row captured at one resolution
    /// must land on the same semitone (hence the same radius) as a fresh row at
    /// another resolution — because each is scaled by its own length, not a shared
    /// frame-wide divisor. The last bin of every full row is the top semitone.
    #[test]
    fn rows_of_different_resolution_map_to_the_same_semitone() {
        let semitone_count = 13;
        let span = (semitone_count - 1) as f32;

        let coarse_len = 12 * 5 + 1; // reassign off
        let fine_len = 12 * 8 + 1; // reassign on
        let coarse_top = (coarse_len - 1) as f32 / bins_per_semitone(coarse_len, semitone_count);
        let fine_top = (fine_len - 1) as f32 / bins_per_semitone(fine_len, semitone_count);

        assert_eq!(coarse_top, span);
        assert_eq!(fine_top, span);
    }

    /// Degenerate inputs never invert the mapping (divisor floored at 1.0).
    #[test]
    fn bins_per_semitone_is_floored_for_degenerate_rows() {
        assert_eq!(bins_per_semitone(0, 13), 1.0);
        assert_eq!(bins_per_semitone(1, 13), 1.0);
        assert_eq!(bins_per_semitone(99, 1), 1.0);
    }
}
