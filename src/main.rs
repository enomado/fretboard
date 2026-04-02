#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
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
    egui,
};
#[cfg(not(target_arch = "wasm32"))]
use eframe::NativeOptions;
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

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct HotReloadMsg {
    jump_table: Option<subsecond::JumpTable>,
    for_pid:    Option<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
enum DevserverMsg {
    HotReload(HotReloadMsg),
    #[serde(other)]
    Other,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("fretboard_canvas")
            .unwrap()
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(App::new(cc)))),
            )
            .await
            .unwrap();
    });
}

#[derive(Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
enum TuningKind {
    Cello,
    StandardE,
    MinorThirds,
}

impl TuningKind {
    fn label(&self) -> &'static str {
        match self {
            TuningKind::Cello => "Cello (C-G-D-A)",
            TuningKind::StandardE => "Guitar (E std)",
            TuningKind::MinorThirds => "Minor thirds",
        }
    }

    fn to_tuning(&self) -> Tuning {
        match self {
            TuningKind::Cello => Tuning::cello(),
            TuningKind::StandardE => Tuning::standart_e(),
            TuningKind::MinorThirds => Tuning::minor_thirds(ANote::parse("D2").to_pitch()),
        }
    }
}

const ALL_TUNINGS: &[TuningKind] = &[TuningKind::Cello, TuningKind::StandardE, TuningKind::MinorThirds];

#[derive(Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
enum ScaleKind {
    Major,
    Minor,
    BluesMinor,
    BluesMinorPentatonic,
    BluesMajor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
}

impl ScaleKind {
    fn label(&self) -> &'static str {
        match self {
            ScaleKind::Major => "Major",
            ScaleKind::Minor => "Minor",
            ScaleKind::BluesMinor => "Blues minor",
            ScaleKind::BluesMinorPentatonic => "Blues minor pent.",
            ScaleKind::BluesMajor => "Blues major",
            ScaleKind::Dorian => "Dorian",
            ScaleKind::Phrygian => "Phrygian",
            ScaleKind::Lydian => "Lydian",
            ScaleKind::Mixolydian => "Mixolydian",
            ScaleKind::Locrian => "Locrian",
        }
    }

    fn to_scale(&self, root: PCNote) -> Scale {
        match self {
            ScaleKind::Major => Scale::major(root),
            ScaleKind::Minor => Scale::minor(root),
            ScaleKind::BluesMinor => Scale::blues_minor(root),
            ScaleKind::BluesMinorPentatonic => Scale::blues_minor_pentatonic(root),
            ScaleKind::BluesMajor => Scale::blues_major(root),
            ScaleKind::Dorian => Scale::dorian(root),
            ScaleKind::Phrygian => Scale::phrygian(root),
            ScaleKind::Lydian => Scale::lydian(root),
            ScaleKind::Mixolydian => Scale::mixolydian(root),
            ScaleKind::Locrian => Scale::locrian(root),
        }
    }
}

const ALL_SCALES: &[ScaleKind] = &[
    ScaleKind::Major,
    ScaleKind::Minor,
    ScaleKind::BluesMinor,
    ScaleKind::BluesMinorPentatonic,
    ScaleKind::BluesMajor,
    ScaleKind::Dorian,
    ScaleKind::Phrygian,
    ScaleKind::Lydian,
    ScaleKind::Mixolydian,
    ScaleKind::Locrian,
];

#[derive(Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
enum ChordKind {
    Major,
    Minor,
    Dominant7,
    Major7,
    Minor7,
    HalfDiminished7,
    Diminished7,
}

impl ChordKind {
    fn label(&self) -> &'static str {
        match self {
            ChordKind::Major => "Major",
            ChordKind::Minor => "Minor",
            ChordKind::Dominant7 => "7",
            ChordKind::Major7 => "Maj7",
            ChordKind::Minor7 => "m7",
            ChordKind::HalfDiminished7 => "m7b5",
            ChordKind::Diminished7 => "dim7",
        }
    }

    fn to_chord(&self, root: PCNote) -> Chord {
        match self {
            ChordKind::Major => Chord::major(root),
            ChordKind::Minor => Chord::minor(root),
            ChordKind::Dominant7 => Chord::dominant7(root),
            ChordKind::Major7 => Chord::major7(root),
            ChordKind::Minor7 => Chord::minor7(root),
            ChordKind::HalfDiminished7 => Chord::half_diminished7(root),
            ChordKind::Diminished7 => Chord::diminished7(root),
        }
    }
}

const ALL_CHORDS: &[ChordKind] = &[
    ChordKind::Major,
    ChordKind::Minor,
    ChordKind::Dominant7,
    ChordKind::Major7,
    ChordKind::Minor7,
    ChordKind::HalfDiminished7,
    ChordKind::Diminished7,
];

#[derive(Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
enum Mode {
    Scale,
    Chord,
}

