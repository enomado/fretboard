use eframe::egui;
use eframe::egui::{
    Color32,
    FontId,
    Rect,
    Stroke,
    pos2,
    vec2,
};

use crate::core_types::pitch::PNote;
use crate::core_types::scale::Scale;
use crate::fretboard::Fretboard;

pub trait Mark {
    fn mark(&self, note: &PNote) -> Color32;
}

pub fn draw_fretboard<F>(painter: egui::Painter, fretboard: &Fretboard, mark: F)
where
    F: Mark,
{
    for string in fretboard.iter_strings() {
        for fret in fretboard.iter_frets() {
            //
            let y = fretboard.string_pos(string);
            let x = fretboard.fret_pos(fret);

            let open = fretboard.tuning.note(string);

            let note = open.add(fret.semitones());

            let pos: egui::Pos2 = pos2(x, y);

            let color = mark.mark(&note);
            let note_rect = Rect::from_center_size(pos, vec2(30.0, 16.0));

            painter.rect_filled(note_rect, 8.0, tinted_note_fill(color));
            painter.rect_stroke(
                note_rect,
                8.0,
                Stroke::new(1.0_f32, color.gamma_multiply(0.65)),
                egui::StrokeKind::Inside,
            );

            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                note.to_anote().name(),
                FontId::monospace(12.),
                color,
            );
        }
    }
}

pub fn draw_string_lines<M: Mark>(
    painter: &egui::Painter,
    fretboard_rect: Rect,
    fretboard: &Fretboard,
    mark: M,
) {
    for stringg in fretboard.iter_strings() {
        let y = fretboard.string_pos(stringg);
        let open = fretboard.tuning.note(stringg);

        let color = mark.mark(&open);

        // open note
        painter.text(
            pos2(fretboard_rect.x_range().min - 26., y),
            egui::Align2::LEFT_CENTER,
            open.to_anote().name(),
            FontId::monospace(12.0),
            color,
        );

        // string N
        painter.text(
            pos2(fretboard_rect.x_range().min - 46., y),
            egui::Align2::LEFT_CENTER,
            stringg.name(),
            FontId::monospace(12.0),
            Color32::from_rgb(214, 179, 110),
        );

        painter.hline(
            fretboard_rect.x_range(),
            y,
            (1.0, Color32::from_rgb(196, 200, 206).gamma_multiply(0.7)),
        );
    }
}

// не-generic обёртки для jump table hotpatch (generic fn pointer не coerce в HRTB fn ptr)
pub fn draw_fretboard_scale(painter: egui::Painter, fretboard: &Fretboard, scale: &Scale) {
    draw_fretboard(painter, fretboard, scale);
}

pub fn draw_string_lines_scale(
    painter: &egui::Painter,
    fretboard_rect: Rect,
    fretboard: &Fretboard,
    scale: &Scale,
) {
    draw_string_lines(painter, fretboard_rect, fretboard, scale);
}

pub fn draw_fret_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for fret in fretboard.iter_frets() {
        let x = fretboard.fret_pos(fret);

        let line_color = if is_octave_fret(fret.0) {
            Color32::from_rgb(206, 154, 92)
        } else {
            Color32::from_rgb(121, 94, 61)
        };
        painter.vline(x, fretboard_rect.y_range(), (1.0, line_color));

        if let Some(inlay_x) = inlay_center_x(fretboard, fret.0) {
            draw_inlay_marker(painter, fretboard_rect, inlay_x, fret.0);
        }

        let color = if fret.0 == 12 {
            Color32::from_rgb(255, 208, 160)
        } else {
            Color32::from_rgb(207, 181, 129)
        };

        painter.text(
            pos2(x, fretboard_rect.y_range().max + 4.),
            egui::Align2::CENTER_TOP,
            format!("{}", fret.0),
            FontId::monospace(12.0),
            color,
        );
    }
}

fn is_octave_fret(fret: usize) -> bool {
    matches!(fret, 12 | 24)
}

fn inlay_center_x(fretboard: &Fretboard, fret: usize) -> Option<f32> {
    let previous_fret = fret.checked_sub(1)?;
    let previous = fretboard.fret_pos(crate::core_types::tuning::Fret(previous_fret));
    let current = fretboard.fret_pos(crate::core_types::tuning::Fret(fret));

    Some((previous + current) * 0.5)
}

fn draw_inlay_marker(painter: &egui::Painter, fretboard_rect: Rect, x: f32, fret: usize) {
    if !matches!(fret, 3 | 5 | 7 | 9 | 12 | 15 | 17) {
        return;
    }

    let center_y = fretboard_rect.center().y;
    let fill = Color32::from_rgb(228, 211, 174).gamma_multiply(0.7);
    let radius = 4.0;

    if fret == 12 {
        painter.circle_filled(pos2(x, center_y - 18.0), radius, fill);
        painter.circle_filled(pos2(x, center_y + 18.0), radius, fill);
    } else {
        painter.circle_filled(pos2(x, center_y), radius, fill);
    }
}

fn tinted_note_fill(color: Color32) -> Color32 {
    let blend =
        |base: u8, accent: u8| -> u8 { ((base as f32 * 0.82) + (accent as f32 * 0.18)).round() as u8 };

    Color32::from_rgb(blend(23, color.r()), blend(19, color.g()), blend(17, color.b()))
}
