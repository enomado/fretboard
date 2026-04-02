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
    tuning_kind: TuningKind,
    scale_kind:  ScaleKind,
    root_note:   Note,
}

impl App {
    pub fn new(cc: &CreationContext) -> Self {
        apply_theme(&cc.egui_ctx);

        Self {
            tuning_kind: TuningKind::Cello,
            scale_kind:  ScaleKind::BluesMinor,
            root_note:   Note::A,
        }
    }

    fn render(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(12, 17, 24))
                    .inner_margin(Margin::same(18)),
            )
            .show_inside(ui, |ui| {
                self.draw_header(ui);
                ui.add_space(14.0);
                self.draw_controls(ui);
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
                        .color(Color32::from_rgb(239, 225, 196))
                        .family(egui::FontFamily::Proportional),
                );
                ui.label(
                    RichText::new(format!(
                        "{} • {} • root {}",
                        self.tuning_kind.subtitle(),
                        self.scale_kind.label(),
                        self.root_label()
                    ))
                    .color(Color32::from_rgb(155, 168, 186)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(
                    ui,
                    "Muted",
                    Color32::from_rgb(117, 126, 145),
                    Color32::from_rgb(69, 76, 92),
                );
                pill(
                    ui,
                    "5th",
                    Color32::from_rgb(255, 186, 119),
                    Color32::from_rgb(84, 54, 28),
                );
                pill(
                    ui,
                    "Root",
                    Color32::from_rgb(255, 208, 160),
                    Color32::from_rgb(140, 58, 48),
                );
            });
        });
    }

    fn draw_controls(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(18))
            .stroke(Stroke::new(1.0, Color32::from_rgb(56, 67, 84)))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Tuning")
                            .color(Color32::from_rgb(214, 200, 171))
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
                            .color(Color32::from_rgb(214, 200, 171))
                            .strong(),
                    );
                    for (note, label) in ALL_ROOTS {
                        let selected = self.root_note == note;
                        let button = egui::Button::new(label)
                            .min_size(vec2(30.0, 28.0))
                            .fill(if selected {
                                Color32::from_rgb(173, 75, 54)
                            } else {
                                Color32::from_rgb(42, 49, 61)
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if selected {
                                    Color32::from_rgb(255, 210, 159)
                                } else {
                                    Color32::from_rgb(79, 92, 109)
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
                            .color(Color32::from_rgb(214, 200, 171))
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

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0, Color32::from_rgb(56, 67, 84)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Scale tones: {}", scale.notes().len()))
                            .color(Color32::from_rgb(148, 165, 184)),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Visible frets: {}-{}", 1, 18))
                            .color(Color32::from_rgb(148, 165, 184)),
                    );
                });

                ui.add_space(10.0);

                let avail_width = ui.available_width();
                let (component_rect, _) =
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
                    Stroke::new(1.0, Color32::from_rgb(135, 93, 52)),
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

                self.draw_footer_note(ui, component_rect);
            });
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
            Color32::from_rgb(121, 136, 157),
        );
    }

    fn root_label(&self) -> &'static str {
        ALL_ROOTS
            .iter()
            .find_map(|(note, label)| (*note == self.root_note).then_some(*label))
            .unwrap_or("?")
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