impl Mode {
    fn label(&self) -> &'static str {
        match self {
            Mode::Scale => "Scale",
            Mode::Chord => "Chord",
        }
    }
}

enum MusicElement {
    Scale(Scale),
    Chord(Chord),
}

const ALL_ROOTS: &[(Note, &str)] = &[
    (Note::C, "C"),
    (Note::D, "D"),
    (Note::E, "E"),
    (Note::F, "F"),
    (Note::G, "G"),
    (Note::A, "A"),
    (Note::B, "B"),
];

#[derive(serde::Deserialize, serde::Serialize)]
struct App {
    tuning_kind: TuningKind,
    mode:        Mode,
    scale_kind:  ScaleKind,
    chord_kind:  ChordKind,
    root_note:   Note,
}

impl App {
    fn new(cc: &CreationContext) -> Self {
        {
            let ctx = cc.egui_ctx.clone();

            #[cfg(not(target_arch = "wasm32"))]
            {
                ctx.set_pixels_per_point(1.75);
                subsecond::register_handler(Arc::new(move || ctx.request_repaint()));
            }
        }

        if let Some(storage) = &cc.storage {
            eframe::get_value::<App>(storage, eframe::APP_KEY).unwrap_or_else(|| Self::default())
        } else {
            Self::default()
        }
    }

    fn default() -> Self {
        Self {
            tuning_kind: TuningKind::Cello,
            mode:        Mode::Scale,
            scale_kind:  ScaleKind::BluesMinor,
            chord_kind:  ChordKind::Major,
            root_note:   Note::A,
        }
    }
}

    fn subsecond_fn(&mut self, ui: &mut Ui) {
        ui.ctx().all_styles_mut(|style| {
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // ── контролы ──
            ui.horizontal(|ui| {
                // tuning
                ui.label("Tuning:");
                egui::ComboBox::from_id_salt("tuning")
                    .selected_text(self.tuning_kind.label())
                    .show_ui(ui, |ui| {
                        for t in ALL_TUNINGS {
                            ui.selectable_value(&mut self.tuning_kind, *t, t.label());
                        }
                    });

                ui.separator();

                // mode
                ui.label("Mode:");
                egui::ComboBox::from_id_salt("mode")
                    .selected_text(self.mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.mode, Mode::Scale, Mode::Scale.label());
                        ui.selectable_value(&mut self.mode, Mode::Chord, Mode::Chord.label());
                    });

                ui.separator();

                // root note
                ui.label("Root:");
                for &(note, name) in ALL_ROOTS {
                    if ui
                        .selectable_label(
                            std::mem::discriminant(&self.root_note) == std::mem::discriminant(&note),
                            name,
                        )
                        .clicked()
                    {
                        self.root_note = note;
                    }
                }

                ui.separator();

                // scale or chord
                match self.mode {
                    Mode::Scale => {
                        ui.label("Scale:");
                        egui::ComboBox::from_id_salt("scale")
                            .selected_text(self.scale_kind.label())
                            .show_ui(ui, |ui| {
                                for s in ALL_SCALES {
                                    ui.selectable_value(&mut self.scale_kind, *s, s.label());
                                }
                            });
                    }
                    Mode::Chord => {
                        ui.label("Chord:");
                        egui::ComboBox::from_id_salt("chord")
                            .selected_text(self.chord_kind.label())
                            .show_ui(ui, |ui| {
                                for c in ALL_CHORDS {
                                    ui.selectable_value(&mut self.chord_kind, *c, c.label());
                                }
                            });
                    }
                }
            });

            ui.add_space(4.0);

            let tuning = self.tuning_kind.to_tuning();
            let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
            let music_element = match self.mode {
                Mode::Scale => MusicElement::Scale(self.scale_kind.to_scale(root_pc)),
                Mode::Chord => MusicElement::Chord(self.chord_kind.to_chord(root_pc)),
            };

            let avail_width = ui.available_width();
            let (component_rect, _resp) =
                ui.allocate_exact_size(vec2(avail_width, 300.0), Sense::click_and_drag());

            let painter = ui.painter_at(component_rect);

            let mut fretboard_rect = component_rect;

            // margin — оставляем место для скобок позиций сверху и снизу
            fretboard_rect.min.y += 110.;
            fretboard_rect.max.y -= 40.;
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
                Stroke::new(1.0, Color32::LIGHT_GRAY),
                egui::StrokeKind::Inside,
            );

            draw_fret_lines(&painter, fretboard_rect, &fretboard);
            draw_string_lines(ui, &painter, fretboard_rect, &fretboard, &music_element);
            draw_fretboard(ui, &painter, &fretboard, &music_element);
            draw_positions(&painter, fretboard_rect, &fretboard);
        });
    }
}

// ── рисование (всё в tip crate для горячей подмены) ──

