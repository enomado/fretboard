#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use egui_subsecond;

use eframe::egui::{Color32, Context, FontId, Rangef, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use eframe::{CreationContext, Frame, NativeOptions, egui};
use egui_subsecond::fretboard::{FretConfig, Fretboard};
use egui_subsecond::tuning::{Fret, GString, Tuning};
use egui_subsecond::types::ANote;
use std::ops::Range;
use std::sync::Arc;

fn main() -> eframe::Result {
    dioxus_devtools::connect_subsecond();

    subsecond::call(|| {
        eframe::run_native(
            "subsecond example",
            NativeOptions::default(),
            Box::new(|cc| Ok(Box::new(App::new(cc)))),
        )
    })
}

struct App {}

impl App {
    fn new(cc: &CreationContext) -> Self {
        {
            let ctx = cc.egui_ctx.clone();

            ctx.set_pixels_per_point(1.75);

            subsecond::register_handler(Arc::new(move || ctx.request_repaint()));
        }
        Self {}
    }

    fn subsecond_fn(&mut self, ctx: &Context) {
        subsecond::call(|| {
            ctx.all_styles_mut(|style| {
                style.spacing.button_padding = egui::vec2(20.0, 10.0);
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                // let tuning = Tuning::standart();
                let tuning = Tuning::minor_thirds(ANote::parse("D2"));

                let avail_width = ui.available_width();
                let (component_rect, resp) =
                    ui.allocate_exact_size(vec2(avail_width, 140.0), Sense::click_and_drag());

                let painter = ui.painter_at(component_rect);

                let mut fretboard_rect = component_rect.clone();

                // margin
                fretboard_rect.max.x -= 20.;
                fretboard_rect.max.y -= 20.;
                fretboard_rect.min.x += 46.;

                let fretboard = Fretboard {
                    screen_size_x: rangef_to_range(fretboard_rect.x_range()),
                    screen_size_y: rangef_to_range(fretboard_rect.y_range()),
                    config: FretConfig::Log,
                    tuning,
                    fret_range_show: Fret(24),
                };

                // border
                painter.rect_stroke(
                    fretboard_rect,
                    0.0,
                    Stroke::new(1.0, Color32::RED),
                    egui::StrokeKind::Inside,
                );

                draw_frets(&painter, fretboard_rect, &fretboard);

                draw_strings(&painter, fretboard_rect, &fretboard);

                draw_fretboard(painter, fretboard);
            });
        });
    }
}

fn draw_fretboard(painter: egui::Painter, fretboard: Fretboard) {
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

fn draw_strings(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
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

fn draw_frets(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for fret in fretboard.iter_frets() {
        let x = fretboard.fret_pos(fret);

        painter.vline(x, fretboard_rect.y_range(), (1.0, Color32::GREEN));

        painter.text(
            pos2(x, fretboard_rect.y_range().max + 4.),
            egui::Align2::CENTER_TOP,
            format!("{}", fret.0),
            FontId::monospace(12.0),
            Color32::YELLOW,
        );
    }
}

// или вручную
pub fn rangef_to_range(r: Rangef) -> Range<f32> {
    r.min..r.max
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, frame: &mut Frame) {
        subsecond::call(|| {
            self.subsecond_fn(ctx);
        });
    }
}
