#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    Color32,
    Context,
    FontId,
    Rangef,
    Rect,
    Sense,
    Stroke,
    Ui,
    Vec2,
    pos2,
    vec2,
};
use eframe::{
    CreationContext,
    Frame,
    NativeOptions,
    egui,
};
use fretboard::core_types::chord::Chord;
use fretboard::core_types::note::{
    ANote,
    Accidental,
    Note,
};
use fretboard::core_types::pitch::{
    PCNote,
    PNote,
};
use fretboard::core_types::scale::Scale;
use fretboard::core_types::tuning::{
    Fret,
    Tuning,
};
use fretboard::fretboard::{
    FretConfig,
    Fretboard,
};

#[derive(serde::Deserialize)]
struct HotReloadMsg {
    jump_table: Option<subsecond::JumpTable>,
    for_pid:    Option<u32>,
}

#[derive(serde::Deserialize)]
enum DevserverMsg {
    HotReload(HotReloadMsg),
    #[serde(other)]
    Other,
}

fn connect_subsecond() {
    let Some(endpoint) = dioxus_cli_config::devserver_ws_endpoint() else {
        return;
    };

    std::thread::spawn(move || {
        let uri = format!(
            "{endpoint}?aslr_reference={}&build_id={}&pid={}",
            subsecond::aslr_reference(),
            dioxus_cli_config::build_id(),
            std::process::id()
        );

        let (mut websocket, _) = match tungstenite::connect(uri) {
            Ok(v) => v,
            Err(_) => return,
        };

        while let Ok(msg) = websocket.read() {
            if let tungstenite::Message::Text(text) = msg {
                if let Ok(DevserverMsg::HotReload(msg)) = serde_json::from_str(&text) {
                    if msg.for_pid == Some(std::process::id()) {
                        if let Some(jumptable) = msg.jump_table {
                            unsafe { subsecond::apply_patch(jumptable).unwrap() };
                        }
                    }
                }
            }
        }
    });
}

fn main() -> eframe::Result {
    connect_subsecond();

    subsecond::call(|| {
        eframe::run_native(
            "fretboard",
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

    fn subsecond_fn(&mut self, ui: &mut Ui) {
        ui.ctx().all_styles_mut(|style| {
            style.spacing.button_padding = egui::vec2(20.0, 10.0);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let tuning = Tuning::cello();
            let scale = Scale::blues_minor(PCNote::from_note(Note::A, Accidental::Natural));

            let avail_width = ui.available_width();
            let (component_rect, _resp) =
                ui.allocate_exact_size(vec2(avail_width, 260.0), Sense::click_and_drag());

            let painter = ui.painter_at(component_rect);

            let mut fretboard_rect = component_rect;

            // margin — оставляем место для скобок позиций сверху и снизу
            fretboard_rect.min.y += 60.;
            fretboard_rect.max.y -= 60.;
            fretboard_rect.max.x -= 20.;
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
            draw_string_lines(&painter, fretboard_rect, &fretboard, &scale);
            draw_fretboard(&painter, &fretboard, &scale);
            draw_positions(&painter, fretboard_rect, &fretboard);
        });
    }
}

// ── рисование (всё в tip crate для горячей подмены) ──

fn mark_scale_note(note: &PNote, scale: &Scale) -> Color32 {
    let (_, pc_note) = note.to_pc();

    match scale.degree(pc_note).map(|s| s.0) {
        Some(1) => Color32::RED,
        Some(5) => Color32::DARK_RED.gamma_multiply(1.2),
        Some(_) => Color32::YELLOW,
        None => Color32::GRAY,
    }
}

fn draw_fretboard(painter: &egui::Painter, fretboard: &Fretboard, scale: &Scale) {
    for string in fretboard.iter_strings() {
        for fret in fretboard.iter_frets() {
            let y = fretboard.string_pos(string);
            let x = fretboard.fret_pos(fret);

            let open = fretboard.tuning.note(string);
            let note = open.add(fret.semitones());
            let pos = pos2(x, y);

            painter.rect_filled(Rect::from_center_size(pos, vec2(30., 14.)), 8.0, Color32::BLACK);

            let color = mark_scale_note(&note, scale);

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

fn draw_string_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard, scale: &Scale) {
    for stringg in fretboard.iter_strings() {
        let y = fretboard.string_pos(stringg);
        let open = fretboard.tuning.note(stringg);

        let color = mark_scale_note(&open, scale);

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

fn draw_fret_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
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

struct Position {
    name:      &'static str,
    fret_from: usize, // первый лад включительно
    fret_to:   usize, // последний лад включительно
}

// позиции виолончели (перекрывающиеся)
fn cello_positions() -> Vec<Position> {
    vec![
        Position {
            name:      "1st",
            fret_from: 1,
            fret_to:   4,
        },
        Position {
            name:      "2nd",
            fret_from: 3,
            fret_to:   6,
        },
        Position {
            name:      "3rd",
            fret_from: 4,
            fret_to:   7,
        },
        Position {
            name:      "4th",
            fret_from: 6,
            fret_to:   9,
        },
    ]
}

fn draw_positions(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    let positions = cello_positions();

    for (i, pos) in positions.iter().enumerate() {
        let x_from = fretboard.fret_pos(Fret(pos.fret_from));
        let x_to = fretboard.fret_pos(Fret(pos.fret_to));

        // 1-я и 4-я позиции — выделяем
        let (color, thickness) = match pos.name {
            "1st" | "4th" => (Color32::from_rgba_unmultiplied(255, 100, 50, 200), 2.5),
            _ => (Color32::from_rgba_unmultiplied(150, 200, 255, 150), 1.5),
        };

        // все скобки сверху, каждая следующая дальше от грифа
        let bracket_offset = 28.0 + i as f32 * 14.0;
        let y = fretboard_rect.min.y - bracket_offset;

        // вертикальные линии от грифа до скобки
        let grip_edge = fretboard_rect.min.y;
        painter.line_segment(
            [pos2(x_from, grip_edge), pos2(x_from, y)],
            Stroke::new(thickness * 0.5, color.gamma_multiply(0.4)),
        );
        painter.line_segment(
            [pos2(x_to, grip_edge), pos2(x_to, y)],
            Stroke::new(thickness * 0.5, color.gamma_multiply(0.4)),
        );

        // горизонтальная линия
        painter.line_segment([pos2(x_from, y), pos2(x_to, y)], Stroke::new(thickness, color));
        // тики вниз (к грифу)
        let tick_len = 4.0;
        painter.line_segment(
            [pos2(x_from, y), pos2(x_from, y + tick_len)],
            Stroke::new(thickness, color),
        );
        painter.line_segment(
            [pos2(x_to, y), pos2(x_to, y + tick_len)],
            Stroke::new(thickness, color),
        );

        // название позиции
        let text_x = (x_from + x_to) / 2.0;
        let text_y = y - 2.0;
        let align = egui::Align2::CENTER_BOTTOM;
        painter.text(
            pos2(text_x, text_y),
            align,
            pos.name,
            FontId::monospace(10.0),
            color,
        );
    }
}

pub fn rangef_to_range(r: Rangef) -> Range<f32> {
    r.min..r.max
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        subsecond::call(|| {
            self.subsecond_fn(ui);
        });
    }
}
