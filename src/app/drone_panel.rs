//! Drone-плеер — инструмент для занятий музыкой: тянет ноту/аккорд непрерывно,
//! пульсирует в такт или арпеджирует набор нот. В отличие от кнопки «Play test
//! note» в Controls (она лишь проверяет вывод), это полноценный голос: набор
//! нот, режимы, темп, тембр. Состояние живёт в аудио-движке ([`DroneState`]),
//! здесь — только отрисовка и правка (читаем → меняем → пишем целиком).

use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Stroke,
    Ui,
    vec2,
};

use super::App;
use crate::audio::{
    ArpPattern,
    DroneMode,
    DroneState,
};
use crate::core_types::pitch::PNote;
use crate::ui::theme::PANEL_FILL;

// Диапазон тумблер-клавиатуры: C2..=B5 (4 октавы) — покрывает строй гитары/виолы
// и удобные дрон-регистры, не разрастаясь по экрану.
const KEYBOARD_MIN_MIDI: u8 = 36; // C2
const KEYBOARD_MAX_MIDI: u8 = 83; // B5

const ACCENT_FILL: Color32 = Color32::from_rgb(112, 86, 72);
const ACCENT_STROKE: Color32 = Color32::from_rgb(207, 187, 166);
const IDLE_FILL: Color32 = Color32::from_rgb(42, 46, 52);
const IDLE_STROKE: Color32 = Color32::from_rgb(84, 89, 97);
const LABEL_COLOR: Color32 = Color32::from_rgb(205, 194, 176);
const VALUE_COLOR: Color32 = Color32::from_rgb(226, 216, 201);
const HINT_COLOR: Color32 = Color32::from_rgb(145, 151, 160);

impl App {
    pub(super) fn draw_drone_card(&mut self, ui: &mut Ui) {
        let frame_width = ui.available_width();

        // Источник истины — движок. Читаем снимок, правим локально, и если что-то
        // изменилось — пишем целиком обратно (колбэк подхватит без перезапуска).
        let mut drone = self.audio.drone_state();
        let mut changed = false;

        // Камертон держим в синхроне с настройками анализа: дрон должен звучать по
        // тому же строю, что и тюнер/резонатор.
        let reference_hz = self.audio.analysis_settings().concert_pitch_hz;
        if (drone.reference_hz - reference_hz).abs() > 1e-3 {
            drone.reference_hz = reference_hz;
            changed = true;
        }

        let playing = self.audio.drone_playing();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(18))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_width(frame_width - 32.0);

                // ── Транспорт: большая Play/Stop + сводка набора ──
                ui.horizontal(|ui| {
                    let (caption, fill, stroke) = if playing {
                        ("■ Stop", Color32::from_rgb(120, 58, 52), Color32::from_rgb(196, 122, 110))
                    } else {
                        ("▶ Play", Color32::from_rgb(42, 78, 72), Color32::from_rgb(111, 154, 142))
                    };
                    let button = egui::Button::new(RichText::new(caption).size(16.0).strong())
                        .min_size(vec2(132.0, 36.0))
                        .fill(fill)
                        .stroke(Stroke::new(1.0_f32, stroke))
                        .corner_radius(CornerRadius::same(16));
                    if ui.add(button).clicked() {
                        if playing {
                            self.audio.stop_drone();
                        } else {
                            self.audio.start_drone();
                        }
                    }

                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(if drone.notes.is_empty() {
                            "No notes selected".to_owned()
                        } else {
                            chord_summary(&drone.notes)
                        })
                        .color(VALUE_COLOR)
                        .monospace(),
                    );
                });

                ui.add_space(14.0);

                // ── Мастер-громкость ──
                changed |= slider_row(ui, "Volume", &mut drone.gain, 0.0..=1.0, |v| format!("{:>3.0}%", v * 100.0));

                // ── Тембр ──
                changed |= slider_row(ui, "Brightness", &mut drone.brightness, 0.0..=1.0, |v| {
                    format!("{:>3.0}%", v * 100.0)
                });

                ui.add_space(12.0);

                // ── Режим ──
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Mode").color(LABEL_COLOR).strong());
                    ui.add_space(6.0);
                    for (mode, name) in [
                        (DroneMode::Sustained, "Sustained"),
                        (DroneMode::Pulse, "Pulse"),
                        (DroneMode::Arp, "Arpeggio"),
                    ] {
                        if mode_button(ui, name, drone.mode == mode).clicked() {
                            drone.mode = mode;
                            changed = true;
                        }
                    }
                });

                // ── Параметры ритма (для Pulse/Arp) ──
                match drone.mode {
                    DroneMode::Sustained => {}
                    DroneMode::Pulse => {
                        ui.add_space(8.0);
                        changed |= slider_row(ui, "Tempo", &mut drone.bpm, 20.0..=300.0, |v| format!("{v:>3.0} bpm"));
                        changed |= slider_row(ui, "Duty", &mut drone.pulse_duty, 0.05..=0.95, |v| {
                            format!("{:>3.0}%", v * 100.0)
                        });
                    }
                    DroneMode::Arp => {
                        ui.add_space(8.0);
                        changed |= slider_row(ui, "Tempo", &mut drone.bpm, 20.0..=300.0, |v| format!("{v:>3.0} bpm"));
                        changed |= slider_row(ui, "Gate", &mut drone.arp_gate, 0.05..=1.0, |v| {
                            format!("{:>3.0}%", v * 100.0)
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Pattern").color(LABEL_COLOR).strong());
                            ui.add_space(6.0);
                            for (pattern, name) in [
                                (ArpPattern::Up, "Up"),
                                (ArpPattern::Down, "Down"),
                                (ArpPattern::UpDown, "Up/Down"),
                            ] {
                                if mode_button(ui, name, drone.arp_pattern == pattern).clicked() {
                                    drone.arp_pattern = pattern;
                                    changed = true;
                                }
                            }
                        });
                    }
                }

                ui.add_space(14.0);

                // ── Набор нот: тумблер-клавиатура + быстрые действия ──
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Notes").color(LABEL_COLOR).strong());
                    ui.add_space(6.0);
                    if quick_button(ui, "Clear").clicked() && !drone.notes.is_empty() {
                        drone.notes.clear();
                        changed = true;
                    }
                    // Достроить от самой низкой выбранной ноты — удобные интервалы
                    // для дрон-аккордов под занятия.
                    if let Some(&root) = drone.notes.first() {
                        if quick_button(ui, "+Octave").clicked() {
                            changed |= add_interval(&mut drone, root, 12);
                        }
                        if quick_button(ui, "+Fifth").clicked() {
                            changed |= add_interval(&mut drone, root, 7);
                        }
                        if quick_button(ui, "+Maj3").clicked() {
                            changed |= add_interval(&mut drone, root, 4);
                        }
                        if quick_button(ui, "+Min3").clicked() {
                            changed |= add_interval(&mut drone, root, 3);
                        }
                    }
                });

                ui.add_space(6.0);
                changed |= draw_keyboard(ui, &mut drone);

                ui.add_space(8.0);
                ui.label(
                    RichText::new("Click keys to toggle notes. Sustained holds the chord; Pulse beats it in time; Arpeggio walks the notes.")
                        .color(HINT_COLOR)
                        .size(12.0),
                );
            });

        if changed {
            self.audio.set_drone_state(drone);
        }
    }
}

