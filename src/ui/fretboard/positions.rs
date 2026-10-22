//! Overlay that brackets the classic cello hand positions (1st–4th) beside the
//! neck — above a horizontal one, right of a vertical one — so a cellist can read
//! which frets each position covers. A pure painter over a borrowed [`Fretboard`].

use eframe::egui::{
    self,
    Color32,
    FontId,
    Stroke,
    pos2,
};

use crate::core_types::tuning::Fret;
use crate::ui::fretboard::geometry::{
    Fretboard,
    NeckAxis,
};

struct Position {
    name:      &'static str,
    fret_from: Fret,
    fret_to:   Fret,
}

fn cello_positions() -> [Position; 4] {
    [
        Position {
            name:      "1st",
            fret_from: Fret(2),
            fret_to:   Fret(5),
        },
        Position {
            name:      "2nd",
            fret_from: Fret(4),
            fret_to:   Fret(7),
        },
        Position {
            name:      "3rd",
            fret_from: Fret(5),
            fret_to:   Fret(8),
        },
        Position {
            name:      "4th",
            fret_from: Fret(7),
            fret_to:   Fret(10),
        },
    ]
}

/// Offset of the first bracket from the neck's edge, and the step between
/// brackets, per axis. The vertical neck shares a phone's ~314 dp card with up to
/// six strings of 30 dp note pills, so its brackets pack tighter and carry their
/// labels at their start rather than in a column of their own.
fn stacking(axis: NeckAxis) -> (f32, f32) {
    match axis {
        NeckAxis::Horizontal => (18.0, 20.0),
        NeckAxis::Vertical => (VERTICAL_FIRST, VERTICAL_STEP),
    }
}
const VERTICAL_FIRST: f32 = 14.0;
const VERTICAL_STEP: f32 = 14.0;

/// How far the brackets reach out from a vertical neck: the outermost bracket
/// plus half its centred label. The caller leaves this much room on string 1's
/// side. (The horizontal card has its own fixed top margin for them.)
pub const VERTICAL_REACH: f32 = VERTICAL_FIRST + 3.0 * VERTICAL_STEP + 10.0;

pub fn draw_positions(painter: &egui::Painter, fretboard: &Fretboard) {
    let (first, step) = stacking(fretboard.axis());
    for (index, position) in cello_positions().iter().enumerate() {
        let from = fretboard.fret_pos(position.fret_from);
        let to = fretboard.fret_pos(position.fret_to);

        let (color, thickness) = match position.name {
            "1st" | "4th" => (Color32::from_rgba_unmultiplied(255, 140, 102, 210), 2.5_f32),
            _ => (Color32::from_rgba_unmultiplied(164, 210, 255, 150), 1.5_f32),
        };

        // Brackets stack outwards from the neck's edge on string 1's side.
        let edge = fretboard.outside(0.0);
        let offset = first + index as f32 * step;
        let out = fretboard.outside(offset);
        let tick_end = fretboard.outside(offset - 4.0);
        let at = |along, across| fretboard.point(along, across);

        painter.line_segment(
            [at(from, edge), at(from, out)],
            Stroke::new(thickness * 0.5_f32, color.gamma_multiply(0.4)),
        );
        painter.line_segment(
            [at(to, edge), at(to, out)],
            Stroke::new(thickness * 0.5_f32, color.gamma_multiply(0.4)),
        );
        painter.line_segment([at(from, out), at(to, out)], Stroke::new(thickness, color));

        painter.line_segment([at(from, out), at(from, tick_end)], Stroke::new(thickness, color));
        painter.line_segment([at(to, out), at(to, tick_end)], Stroke::new(thickness, color));

        // Horizontal: the label sits above its bracket, at the middle of its frets.
        // Vertical: centred on the bracket, just above where it starts — the
        // brackets start at different frets (2, 4, 5, 7), so the labels do not
        // collide, and no extra column of width is spent on them.
        let (label_at, align) = match fretboard.axis() {
            NeckAxis::Horizontal => (pos2((from + to) / 2.0, out - 2.0), egui::Align2::CENTER_BOTTOM),
            NeckAxis::Vertical => (pos2(out, from - 2.0), egui::Align2::CENTER_BOTTOM),
        };
        painter.text(label_at, align, position.name, FontId::monospace(10.0), color);
    }
}
