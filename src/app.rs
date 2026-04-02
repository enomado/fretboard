use std::ops::Range;

use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    FontId,
    Frame,
    Margin,
    Rangef,
    Rect,
    RichText,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};
use eframe::{
    CreationContext,
    Frame as AppFrame,
};

use crate::audio::{
    AudioEngine,
    AudioStatus,
    TunerReading,
};
use crate::core_types::note::{
    ANote,
    Accidental,
    Note,
};
use crate::core_types::pitch::PCNote;
use crate::core_types::scale::Scale;
use crate::core_types::tuning::{
    Fret,
    Tuning,
};
use crate::fretboard::{
    FretConfig,
    Fretboard,
};
use crate::ui::theme::{
    PANEL_FILL,
    apply_theme,
    fretboard_fill,
};
use crate::ui::{
    draw_fret_lines,
    draw_fretboard_scale,
    draw_positions,
    draw_string_lines_scale,
};

const FRETBOARD_HEIGHT: f32 = 340.0;
const FRETBOARD_MARGIN_LEFT: f32 = 54.0;
const FRETBOARD_MARGIN_RIGHT: f32 = 24.0;
const FRETBOARD_MARGIN_TOP: f32 = 110.0;
const FRETBOARD_MARGIN_BOTTOM: f32 = 52.0;

#[derive(Clone, Copy, PartialEq)]
enum TuningKind {
    Cello,
    StandardE,
    MinorThirds,
}

impl TuningKind {
    fn label(self) -> &'static str {
        match self {
            Self::Cello => "Cello (C-G-D-A)",
            Self::StandardE => "Guitar (E std)",
            Self::MinorThirds => "Minor thirds",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Cello => "Compact orchestral layout",
            Self::StandardE => "Classic six-string tuning",
            Self::MinorThirds => "Symmetric fretboard geometry",
        }
    }

    fn to_tuning(self) -> Tuning {
        match self {
            Self::Cello => Tuning::cello(),
            Self::StandardE => Tuning::standart_e(),
            Self::MinorThirds => Tuning::minor_thirds(ANote::parse("D2").to_pitch()),
        }
    }
}

const ALL_TUNINGS: [TuningKind; 3] = [TuningKind::Cello, TuningKind::StandardE, TuningKind::MinorThirds];

#[derive(Clone, Copy, PartialEq)]
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
    fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::BluesMinor => "Blues minor",
            Self::BluesMinorPentatonic => "Blues minor pent.",
            Self::BluesMajor => "Blues major",
            Self::Dorian => "Dorian",
            Self::Phrygian => "Phrygian",
            Self::Lydian => "Lydian",
            Self::Mixolydian => "Mixolydian",
            Self::Locrian => "Locrian",
        }
    }

    fn to_scale(self, root: PCNote) -> Scale {
        match self {
            Self::Major => Scale::major(root),
            Self::Minor => Scale::minor(root),
            Self::BluesMinor => Scale::blues_minor(root),
            Self::BluesMinorPentatonic => Scale::blues_minor_pentatonic(root),
            Self::BluesMajor => Scale::blues_major(root),
            Self::Dorian => Scale::dorian(root),
            Self::Phrygian => Scale::phrygian(root),
            Self::Lydian => Scale::lydian(root),
            Self::Mixolydian => Scale::mixolydian(root),
            Self::Locrian => Scale::locrian(root),
        }
    }
}

