use eframe::egui::{
    self,
    Color32,
    FontId,
    Rect,
    Stroke,
    pos2,
};

use crate::core_types::tuning::Fret;
use crate::fretboard::Fretboard;

struct Position {
    name:      &'static str,
    fret_from: usize,
    fret_to:   usize,
}

fn cello_positions() -> [Position; 4] {
    [
        Position {
            name:      "1st",
            fret_from: 2,
            fret_to:   5,
        },
        Position {
            name:      "2nd",
            fret_from: 4,
            fret_to:   7,
        },
        Position {
            name:      "3rd",
            fret_from: 5,
            fret_to:   8,
        },
        Position {
            name:      "4th",
            fret_from: 7,
            fret_to:   10,
        },
    ]
}

pub fn draw_positions(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for (index, position) in cello_positions().iter().enumerate() {
        let x_from = fretboard.fret_pos(Fret(position.fret_from));
        let x_to = fretboard.fret_pos(Fret(position.fret_to));

        let (color, thickness) = match position.name {
            "1st" | "4th" => (Color32::from_rgba_unmultiplied(255, 140, 102, 210), 2.5),
            _ => (Color32::from_rgba_unmultiplied(164, 210, 255, 150), 1.5),
        };

        let bracket_offset = 18.0 + index as f32 * 20.0;
        let y = fretboard_rect.min.y - bracket_offset;

        painter.line_segment(
            [pos2(x_from, fretboard_rect.min.y), pos2(x_from, y)],
            Stroke::new(thickness * 0.5, color.gamma_multiply(0.4)),
        );
        painter.line_segment(
            [pos2(x_to, fretboard_rect.min.y), pos2(x_to, y)],
            Stroke::new(thickness * 0.5, color.gamma_multiply(0.4)),
        );
        painter.line_segment([pos2(x_from, y), pos2(x_to, y)], Stroke::new(thickness, color));

        let tick_length = 4.0;
        painter.line_segment(
            [pos2(x_from, y), pos2(x_from, y + tick_length)],
            Stroke::new(thickness, color),
        );
        painter.line_segment(
            [pos2(x_to, y), pos2(x_to, y + tick_length)],
            Stroke::new(thickness, color),
        );

        painter.text(
            pos2((x_from + x_to) / 2.0, y - 2.0),
            egui::Align2::CENTER_BOTTOM,
            position.name,
            FontId::monospace(10.0),
            color,
        );
    }
}
