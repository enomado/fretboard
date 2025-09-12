use crate::core_types::note::{ANote, Accidental, Note};
use crate::core_types::pitch::PCNote;
use crate::core_types::scale::Scale;
use crate::core_types::tuning::{Fret, Tuning};
use crate::fretboard::{FretConfig, Fretboard, fret_position_log_range};
use eframe::egui::{Color32, Context, FontId, Rangef, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use eframe::{CreationContext, Frame, NativeOptions, egui};

use std::ops::Range;
use std::sync::Arc;

pub fn draw_fretboard(painter: egui::Painter, fretboard: Fretboard) {
    let scale = Scale::minor(PCNote::from_note(Note::C, Accidental::Sharp));

    for string in fretboard.iter_strings() {
        for fret in fretboard.iter_frets() {
            //
            let y = fretboard.string_pos(string);
            let x = fretboard.fret_pos(fret);

            let open = fretboard.tuning.note(string);

            let note = open.add_interval(fret.semitones());

            let pos: egui::Pos2 = pos2(x, y);

            painter.rect_filled(
                Rect::from_center_size(pos, vec2(30., 14.)),
                8.0,
                Color32::BLACK,
            );

            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                note.name(),
                FontId::monospace(12.),
                Color32::RED,
            );
        }
    }
}

pub fn draw_string_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for stringg in fretboard.iter_strings() {
        let y = fretboard.string_pos(stringg);
        let open = fretboard.tuning.note(stringg);

        // open note
        painter.text(
            pos2(fretboard_rect.x_range().min - 26., y),
            egui::Align2::LEFT_CENTER,
            format!("{}", open.name()),
            FontId::monospace(12.0),
            Color32::YELLOW,
        );

        // string N
        painter.text(
            pos2(fretboard_rect.x_range().min - 46., y),
            egui::Align2::LEFT_CENTER,
            format!("{}", stringg.name()),
            FontId::monospace(12.0),
            Color32::YELLOW,
        );

        painter.hline(fretboard_rect.x_range(), y, (1.0, Color32::GREEN));
    }
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