const ALL_SCALES: [ScaleKind; 10] = [
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

const ALL_ROOTS: [(Note, &str); 7] = [
    (Note::C, "C"),
    (Note::D, "D"),
    (Note::E, "E"),
    (Note::F, "F"),
    (Note::G, "G"),
    (Note::A, "A"),
    (Note::B, "B"),
];

pub struct App {
    audio:       AudioEngine,
    tuning_kind: TuningKind,
    scale_kind:  ScaleKind,
    root_note:   Note,
}

struct HoveredNote {
    string:    usize,
    fret:      usize,
    note_name: String,
    degree:    Option<u8>,
    center:    egui::Pos2,
    rect:      Rect,
}

struct TunerTarget {
    string:       usize,
    fret:         usize,
    note_name:    String,
    frequency_hz: f32,
    cents:        f32,
    degree:       Option<u8>,
}

impl App {
    pub fn new(cc: &CreationContext) -> Self {
        apply_theme(&cc.egui_ctx);

        Self {
            audio:       AudioEngine::new(),
            tuning_kind: TuningKind::Cello,
            scale_kind:  ScaleKind::BluesMinor,
            root_note:   Note::A,
        }
    }

    fn render(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(16, 20, 25))
                    .inner_margin(Margin::same(18)),
            )
            .show_inside(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));
                self.draw_header(ui);
                ui.add_space(14.0);
                self.draw_controls(ui);
                ui.add_space(14.0);
                self.draw_tuner_card(ui);
                ui.add_space(14.0);
                self.draw_fretboard_card(ui);
            });
    }

    fn draw_header(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Fretboard Explorer")
                        .size(28.0)
                        .color(Color32::from_rgb(230, 223, 210))
                        .family(egui::FontFamily::Proportional),
                );
                ui.label(
                    RichText::new(format!(
                        "{} • {} • root {}",
                        self.tuning_kind.subtitle(),
                        self.scale_kind.label(),
                        self.root_label()
                    ))
                    .color(Color32::from_rgb(154, 160, 168)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(
                    ui,
                    "Muted",
                    Color32::from_rgb(152, 159, 168),
                    Color32::from_rgb(61, 67, 75),
                );
                pill(
                    ui,
                    "5th",
                    Color32::from_rgb(203, 182, 147),
                    Color32::from_rgb(72, 58, 47),
                );
                pill(
                    ui,
                    "Root",
                    Color32::from_rgb(214, 190, 168),
                    Color32::from_rgb(89, 64, 56),
                );
            });
        });
    }

    fn draw_controls(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(18))
            .stroke(Stroke::new(1.0, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Tuning")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    egui::ComboBox::from_id_salt("tuning")
                        .selected_text(self.tuning_kind.label())
                        .show_ui(ui, |ui| {
                            for tuning in ALL_TUNINGS {
                                ui.selectable_value(&mut self.tuning_kind, tuning, tuning.label());
                            }
                        });

                    ui.separator();

                    ui.label(
                        RichText::new("Root")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    for (note, label) in ALL_ROOTS {
                        let selected = self.root_note == note;
                        let button = egui::Button::new(label)
                            .min_size(vec2(30.0, 28.0))
                            .fill(if selected {
                                Color32::from_rgb(112, 86, 72)
                            } else {
                                Color32::from_rgb(42, 46, 52)
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if selected {
                                    Color32::from_rgb(207, 187, 166)
                                } else {
                                    Color32::from_rgb(84, 89, 97)
                                },
                            ))
                            .corner_radius(CornerRadius::same(14));

                        if ui.add(button).clicked() {
                            self.root_note = note;
                        }
                    }

                    ui.separator();

                    ui.label(
                        RichText::new("Scale")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    egui::ComboBox::from_id_salt("scale")
                        .selected_text(self.scale_kind.label())
                        .show_ui(ui, |ui| {
                            for scale in ALL_SCALES {
                                ui.selectable_value(&mut self.scale_kind, scale, scale.label());
                            }
                        });
                });
            });
    }

    fn draw_fretboard_card(&self, ui: &mut Ui) {
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_target = self.filtered_tuner_target(&tuning, &scale);

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Scale tones: {}", scale.notes().len()))
                            .color(Color32::from_rgb(143, 150, 160)),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Visible frets: {}-{}", 1, 18))
                            .color(Color32::from_rgb(143, 150, 160)),
                    );
                });

                ui.add_space(10.0);

                let avail_width = ui.available_width();
                let (component_rect, response) =
                    ui.allocate_exact_size(vec2(avail_width, FRETBOARD_HEIGHT), Sense::hover());

                let painter = ui.painter_at(component_rect);
                painter.rect_filled(component_rect, 20.0, fretboard_fill());

                let mut fretboard_rect = component_rect;
                fretboard_rect.min.x += FRETBOARD_MARGIN_LEFT;
                fretboard_rect.max.x -= FRETBOARD_MARGIN_RIGHT;
                fretboard_rect.min.y += FRETBOARD_MARGIN_TOP;
                fretboard_rect.max.y -= FRETBOARD_MARGIN_BOTTOM;

                painter.rect_stroke(
                    fretboard_rect,
                    18.0,
                    Stroke::new(1.0, Color32::from_rgb(112, 88, 66)),
                    egui::StrokeKind::Inside,
                );

                let fretboard = Fretboard {
                    screen_size_x: rangef_to_range(fretboard_rect.x_range()),
                    screen_size_y: rangef_to_range(fretboard_rect.y_range()),
                    config: FretConfig::Log,
                    tuning,
                    fret_range_show: Fret(1)..Fret(19),
                };

                draw_fret_lines(&painter, fretboard_rect, &fretboard);
                draw_string_lines_scale(&painter, fretboard_rect, &fretboard, &scale);
                draw_fretboard_scale(painter.clone(), &fretboard, &scale);
                draw_positions(&painter, fretboard_rect, &fretboard);
                if let Some(target) = tuner_target.as_ref() {
                    self.draw_tuner_target(&painter, &fretboard, target);
                }
                if let Some(pointer_pos) = response.hover_pos() {
                    if let Some(hovered) = self.hovered_note(pointer_pos, &fretboard, &scale) {
                        self.draw_hovered_note(&painter, component_rect, &hovered);
                    }
                }

                self.draw_footer_note(ui, component_rect);
            });
    }

    fn draw_tuner_card(&self, ui: &mut Ui) {
        let status = self.audio.status();
        let reading = self.audio.reading();
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let target = self.filtered_tuner_target(&tuning, &scale);

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Live tuner")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            RichText::new(audio_status_label(&status)).color(audio_status_color(&status)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = &reading {
                            pill(
                                ui,
                                &format!("clarity {:.2}", reading.clarity),
                                Color32::from_rgb(206, 198, 183),
                                Color32::from_rgb(64, 68, 73),
                            );
                        }
                    });
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    self.draw_tuner_meter(ui, target.as_ref());
                    ui.add_space(16.0);
                    self.draw_spectrum(
                        ui,
                        target.as_ref().map(|value| value as &TunerTarget),
                        reading.as_ref(),
                    );
                });
            });
    }

    fn draw_tuner_meter(&self, ui: &mut Ui, reading: Option<&TunerTarget>) {
        let desired_size = vec2(250.0, 120.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        let center_x = rect.center().x;
        let meter_y = rect.bottom() - 30.0;
        painter.line_segment(
            [
                pos2(rect.left() + 18.0, meter_y),
                pos2(rect.right() - 18.0, meter_y),
            ],
            Stroke::new(2.0, Color32::from_rgb(89, 92, 98)),
        );

        for cents in [-50.0_f32, -25.0, 0.0, 25.0, 50.0] {
            let x = egui::remap(cents, -50.0..=50.0, (rect.left() + 22.0)..=(rect.right() - 22.0));
            let height = if cents == 0.0 { 18.0 } else { 10.0 };
            painter.line_segment(
                [pos2(x, meter_y - height), pos2(x, meter_y + 2.0)],
                Stroke::new(1.0, Color32::from_rgb(117, 122, 128)),
            );
        }

        match reading {
            Some(reading) => {
                painter.text(
                    pos2(rect.left() + 18.0, rect.top() + 18.0),
                    egui::Align2::LEFT_TOP,
                    reading.note_name.as_str(),
                    FontId::proportional(30.0),
                    Color32::from_rgb(230, 223, 210),
                );
                painter.text(
                    pos2(rect.left() + 18.0, rect.top() + 54.0),
                    egui::Align2::LEFT_TOP,
                    format!("{:.1} Hz", reading.frequency_hz),
                    FontId::proportional(15.0),
                    Color32::from_rgb(162, 166, 172),
                );

                let cents = reading.cents.clamp(-50.0, 50.0);
                let needle_x = egui::remap(cents, -50.0..=50.0, (rect.left() + 22.0)..=(rect.right() - 22.0));
                painter.line_segment(
                    [pos2(needle_x, meter_y - 22.0), pos2(needle_x, meter_y + 4.0)],
                    Stroke::new(3.0, cents_color(cents)),
                );
                painter.circle_filled(pos2(needle_x, meter_y), 5.0, cents_color(cents));
                painter.text(
                    pos2(rect.right() - 18.0, rect.top() + 18.0),
                    egui::Align2::RIGHT_TOP,
                    format!("{:+.1} cents", reading.cents),
                    FontId::proportional(15.0),
                    cents_color(cents),
                );
                painter.text(
                    pos2(rect.right() - 18.0, rect.top() + 40.0),
                    egui::Align2::RIGHT_TOP,
                    format!("S{} • F{}", reading.string, reading.fret),
                    FontId::proportional(12.0),
                    Color32::from_rgb(160, 165, 171),
                );
            }
            None => {
                painter.text(
                    rect.center_top() + vec2(0.0, 20.0),
                    egui::Align2::CENTER_TOP,
                    "Waiting for pitch",
                    FontId::proportional(20.0),
                    Color32::from_rgb(188, 182, 171),
                );
                painter.text(
                    rect.center_top() + vec2(0.0, 50.0),
                    egui::Align2::CENTER_TOP,
                    "Play a single sustained note near the microphone",
                    FontId::proportional(13.0),
                    Color32::from_rgb(139, 143, 149),
                );
            }
        }

        painter.line_segment(
            [pos2(center_x, meter_y - 24.0), pos2(center_x, meter_y + 6.0)],
            Stroke::new(1.0, Color32::from_rgb(177, 167, 150)),
        );
    }

    fn draw_spectrum(&self, ui: &mut Ui, target: Option<&TunerTarget>, reading: Option<&TunerReading>) {
        let desired_size = vec2((ui.available_width()).max(280.0), 220.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        painter.text(
            pos2(rect.left() + 14.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "Spectrum + note waterfall",
            FontId::proportional(15.0),
            Color32::from_rgb(201, 195, 184),
        );

        if let Some(target) = target {
            painter.text(
                pos2(rect.right() - 14.0, rect.top() + 12.0),
                egui::Align2::RIGHT_TOP,
                format!(
                    "auto-filter: S{} F{}{}",
                    target.string,
                    target.fret,
                    degree_suffix(target.degree)
                ),
                FontId::proportional(12.0),
                Color32::from_rgb(152, 158, 165),
            );
        }

        if let Some(reading) = reading {
            let freq_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 34.0),
                pos2(rect.right() - 14.0, rect.top() + 84.0),
            );
            painter.text(
                pos2(freq_rect.left(), freq_rect.top() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                "Frequency waterfall",
                FontId::proportional(11.0),
                Color32::from_rgb(152, 158, 165),
            );
            self.draw_waterfall(&painter, freq_rect, &reading.waterfall);

            let note_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 104.0),
                pos2(rect.right() - 56.0, rect.top() + 154.0),
            );
            painter.text(
                pos2(note_rect.left(), note_rect.top() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                "Note waterfall",
                FontId::proportional(11.0),
                Color32::from_rgb(152, 158, 165),
            );
            self.draw_note_waterfall(&painter, note_rect, &reading.note_waterfall, &reading.note_labels);

            let bars_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 172.0),
                pos2(rect.right() - 14.0, rect.bottom() - 14.0),
            );
            let bar_width = bars_rect.width() / reading.spectrum.len().max(1) as f32;

            for (index, value) in reading.spectrum.iter().enumerate() {
                let x0 = bars_rect.left() + index as f32 * bar_width;
                let x1 = x0 + bar_width - 2.0;
                let height = bars_rect.height() * value.clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    pos2(x0, bars_rect.bottom() - height),
                    pos2(x1.max(x0 + 1.0), bars_rect.bottom()),
                );
                painter.rect_filled(bar_rect, 3.0, spectrum_color(*value));
            }

            self.draw_note_bar_overlay(&painter, bars_rect, &reading.note_spectrum);
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waterfalls will appear when the tuner locks onto a note",
                FontId::proportional(13.0),
                Color32::from_rgb(139, 143, 149),
            );
        }
    }

    fn draw_waterfall(&self, painter: &egui::Painter, rect: Rect, waterfall: &[Vec<f32>]) {
        if waterfall.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waterfall will fill as audio arrives",
                FontId::proportional(12.0),
                Color32::from_rgb(128, 133, 139),
            );
            return;
        }

        let rows = waterfall.len().max(1);
        let cols = waterfall[0].len().max(1);
        let cell_h = rect.height() / rows as f32;
        let cell_w = rect.width() / cols as f32;

        for (row_index, row) in waterfall.iter().enumerate() {
            for (col_index, value) in row.iter().enumerate() {
                let min = pos2(
                    rect.left() + col_index as f32 * cell_w,
                    rect.top() + row_index as f32 * cell_h,
                );
                let max = pos2(min.x + cell_w + 0.5, min.y + cell_h + 0.5);
                painter.rect_filled(
                    Rect::from_min_max(min, max),
                    0.0,
                    waterfall_color(*value, row_index as f32 / rows as f32),
                );
            }
        }
    }

    fn draw_note_waterfall(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        waterfall: &[Vec<f32>],
        labels: &[String],
    ) {
        self.draw_waterfall(painter, rect, waterfall);

        if labels.is_empty() {
            return;
        }

        let label_stride = 6usize;
        let cell_w = rect.width() / labels.len() as f32;
        for index in (0..labels.len()).step_by(label_stride) {
            let x = rect.left() + index as f32 * cell_w;
            painter.text(
                pos2(x, rect.bottom() + 4.0),
                egui::Align2::LEFT_TOP,
                labels[index].as_str(),
                FontId::proportional(10.0),
                Color32::from_rgb(128, 133, 139),
            );
        }
    }

    fn draw_note_bar_overlay(&self, painter: &egui::Painter, rect: Rect, note_spectrum: &[f32]) {
        if note_spectrum.is_empty() {
            return;
        }

        let width = rect.width() / note_spectrum.len() as f32;
        for (index, value) in note_spectrum.iter().enumerate() {
            if *value < 0.18 {
                continue;
            }
            let x = rect.left() + index as f32 * width;
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(214, 200, 182, 36)),
            );
        }
    }

    fn draw_footer_note(&self, ui: &mut Ui, component_rect: Rect) {
        let painter = ui.painter_at(component_rect);
        painter.text(
            pos2(component_rect.right() - 16.0, component_rect.bottom() - 16.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "{} tuning • {} scale",
                self.tuning_kind.label(),
                self.scale_kind.label()
            ),
            FontId::proportional(12.0),
            Color32::from_rgb(128, 134, 143),
        );
    }

    fn hovered_note(
        &self,
        pointer_pos: egui::Pos2,
        fretboard: &Fretboard,
        scale: &Scale,
    ) -> Option<HoveredNote> {
        for string in fretboard.iter_strings() {
            for fret in fretboard.iter_frets() {
                let center = pos2(fretboard.fret_pos(fret), fretboard.string_pos(string));
                let rect = Rect::from_center_size(center, vec2(34.0, 22.0));

                if rect.contains(pointer_pos) {
                    let note = fretboard.tuning.note(string).add(fret.semitones());
                    let degree = scale.degree(note.to_pc().1).map(|value| value.0);

                    return Some(HoveredNote {
                        string: string.0,
                        fret: fret.0,
                        note_name: note.to_anote().name(),
                        degree,
                        center,
                        rect,
                    });
                }
            }
        }

        None
    }

    fn draw_hovered_note(&self, painter: &egui::Painter, component_rect: Rect, hovered: &HoveredNote) {
        painter.rect_stroke(
            hovered.rect.expand2(vec2(4.0, 4.0)),
            10.0,
            Stroke::new(2.0, Color32::from_rgb(214, 200, 182)),
            egui::StrokeKind::Outside,
        );
        painter.circle_filled(hovered.center, 3.0, Color32::from_rgb(224, 213, 196));

        let degree_label = hovered
            .degree
            .map(|degree| format!("degree {}", degree))
            .unwrap_or_else(|| "outside scale".to_owned());

        let tooltip_rect = Rect::from_min_size(
            pos2(component_rect.left() + 14.0, component_rect.top() + 14.0),
            vec2(200.0, 58.0),
        );

        painter.rect_filled(
            tooltip_rect,
            14.0,
            Color32::from_rgba_unmultiplied(24, 26, 30, 236),
        );
        painter.rect_stroke(
            tooltip_rect,
            14.0,
            Stroke::new(1.0, Color32::from_rgb(88, 92, 98)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            pos2(tooltip_rect.left() + 12.0, tooltip_rect.top() + 11.0),
            egui::Align2::LEFT_TOP,
            hovered.note_name.as_str(),
            FontId::proportional(17.0),
            Color32::from_rgb(228, 220, 208),
        );
        painter.text(
            pos2(tooltip_rect.left() + 12.0, tooltip_rect.top() + 34.0),
            egui::Align2::LEFT_TOP,
            format!(
                "string {}  •  fret {}  •  {}",
                hovered.string, hovered.fret, degree_label
            ),
            FontId::proportional(12.0),
            Color32::from_rgb(160, 165, 171),
        );
    }

    fn root_label(&self) -> &'static str {
        ALL_ROOTS
            .iter()
            .find_map(|(note, label)| (*note == self.root_note).then_some(*label))
            .unwrap_or("?")
    }

    fn filtered_tuner_target(&self, tuning: &Tuning, scale: &Scale) -> Option<TunerTarget> {
        let reading = self.audio.reading()?;
        let mut best: Option<TunerTarget> = None;

        for string in 1..=tuning.string_count() {
            let open = tuning.note(crate::core_types::tuning::GString(string));
            for fret in 0..=18 {
                let note = open.add(crate::core_types::pitch::Interval(fret as i32));
                let target_frequency = midi_to_frequency(note.as_u8() as f32);
                let cents = 1200.0 * (reading.frequency_hz / target_frequency).log2();
                let distance = cents.abs();

                if distance > 65.0 {
                    continue;
                }

                let replace = best
                    .as_ref()
                    .map(|current| distance < current.cents.abs())
                    .unwrap_or(true);

                if replace {
                    let degree = scale.degree(note.to_pc().1).map(|value| value.0);
                    best = Some(TunerTarget {
                        string,
                        fret,
                        note_name: note.to_anote().name(),
                        frequency_hz: reading.frequency_hz,
                        cents,
                        degree,
                    });
                }
            }
        }

        best
    }

    fn draw_tuner_target(&self, painter: &egui::Painter, fretboard: &Fretboard, target: &TunerTarget) {
        let center = pos2(
            fretboard.fret_pos(Fret(target.fret)),
            fretboard.string_pos(crate::core_types::tuning::GString(target.string)),
        );
        painter.circle_stroke(center, 18.0, Stroke::new(2.0, Color32::from_rgb(216, 205, 187)));
        painter.circle_stroke(
            center,
            24.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(216, 205, 187, 96)),
        );
    }
}

