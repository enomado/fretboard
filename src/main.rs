#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;

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
    fret_position_log_range,
};
use fretboard::ui::draw_fretboard::{
    Mark,
    draw_fret_lines,
    draw_fretboard,
    draw_fretboard_scale,
    draw_string_lines,
    draw_string_lines_scale,
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

    // инициализируем OnceLock до первого патча — запоминаем оригинальные адреса из бинарника
    DRAW_FRET_LINES_PTR.get_or_init(|| draw_fret_lines as *const () as u64);
    DRAW_STRING_LINES_PTR.get_or_init(|| draw_string_lines_scale as *const () as u64);
    DRAW_FRETBOARD_PTR.get_or_init(|| draw_fretboard_scale as *const () as u64);

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
            // let tuning = Tuning::standart_e();
            // let tuning = Tuning::standard_from(ANote::parse("E2").to_pitch());
            // let tuning = Tuning::minor_thirds(ANote::parse("D2").to_pitch());
            let tuning = Tuning::cello();
            // let scale =
            // Scale::blues_minor_pentatonic(PCNote::from_note(Note::E, Accidental::Natural));

            let scale = Scale::blues_minor(PCNote::from_note(Note::A, Accidental::Natural));

            let avail_width = ui.available_width();
            let (component_rect, resp) =
                ui.allocate_exact_size(vec2(avail_width, 140.0), Sense::click_and_drag());

            let painter = ui.painter_at(component_rect);

            let mut fretboard_rect = component_rect;

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

            call_draw_fret_lines(&painter, fretboard_rect, &fretboard);

            call_draw_string_lines(&painter, fretboard_rect, &fretboard, &scale);

            call_draw_fretboard(painter, fretboard, &scale);
            // draw_fretboard(painter, fretboard, mark_some_chord);
        });
    }
}

pub struct MarkSomeChord;

impl Mark for &MarkSomeChord {
    fn mark(&self, note: &PNote) -> Color32 {
        mark_some_chord(note)
    }
}

fn mark_some_chord(note: &PNote) -> Color32 {
    let scale = Chord::diminished7(Note::A.to_pc());

    let (_, pc_note) = note.to_pc();

    match scale.degree(pc_note) {
        Some(1) => Color32::RED, // I ступень
        // Some(2) => Color32::DARK_RED, // любая другая ступень
        Some(_) => Color32::YELLOW, // любая другая ступень
        None => Color32::GRAY,      // нет в гамме
    }
}

pub fn rangef_to_range(r: Rangef) -> Range<f32> {
    r.min..r.max
}

// OnceLock для оригинальных адресов lib-функций (до первого патча).
// После патча GOT-указатели из .so невалидны для jump table,
// поэтому запоминаем адреса из оригинального бинарника.
static DRAW_FRET_LINES_PTR: OnceLock<u64> = OnceLock::new();
static DRAW_STRING_LINES_PTR: OnceLock<u64> = OnceLock::new();
static DRAW_FRETBOARD_PTR: OnceLock<u64> = OnceLock::new();

/// Вызов lib-функции через jump table lookup.
/// Если есть патч — берём новый адрес из таблицы, иначе вызываем оригинал.
/// catch_unwind: паника в lib крейте не убивает приложение.
fn call_draw_fret_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    let orig = *DRAW_FRET_LINES_PTR.get_or_init(|| draw_fret_lines as *const () as u64);
    let f: fn(&egui::Painter, Rect, &Fretboard) = unsafe {
        if let Some(jt) = subsecond::get_jump_table() {
            if let Some(&new_addr) = jt.map.get(&orig) {
                std::mem::transmute(new_addr)
            } else {
                draw_fret_lines
            }
        } else {
            draw_fret_lines
        }
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(painter, fretboard_rect, fretboard);
    }));
}

fn call_draw_string_lines(
    painter: &egui::Painter,
    fretboard_rect: Rect,
    fretboard: &Fretboard,
    scale: &Scale,
) {
    let orig = *DRAW_STRING_LINES_PTR.get_or_init(|| draw_string_lines_scale as *const () as u64);
    let f: fn(&egui::Painter, Rect, &Fretboard, &Scale) = unsafe {
        if let Some(jt) = subsecond::get_jump_table() {
            if let Some(&new_addr) = jt.map.get(&orig) {
                std::mem::transmute(new_addr)
            } else {
                draw_string_lines_scale
            }
        } else {
            draw_string_lines_scale
        }
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(painter, fretboard_rect, fretboard, scale);
    }));
}

fn call_draw_fretboard(painter: egui::Painter, fretboard: Fretboard, scale: &Scale) {
    let orig = *DRAW_FRETBOARD_PTR.get_or_init(|| draw_fretboard_scale as *const () as u64);
    let f: fn(egui::Painter, Fretboard, &Scale) = unsafe {
        if let Some(jt) = subsecond::get_jump_table() {
            if let Some(&new_addr) = jt.map.get(&orig) {
                std::mem::transmute(new_addr)
            } else {
                draw_fretboard_scale
            }
        } else {
            draw_fretboard_scale
        }
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(painter, fretboard, scale);
    }));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        subsecond::call(|| {
            self.subsecond_fn(ui);
        });
    }
}
