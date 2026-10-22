//! Renderers for the fretboard itself: note marks per string/fret, the open
//! string lines and labels, fret lines with their inlay dots, and the [`Mark`]
//! trait that lets a caller colour each note (a [`Scale`] implements it). All
//! functions are pure painters — they read positions from a borrowed
//! [`Fretboard`] and own no state.

use eframe::egui;
use eframe::egui::{
    Align2,
    Color32,
    FontId,
    Rect,
    Stroke,
    Vec2,
    pos2,
    vec2,
};

use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::PNote;
use crate::core_types::scale::Scale;
use crate::core_types::tuning::Fret;
use crate::ui::fretboard::geometry::{
    Fretboard,
    NeckAxis,
};

/// The pill every note on the neck is drawn in. A neck whose frets or strings sit
/// closer than this draws its notes on top of each other.
pub const NOTE_PILL: Vec2 = vec2(30.0, 16.0);

pub trait Mark {
    fn mark(&self, note: &PNote) -> Color32;
}

impl Mark for &Scale {
    fn mark(&self, note: &PNote) -> Color32 {
        mark_some_scale(note, self)
    }
}

fn mark_some_scale(note: &PNote, scale: &Scale) -> Color32 {
    let (_, pc_note) = note.to_pc();

    match scale.degree(pc_note).map(|s| s.0) {
        Some(1) => Color32::from_rgb(210, 166, 136), // I ступень
        Some(5) => Color32::from_rgb(184, 146, 115), // квинта
        Some(_) => Color32::from_rgb(198, 188, 145), // прочие ступени
        None => Color32::from_rgb(117, 120, 122),    // нет в гамме
    }
}

pub fn draw_fretboard<F>(painter: egui::Painter, fretboard: &Fretboard, mark: F, style: AccidentalStyle)
where
    F: Mark,
{
    for string in fretboard.iter_strings() {
        for fret in fretboard.iter_frets() {
            let pos = fretboard.note_center(string, fret);
            let note = fretboard.tuning.note(string).add(fret.semitones());

            let color = mark.mark(&note);
            let note_rect = Rect::from_center_size(pos, NOTE_PILL);

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
                note.to_anote().name_styled(style),
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
    style: AccidentalStyle,
) {
    for stringg in fretboard.iter_strings() {
        let across = fretboard.string_pos(stringg);
        let open = fretboard.tuning.note(stringg);

        let color = mark.mark(&open);

        // Before the nut: "string N" then the open note — to the left of a
        // horizontal neck, stacked above a vertical one.
        let (open_pos, name_pos, align) = match fretboard.axis() {
            NeckAxis::Horizontal => {
                (
                    pos2(fretboard_rect.left() - 26., across),
                    pos2(fretboard_rect.left() - 46., across),
                    Align2::LEFT_CENTER,
                )
            }
            NeckAxis::Vertical => {
                (
                    pos2(across, fretboard_rect.top() - 6.),
                    pos2(across, fretboard_rect.top() - 22.),
                    Align2::CENTER_BOTTOM,
                )
            }
        };
        painter.text(
            open_pos,
            align,
            open.to_anote().name_styled(style),
            FontId::monospace(12.0),
            color,
        );
        painter.text(
            name_pos,
            align,
            stringg.name(),
            FontId::monospace(12.0),
            Color32::from_rgb(214, 179, 110),
        );

        painter.line_segment(
            fretboard.string_line(stringg),
            (1.0, Color32::from_rgb(196, 200, 206).gamma_multiply(0.7)),
        );
    }
}

// не-generic обёртки для jump table hotpatch (generic fn pointer не coerce в HRTB fn ptr)
pub fn draw_fretboard_scale(
    painter: egui::Painter,
    fretboard: &Fretboard,
    scale: &Scale,
    style: AccidentalStyle,
) {
    draw_fretboard(painter, fretboard, scale, style);
}

pub fn draw_string_lines_scale(
    painter: &egui::Painter,
    fretboard_rect: Rect,
    fretboard: &Fretboard,
    scale: &Scale,
    style: AccidentalStyle,
) {
    draw_string_lines(painter, fretboard_rect, fretboard, scale, style);
}

pub fn draw_fret_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for fret in fretboard.iter_frets() {
        let along = fretboard.fret_pos(fret);

        let line_color = if is_octave_fret(fret.0) {
            Color32::from_rgb(206, 154, 92)
        } else {
            Color32::from_rgb(121, 94, 61)
        };
        painter.line_segment(fretboard.fret_line(fret), (1.0, line_color));

        if let Some(inlay_along) = inlay_center(fretboard, fret.0) {
            draw_inlay_marker(painter, fretboard, inlay_along, fret.0);
        }

        let color = if fret.0 == 12 {
            Color32::from_rgb(255, 208, 160)
        } else {
            Color32::from_rgb(207, 181, 129)
        };

        // Fret numbers on the side away from the position brackets: under a
        // horizontal neck, left of a vertical one.
        let (pos, align) = match fretboard.axis() {
            NeckAxis::Horizontal => (pos2(along, fretboard_rect.bottom() + 4.), Align2::CENTER_TOP),
            NeckAxis::Vertical => (pos2(fretboard_rect.left() - 6., along), Align2::RIGHT_CENTER),
        };
        painter.text(pos, align, format!("{}", fret.0), FontId::monospace(12.0), color);
    }
}

fn is_octave_fret(fret: usize) -> bool {
    matches!(fret, 12 | 24)
}

/// Along-the-neck midpoint between fret `fret` and the one before it.
fn inlay_center(fretboard: &Fretboard, fret: usize) -> Option<f32> {
    let previous_fret = fret.checked_sub(1)?;
    let previous = fretboard.fret_pos(Fret(previous_fret));
    let current = fretboard.fret_pos(Fret(fret));

    Some((previous + current) * 0.5)
}

fn draw_inlay_marker(painter: &egui::Painter, fretboard: &Fretboard, along: f32, fret: usize) {
    if !matches!(fret, 3 | 5 | 7 | 9 | 12 | 15 | 17) {
        return;
    }

    let across = fretboard.across_center();
    let fill = Color32::from_rgb(228, 211, 174).gamma_multiply(0.7);
    let radius = 4.0;

    if fret == 12 {
        painter.circle_filled(fretboard.point(along, across - 18.0), radius, fill);
        painter.circle_filled(fretboard.point(along, across + 18.0), radius, fill);
    } else {
        painter.circle_filled(fretboard.point(along, across), radius, fill);
    }
}

fn tinted_note_fill(color: Color32) -> Color32 {
    let blend =
        |base: u8, accent: u8| -> u8 { ((base as f32 * 0.82) + (accent as f32 * 0.18)).round() as u8 };

    Color32::from_rgb(blend(23, color.r()), blend(19, color.g()), blend(17, color.b()))
}
