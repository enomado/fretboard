use eframe::egui;
use eframe::egui::{
    Color32,
    FontId,
    Rect,
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

            painter.rect_filled(Rect::from_center_size(pos, vec2(30., 14.)), 8.0, Color32::BLACK);

            let color = mark.mark(&note);

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
            Color32::YELLOW,
        );

        painter.hline(fretboard_rect.x_range(), y, (1.0, Color32::GREEN));
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

        painter.vline(x, fretboard_rect.y_range(), (1.0, Color32::GREEN));

        let color = if fret.0 == 12 {
            Color32::RED
        } else {
            Color32::YELLOW
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