fn pill(ui: &mut Ui, label: &str, fg: Color32, bg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(12.0).color(fg));
        });
}

fn audio_status_label(status: &AudioStatus) -> String {
    match status {
        AudioStatus::Idle => "Microphone idle".to_owned(),
        AudioStatus::Listening => "Listening to microphone".to_owned(),
        AudioStatus::Error(message) => format!("Audio error: {message}"),
    }
}

fn audio_status_color(status: &AudioStatus) -> Color32 {
    match status {
        AudioStatus::Idle => Color32::from_rgb(154, 160, 168),
        AudioStatus::Listening => Color32::from_rgb(185, 194, 176),
        AudioStatus::Error(_) => Color32::from_rgb(210, 166, 136),
    }
}

fn cents_color(cents: f32) -> Color32 {
    if cents.abs() < 6.0 {
        Color32::from_rgb(182, 197, 164)
    } else if cents.abs() < 18.0 {
        Color32::from_rgb(206, 188, 151)
    } else {
        Color32::from_rgb(198, 146, 126)
    }
}

fn spectrum_color(value: f32) -> Color32 {
    let value = value.clamp(0.0, 1.0);
    let r = (96.0 + value * 70.0).round() as u8;
    let g = (88.0 + value * 82.0).round() as u8;
    let b = (82.0 + value * 56.0).round() as u8;
    Color32::from_rgb(r, g, b)
}

fn waterfall_color(value: f32, age: f32) -> Color32 {
    let intensity = value.clamp(0.0, 1.0);
    let fade = (0.35 + age * 0.65).clamp(0.0, 1.0);
    let r = (34.0 + intensity * 138.0 * fade).round() as u8;
    let g = (42.0 + intensity * 120.0 * fade).round() as u8;
    let b = (52.0 + intensity * 92.0 * fade).round() as u8;
    Color32::from_rgb(r, g, b)
}

fn midi_to_frequency(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

fn degree_suffix(degree: Option<u8>) -> String {
    degree
        .map(|value| format!(" • degree {}", value))
        .unwrap_or_default()
}

pub fn rangef_to_range(range: Rangef) -> Range<f32> {
    range.min..range.max
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut AppFrame) {
        #[cfg(not(target_arch = "wasm32"))]
        subsecond::call(|| self.render(ui));

        #[cfg(target_arch = "wasm32")]
        self.render(ui);
    }
}
