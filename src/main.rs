#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use fretboard;

use eframe::egui::{Color32, Context, FontId, Rangef, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use eframe::{CreationContext, Frame, NativeOptions, egui};
use fretboard::core_types::note::{ANote, Accidental, Note};
use fretboard::core_types::pitch::PCNote;
use fretboard::core_types::scale::Scale;
use fretboard::core_types::tuning::{Fret, Tuning};
use fretboard::fretboard::{FretConfig, Fretboard, fret_position_log_range};
use fretboard::ui::draw_fretboard::{draw_fret_lines, draw_fretboard, draw_string_lines};

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
        ctx.all_styles_mut(|style| {
            style.spacing.button_padding = egui::vec2(20.0, 10.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // let tuning = Tuning::standart();
            let tuning = Tuning::minor_thirds(ANote::parse("D2").to_pitch());

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
                fret_range_show: Fret(1)..Fret(19),
            };

            // border
            painter.rect_stroke(
                fretboard_rect,
                0.0,
                Stroke::new(1.0, Color32::RED),
                egui::StrokeKind::Inside,
            );

            draw_fret_lines(&painter, fretboard_rect, &fretboard);

            draw_string_lines(&painter, fretboard_rect, &fretboard);

            draw_fretboard(painter, fretboard);
        });
    }
}

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