fn mark_note(note: &PNote, element: &MusicElement) -> Color32 {
    let (_, pc_note) = note.to_pc();

    match element {
        MusicElement::Scale(scale) => match scale.degree(pc_note).map(|s| s.0) {
            Some(1) => Color32::from_rgb(220, 100, 100), // soft red
            Some(5) => Color32::from_rgb(180, 80, 80),   // soft dark red
            Some(_) => Color32::from_rgb(220, 200, 100), // soft yellow
            None => Color32::LIGHT_GRAY,
        },
        MusicElement::Chord(chord) => match chord.degree(pc_note) {
            Some(1) => Color32::from_rgb(220, 100, 100), // root soft red
            Some(3) => Color32::from_rgb(100, 150, 220), // third soft blue
            Some(5) => Color32::from_rgb(100, 180, 100), // fifth soft green
            Some(7) => Color32::from_rgb(220, 200, 100), // seventh soft yellow
            Some(_) => Color32::from_rgb(220, 150, 80),  // other soft orange
            None => Color32::LIGHT_GRAY,
        },
    }
}

fn draw_fretboard(ui: &mut Ui, painter: &egui::Painter, fretboard: &Fretboard, element: &MusicElement) {
    for string in fretboard.iter_strings() {
        for fret in fretboard.iter_frets() {
            let y = fretboard.string_pos(string);
            let x = fretboard.fret_pos(fret);

            let open = fretboard.tuning.note(string);
            let note = open.add(fret.semitones());
            let pos = pos2(x, y);

            let radius = 12.0;
            let rect = Rect::from_center_size(pos, vec2(radius * 2., radius * 2.));
            let response = ui.allocate_rect(rect, Sense::hover());

            painter.circle_filled(pos, radius, Color32::from_rgb(245, 245, 245));

            if response.hovered() {
                painter.circle_stroke(pos, radius + 4.0, Stroke::new(2.0, Color32::from_rgb(200, 200, 200)));
                response.on_hover_text(format!("Note: {}", note.to_anote().name()));
            }

            let color = mark_note(&note, element);

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

fn draw_string_lines(ui: &mut Ui, painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard, element: &MusicElement) {
    for stringg in fretboard.iter_strings() {
        let y = fretboard.string_pos(stringg);
        let open = fretboard.tuning.note(stringg);

        let color = mark_note(&open, element);

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
            Color32::DARK_GRAY,
        );

        painter.hline(fretboard_rect.x_range(), y, (1.0, Color32::LIGHT_GRAY));
    }
}

fn draw_fret_lines(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    for fret in fretboard.iter_frets() {
        let x = fretboard.fret_pos(fret);

        painter.vline(x, fretboard_rect.y_range(), (1.0, Color32::LIGHT_GRAY));

        let color = if fret.0 == 12 {
            Color32::from_rgb(100, 100, 100)
        } else {
            Color32::DARK_GRAY
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

// Позиции виолончели (lower position fingering system).
// Каждая позиция покрывает 4 полутона (1-й палец → 4-й палец).
// Между основными позициями есть промежуточные (half/upper/lower),
// но рисуем только основные.
//
// Полная таблица (лад = полутон от открытой струны):
//   Half Position:              лады 1–4
//   1st Position:               лады 2–5
//   Upper 1st / Lower 2nd:     лады 3–6
//   2nd Position:               лады 4–7
//   Upper 2nd:                  лады 5–8
//   3rd Position:               лады 5–8  (то же, другая аппликатура)
//   Upper 3rd / Lower 4th:     лады 6–9
//   4th Position:               лады 7–10
//   Upper 4th / Lower 5th:     лады 8–11
//   5th Position:               лады 9–12
//   6th Position:               лады 10–13
//   Upper 6th:                  лады 11–14
//   7th Position:               лады 12–15
//   Upper 7th:                  лады 13–16
fn cello_positions() -> Vec<Position> {
    vec![
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

fn draw_positions(painter: &egui::Painter, fretboard_rect: Rect, fretboard: &Fretboard) {
    let positions = cello_positions();

    for (i, pos) in positions.iter().enumerate() {
        let x_from = fretboard.fret_pos(Fret(pos.fret_from));
        let x_to = fretboard.fret_pos(Fret(pos.fret_to));

        // 1-я и 4-я позиции — выделяем
        let (color, thickness) = match pos.name {
            "1st" | "4th" => (Color32::from_rgba_unmultiplied(220, 150, 100, 180), 2.5),
            _ => (Color32::from_rgba_unmultiplied(180, 200, 220, 120), 1.5),
        };

        // все скобки сверху, каждая следующая дальше от грифа
        let bracket_offset = 16.0 + i as f32 * 20.0;
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
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        subsecond::call(|| {
            self.subsecond_fn(ui);
        });

        // save state
        if let Some(storage) = frame.storage_mut() {
            eframe::set_value(storage, eframe::APP_KEY, self);
        }

        #[cfg(target_arch = "wasm32")]
        self.subsecond_fn(ui);
    }
}