/// Тумблер-клавиатура C2..=B5: ряд на октаву, чёрные клавиши приглушены,
/// выбранные подсвечены акцентом. Возвращает true, если набор изменился.
fn draw_keyboard(ui: &mut Ui, drone: &mut DroneState) -> bool {
    const BLACK_KEYS: [bool; 12] = [
        false, true, false, true, false, false, true, false, true, false, true, false,
    ];
    let mut changed = false;
    let mut midi = KEYBOARD_MIN_MIDI;
    while midi <= KEYBOARD_MAX_MIDI {
        ui.horizontal(|ui| {
            for _ in 0..12 {
                if midi > KEYBOARD_MAX_MIDI {
                    break;
                }
                let note = PNote::new(midi).unwrap();
                let selected = drone.notes.binary_search(&note).is_ok();
                let is_black = BLACK_KEYS[(midi % 12) as usize];
                let fill = if selected {
                    ACCENT_FILL
                } else if is_black {
                    Color32::from_rgb(30, 33, 38)
                } else {
                    Color32::from_rgb(48, 52, 58)
                };
                let stroke = if selected { ACCENT_STROKE } else { IDLE_STROKE };
                let text_color = if selected { VALUE_COLOR } else { Color32::from_rgb(188, 192, 198) };
                let button = egui::Button::new(RichText::new(note_name(midi)).size(11.0).color(text_color))
                    .min_size(vec2(34.0, 24.0))
                    .fill(fill)
                    .stroke(Stroke::new(1.0_f32, stroke))
                    .corner_radius(CornerRadius::same(6));
                if ui.add(button).clicked() {
                    drone.toggle_note(note);
                    changed = true;
                }
                midi += 1;
            }
        });
    }
    changed
}

/// Добавить ноту `root + semitones` в набор (если влезает и валидна по диапазону).
fn add_interval(drone: &mut DroneState, root: PNote, semitones: u8) -> bool {
    let target = root.as_u8().saturating_add(semitones);
    let Some(note) = PNote::new(target) else {
        return false;
    };
    if drone.notes.binary_search(&note).is_ok() {
        return false;
    }
    let before = drone.notes.len();
    drone.toggle_note(note);
    drone.notes.len() != before
}

/// Строка-слайдер «label … value» в едином стиле панели. Возвращает true при изменении.
fn slider_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    fmt: impl Fn(f32) -> String,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(LABEL_COLOR).strong());
        let slider = egui::Slider::new(value, range)
            .clamping(egui::SliderClamping::Always)
            .trailing_fill(true)
            .show_value(false);
        if ui.add_sized([200.0, 18.0], slider).changed() {
            changed = true;
        }
        ui.label(RichText::new(fmt(*value)).color(VALUE_COLOR).monospace());
    });
    changed
}

fn mode_button(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let button = egui::Button::new(label)
        .min_size(vec2(86.0, 26.0))
        .fill(if active { ACCENT_FILL } else { IDLE_FILL })
        .stroke(Stroke::new(1.0_f32, if active { ACCENT_STROKE } else { IDLE_STROKE }))
        .corner_radius(CornerRadius::same(13));
    ui.add(button)
}

fn quick_button(ui: &mut Ui, label: &str) -> egui::Response {
    let button = egui::Button::new(label)
        .min_size(vec2(64.0, 24.0))
        .fill(IDLE_FILL)
        .stroke(Stroke::new(1.0_f32, IDLE_STROKE))
        .corner_radius(CornerRadius::same(12));
    ui.add(button)
}

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

fn note_name(midi: u8) -> String {
    let octave = midi as i32 / 12 - 1;
    format!("{}{}", NOTE_NAMES[(midi % 12) as usize], octave)
}

/// Краткая сводка набора нот для строки транспорта, напр. «A2 · E3 · A3».
fn chord_summary(notes: &[PNote]) -> String {
    notes
        .iter()
        .map(|n| note_name(n.as_u8()))
        .collect::<Vec<_>>()
        .join(" · ")
}
