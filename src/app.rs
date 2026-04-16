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
    AnalysisSettings,
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
const SPIRAL_PITCH_LABELS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
const WINDOW_SIZE_PRESETS: [usize; 6] = [2048, 4096, 6144, 8192, 12288, 16384];
const FFT_SIZE_PRESETS: [usize; 4] = [4096, 8192, 16384, 32768];

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum LiveChartKind {
    Tuner,
    Fft,
    Spiral,
}

impl LiveChartKind {
    fn label(self) -> &'static str {
        match self {
            Self::Tuner => "Tuner",
            Self::Fft => "FFT",
            Self::Spiral => "Spiral",
        }
    }
}

pub struct App {
    audio:       AudioEngine,
    tuning_kind: TuningKind,
    scale_kind:  ScaleKind,
    root_note:   Note,
    live_chart:  LiveChartKind,
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
            live_chart:  LiveChartKind::Spiral,
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
        let mut input_gain = self.audio.input_gain();

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

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Mic gain")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );

                    let slider = egui::Slider::new(&mut input_gain, 1.0..=12.0)
                        .logarithmic(true)
                        .clamping(egui::SliderClamping::Always)
                        .trailing_fill(true)
                        .show_value(false);
                    if ui.add_sized([220.0, 18.0], slider).changed() {
                        self.audio.set_input_gain(input_gain);
                    }

                    ui.label(
                        RichText::new(format!("{input_gain:.1}x"))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                });

                ui.add_space(14.0);
                self.draw_analysis_controls(ui);
            });
    }

    fn draw_analysis_controls(&mut self, ui: &mut Ui) {
        let defaults = AnalysisSettings::default();
        let mut settings = self.audio.analysis_settings();
        let mut changed = false;

        Frame::new()
            .fill(Color32::from_rgb(25, 29, 34))
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0, Color32::from_rgb(52, 58, 66)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("FFT tweak panel")
                                .color(Color32::from_rgb(226, 216, 201))
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Adjust the live spectrum and spiral response in real time")
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset").clicked() {
                            settings = defaults.clone();
                            changed = true;
                        }
                    });
                });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Window")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    egui::ComboBox::from_id_salt("analysis_window_size")
                        .selected_text(format_sample_count(settings.window_size))
                        .show_ui(ui, |ui| {
                            for preset in WINDOW_SIZE_PRESETS {
                                if ui
                                    .selectable_value(
                                        &mut settings.window_size,
                                        preset,
                                        format_sample_count(preset),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });

                    ui.separator();

                    ui.label(
                        RichText::new("FFT")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    egui::ComboBox::from_id_salt("analysis_fft_size")
                        .selected_text(format_sample_count(settings.fft_size))
                        .show_ui(ui, |ui| {
                            for preset in FFT_SIZE_PRESETS {
                                if ui
                                    .selectable_value(
                                        &mut settings.fft_size,
                                        preset,
                                        format_sample_count(preset),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });

                    ui.separator();

                    ui.label(
                        RichText::new("Smooth")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [120.0, 18.0],
                            egui::Slider::new(&mut settings.spectrum_smoothing, 0..=4).show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(settings.spectrum_smoothing.to_string())
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Min Hz")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [180.0, 18.0],
                            egui::Slider::new(&mut settings.min_frequency, 20.0..=600.0)
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(format!("{:.0}", settings.min_frequency))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );

                    ui.add_space(10.0);

                    ui.label(
                        RichText::new("Max Hz")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [180.0, 18.0],
                            egui::Slider::new(&mut settings.max_frequency, 300.0..=4_000.0)
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(format!("{:.0}", settings.max_frequency))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Note spread")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [150.0, 18.0],
                            egui::Slider::new(&mut settings.note_spread, 0.15..=0.8).show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(format!("{:.2}", settings.note_spread))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );

                    ui.add_space(10.0);

                    ui.label(
                        RichText::new("FFT gamma")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [140.0, 18.0],
                            egui::Slider::new(&mut settings.spectrum_gamma, 0.35..=1.2).show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(format!("{:.2}", settings.spectrum_gamma))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );

                    ui.add_space(10.0);

                    ui.label(
                        RichText::new("Note gamma")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [140.0, 18.0],
                            egui::Slider::new(&mut settings.note_gamma, 0.35..=1.2).show_value(false),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(format!("{:.2}", settings.note_gamma))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                });
            });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    fn draw_fretboard_card(&self, ui: &mut Ui) {
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_targets = self.filtered_tuner_targets(&tuning, &scale);

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
                if !tuner_targets.is_empty() {
                    self.draw_tuner_targets(&painter, &fretboard, &tuner_targets);
                }
                if let Some(pointer_pos) = response.hover_pos() {
                    if let Some(hovered) = self.hovered_note(pointer_pos, &fretboard, &scale) {
                        self.draw_hovered_note(&painter, component_rect, &hovered);
                    }
                }

                self.draw_footer_note(ui, component_rect);
            });
    }

    fn draw_tuner_card(&mut self, ui: &mut Ui) {
        let status = self.audio.status();
        let reading = self.audio.reading();
        let input_level = self.audio.input_level();
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_targets = self.filtered_tuner_targets(&tuning, &scale);
        let target = tuner_targets.first();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Live analysis")
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
                            ui.add_space(8.0);
                        }

                        for chart in [LiveChartKind::Spiral, LiveChartKind::Fft, LiveChartKind::Tuner] {
                            let selected = self.live_chart == chart;
                            let button = egui::Button::new(chart.label())
                                .min_size(vec2(72.0, 28.0))
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
                                self.live_chart = chart;
                            }
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_input_level(ui, input_level);
                ui.add_space(12.0);
                match self.live_chart {
                    LiveChartKind::Tuner => self.draw_tuner_meter(ui, target),
                    LiveChartKind::Fft => self.draw_spectrum(ui, target, reading.as_ref()),
                    LiveChartKind::Spiral => self.draw_spiral_spectrogram(ui, reading.as_ref()),
                }
            });
    }

    fn draw_input_level(&self, ui: &mut Ui, input_level: f32) {
        let desired_size = vec2(ui.available_width(), 28.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 14.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        let inner = rect.shrink2(vec2(6.0, 6.0));
        let fill_width = inner.width() * input_level.clamp(0.0, 1.0);
        let fill_rect = Rect::from_min_max(inner.min, pos2(inner.min.x + fill_width, inner.max.y));
        let fill_color = if input_level < 0.15 {
            Color32::from_rgb(106, 116, 128)
        } else if input_level < 0.85 {
            Color32::from_rgb(192, 150, 97)
        } else {
            Color32::from_rgb(214, 108, 86)
        };
        if fill_width > 0.0 {
            painter.rect_filled(fill_rect, 10.0, fill_color);
        }

        painter.text(
            inner.left_center(),
            egui::Align2::LEFT_CENTER,
            "Mic level",
            FontId::proportional(13.0),
            Color32::from_rgb(196, 189, 177),
        );
        painter.text(
            inner.right_center(),
            egui::Align2::RIGHT_CENTER,
            format!("{:>3.0}%", input_level.clamp(0.0, 1.0) * 100.0),
            FontId::monospace(12.0),
            Color32::from_rgb(230, 223, 210),
        );
    }

    fn draw_tuner_meter(&self, ui: &mut Ui, reading: Option<&TunerTarget>) {
        let desired_size = vec2(ui.available_width().max(250.0), 120.0);
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
        let desired_size = vec2((ui.available_width()).max(280.0), 244.0);
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
            let section_label_gap = 16.0;
            let freq_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 50.0),
                pos2(rect.right() - 14.0, rect.top() + 94.0),
            );
            painter.text(
                pos2(freq_rect.left(), freq_rect.top() - section_label_gap),
                egui::Align2::LEFT_TOP,
                "Frequency waterfall",
                FontId::proportional(11.0),
                Color32::from_rgb(152, 158, 165),
            );
            self.draw_waterfall(&painter, freq_rect, &reading.waterfall);

            let note_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 124.0),
                pos2(rect.right() - 14.0, rect.top() + 174.0),
            );
            painter.text(
                pos2(note_rect.left(), note_rect.top() - section_label_gap),
                egui::Align2::LEFT_TOP,
                "Note waterfall",
                FontId::proportional(11.0),
                Color32::from_rgb(152, 158, 165),
            );
            self.draw_note_waterfall(
                &painter,
                note_rect,
                &reading.note_waterfall,
                &reading.note_labels,
                Some(reading.note_name.as_str()),
            );

            let bars_rect = Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + 192.0),
                pos2(rect.right() - 14.0, rect.bottom() - 14.0),
            );
            let bar_width = bars_rect.width() / reading.note_spectrum.len().max(1) as f32;

            for (index, value) in reading.note_spectrum.iter().enumerate() {
                let x0 = bars_rect.left() + index as f32 * bar_width;
                let x1 = x0 + bar_width - 2.0;
                let height = bars_rect.height() * value.clamp(0.0, 1.0);
                let bar_rect = Rect::from_min_max(
                    pos2(x0, bars_rect.bottom() - height),
                    pos2(x1.max(x0 + 1.0), bars_rect.bottom()),
                );
                painter.rect_filled(bar_rect, 3.0, spectrum_color(*value));
            }
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

    fn draw_spiral_spectrogram(&self, ui: &mut Ui, reading: Option<&TunerReading>) {
        let desired_size = vec2(ui.available_width().max(320.0), 376.0);
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
            "Spiral spectrogram",
            FontId::proportional(15.0),
            Color32::from_rgb(201, 195, 184),
        );
        painter.text(
            pos2(rect.right() - 14.0, rect.top() + 12.0),
            egui::Align2::RIGHT_TOP,
            "octaves wrap onto the same pitch angle",
            FontId::proportional(12.0),
            Color32::from_rgb(152, 158, 165),
        );

        let viz_rect = Rect::from_min_max(
            pos2(rect.left() + 20.0, rect.top() + 44.0),
            pos2(rect.right() - 20.0, rect.bottom() - 20.0),
        );

        if let Some(reading) = reading {
            let settings = self.audio.analysis_settings();
            if reading.spiral_spectrum.is_empty() {
                painter.text(
                    viz_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "The note spectrum is empty",
                    FontId::proportional(13.0),
                    Color32::from_rgb(139, 143, 149),
                );
                return;
            }

            let square = viz_rect.width().min(viz_rect.height());
            let chart_rect = Rect::from_center_size(viz_rect.center(), vec2(square, square));
            let center = chart_rect.center();
            let inner_radius = square * 0.12;
            let outer_radius = square * 0.47;
            let semitone_count = reading.note_labels.len().max(1);
            let spiral_bin_count = reading.spiral_spectrum.len().max(1);
            let bins_per_semitone = if semitone_count > 1 {
                (spiral_bin_count.saturating_sub(1) as f32 / (semitone_count - 1) as f32).max(1.0)
            } else {
                1.0
            };
            let radius_step = if semitone_count > 1 {
                (outer_radius - inner_radius) / (semitone_count - 1) as f32
            } else {
                0.0
            };
            let active_index = reading
                .note_labels
                .iter()
                .position(|label| label == &reading.note_name);

            painter.circle_filled(
                center,
                inner_radius * 0.82,
                Color32::from_rgba_unmultiplied(70, 106, 148, 26),
            );

            for ring in 0..=semitone_count.saturating_sub(1) / 12 {
                let radius = inner_radius + ring as f32 * radius_step * 12.0;
                painter.circle_stroke(
                    center,
                    radius.min(outer_radius),
                    Stroke::new(1.0, Color32::from_rgb(59, 64, 72)),
                );
            }

            for pitch_class in 0..12 {
                let angle = pitch_class_angle(pitch_class);
                let direction = vec2(angle.cos(), angle.sin());
                let label_pos = center + direction * (outer_radius + 20.0);
                let spoke_color = pitch_class_color(pitch_class);
                let spoke_stroke = if Some(pitch_class) == active_index.map(|index| index % 12) {
                    Stroke::new(1.6, spoke_color)
                } else {
                    Stroke::new(1.0, Color32::from_rgb(55, 60, 67))
                };

                painter.line_segment(
                    [
                        center + direction * inner_radius * 0.58,
                        center + direction * outer_radius,
                    ],
                    spoke_stroke,
                );
                painter.text(
                    label_pos,
                    egui::Align2::CENTER_CENTER,
                    SPIRAL_PITCH_LABELS[pitch_class],
                    FontId::proportional(18.0),
                    spoke_color,
                );
            }

            let spiral_points: Vec<_> = (0..spiral_bin_count)
                .map(|index| {
                    spiral_point_fractional(
                        center,
                        inner_radius,
                        radius_step,
                        index as f32 / bins_per_semitone,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(
                spiral_points,
                Stroke::new(1.1, Color32::from_rgb(76, 82, 90)),
            ));

            for (history_index, row) in reading.spiral_waterfall.iter().enumerate() {
                let age = history_index as f32 / reading.spiral_waterfall.len().max(1) as f32;
                let strengths = spiral_contrast_strengths(row, &settings);
                for (note_index, intensity) in strengths.iter().copied().enumerate() {
                    if intensity <= 0.0 {
                        continue;
                    }

                    let semitone_position = note_index as f32 / bins_per_semitone;
                    let position =
                        spiral_point_fractional(center, inner_radius, radius_step, semitone_position);
                    let glow = 1.8 + intensity * 6.0 * (0.45 + age * 0.40);
                    painter.circle_filled(
                        position,
                        glow,
                        spiral_note_color(
                            semitone_position.floor() as usize % 12,
                            intensity,
                            10 + (age * 28.0) as u8,
                        ),
                    );
                }
            }

            for (note_index, intensity) in spiral_contrast_strengths(&reading.spiral_spectrum, &settings)
                .into_iter()
                .enumerate()
            {
                if intensity <= 0.0 {
                    continue;
                }

                let semitone_position = note_index as f32 / bins_per_semitone;
                let position = spiral_point_fractional(center, inner_radius, radius_step, semitone_position);
                let glow_radius = 3.0 + intensity * 8.0;
                let core_radius = 1.4 + intensity * 3.2;
                let color = pitch_class_color(semitone_position.floor() as usize % 12);

                painter.circle_filled(
                    position,
                    glow_radius,
                    spiral_note_color(
                        semitone_position.floor() as usize % 12,
                        intensity,
                        28 + (intensity * 96.0) as u8,
                    ),
                );
                painter.circle_filled(position, core_radius, color);
            }

            if let Some(active_index) = active_index {
                let active_position =
                    spiral_point_fractional(center, inner_radius, radius_step, active_index as f32);
                let active_color = pitch_class_color(active_index % 12);
                painter.circle_stroke(active_position, 11.0, Stroke::new(2.0, active_color));
                painter.circle_stroke(
                    active_position,
                    17.0,
                    Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(
                            active_color.r(),
                            active_color.g(),
                            active_color.b(),
                            100,
                        ),
                    ),
                );
                painter.text(
                    pos2(rect.left() + 14.0, rect.bottom() - 14.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("active note {}", reading.note_name),
                    FontId::proportional(12.0),
                    Color32::from_rgb(214, 206, 192),
                );
            }
        } else {
            painter.text(
                viz_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Play a sustained note to light up the spiral",
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
        active_note: Option<&str>,
    ) {
        self.draw_waterfall(painter, rect, waterfall);

        if labels.is_empty() {
            return;
        }

        let label_stride = 6usize;
        let cell_w = rect.width() / labels.len() as f32;
        let active_index = active_note.and_then(|note| labels.iter().position(|label| label == note));

        if let Some(index) = active_index {
            let x0 = rect.left() + index as f32 * cell_w;
            let x1 = x0 + cell_w;
            let center_x = (x0 + x1) * 0.5;
            painter.rect_filled(
                Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom())),
                0.0,
                Color32::from_rgba_unmultiplied(214, 200, 182, 24),
            );
            painter.line_segment(
                [pos2(center_x, rect.top()), pos2(center_x, rect.bottom())],
                Stroke::new(2.0, Color32::from_rgb(214, 200, 182)),
            );
        }

        for index in (0..labels.len()).step_by(label_stride) {
            if Some(index) == active_index {
                continue;
            }

            let x = rect.left() + (index as f32 + 0.5) * cell_w;
            painter.text(
                pos2(x, rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                labels[index].as_str(),
                FontId::proportional(10.0),
                Color32::from_rgb(128, 133, 139),
            );
        }

        if let Some(index) = active_index {
            let x = rect.left() + (index as f32 + 0.5) * cell_w;
            painter.text(
                pos2(x, rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                labels[index].as_str(),
                FontId::proportional(10.0),
                Color32::from_rgb(228, 220, 208),
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

    fn filtered_tuner_targets(&self, tuning: &Tuning, scale: &Scale) -> Vec<TunerTarget> {
        let Some(reading) = self.audio.reading() else {
            return Vec::new();
        };
        let detected_midi = frequency_to_midi(reading.frequency_hz).round() as u8;
        let detected_frequency = midi_to_frequency(detected_midi as f32);
        let cents = 1200.0 * (reading.frequency_hz / detected_frequency).log2();
        let mut matches = Vec::new();

        for string in 1..=tuning.string_count() {
            let open = tuning.note(crate::core_types::tuning::GString(string));
            for fret in 0..=18 {
                let note = open.add(crate::core_types::pitch::Interval(fret as i32));
                if note.as_u8() != detected_midi {
                    continue;
                }

                let degree = scale.degree(note.to_pc().1).map(|value| value.0);
                matches.push(TunerTarget {
                    string,
                    fret,
                    note_name: note.to_anote().name(),
                    frequency_hz: reading.frequency_hz,
                    cents,
                    degree,
                });
            }
        }

        matches.sort_by_key(|target| (target.fret, target.string));
        matches
    }

    fn draw_tuner_targets(&self, painter: &egui::Painter, fretboard: &Fretboard, targets: &[TunerTarget]) {
        for target in targets {
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

fn pitch_class_angle(pitch_class: usize) -> f32 {
    -std::f32::consts::FRAC_PI_2 + pitch_class as f32 * std::f32::consts::TAU / 12.0
}

fn spiral_point_fractional(
    center: egui::Pos2,
    inner_radius: f32,
    radius_step: f32,
    semitone_position: f32,
) -> egui::Pos2 {
    let angle = -std::f32::consts::FRAC_PI_2 + semitone_position * std::f32::consts::TAU / 12.0;
    let radius = inner_radius + semitone_position * radius_step;
    center + vec2(angle.cos(), angle.sin()) * radius
}

fn pitch_class_color(pitch_class: usize) -> Color32 {
    match pitch_class % 12 {
        0 => Color32::from_rgb(92, 230, 105),
        1 => Color32::from_rgb(104, 222, 170),
        2 => Color32::from_rgb(112, 204, 238),
        3 => Color32::from_rgb(122, 173, 255),
        4 => Color32::from_rgb(127, 138, 255),
        5 => Color32::from_rgb(164, 116, 246),
        6 => Color32::from_rgb(212, 98, 219),
        7 => Color32::from_rgb(236, 93, 168),
        8 => Color32::from_rgb(232, 110, 121),
        9 => Color32::from_rgb(239, 167, 102),
        10 => Color32::from_rgb(230, 203, 94),
        _ => Color32::from_rgb(156, 218, 115),
    }
}

fn spiral_note_color(pitch_class: usize, intensity: f32, alpha: u8) -> Color32 {
    let base = pitch_class_color(pitch_class);
    let glow = (40.0 + intensity * 120.0).round() as u8;
    Color32::from_rgba_unmultiplied(
        base.r().saturating_add(glow / 4),
        base.g().saturating_add(glow / 4),
        base.b().saturating_add(glow / 5),
        alpha,
    )
}

fn spiral_contrast_strengths(values: &[f32], settings: &AnalysisSettings) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }

    let peak = values.iter().copied().fold(0.0, f32::max);
    if peak <= 0.0 {
        return vec![0.0; values.len()];
    }

    let mean = values.iter().copied().sum::<f32>() / values.len() as f32;
    let gamma_norm = normalize_setting(settings.note_gamma, 0.35, 1.2);
    let spread_norm = normalize_setting(settings.note_spread, 0.15, 0.8);
    let threshold_floor = lerp(0.025, 0.11, gamma_norm);
    let threshold_ceiling = lerp(0.22, 0.36, gamma_norm);
    let threshold =
        (mean * lerp(1.15, 1.95, spread_norm) + threshold_floor).clamp(threshold_floor, threshold_ceiling);
    let scale = (1.0 - threshold).max(0.001);
    let mut strengths = vec![0.0; values.len()];

    for index in 0..values.len() {
        let intensity = values[index].clamp(0.0, 1.0);
        let normalized = ((intensity - threshold) / scale).clamp(0.0, 1.0);
        if normalized <= 0.0 {
            continue;
        }

        let left = values[index.saturating_sub(1)].clamp(0.0, 1.0);
        let right = values[(index + 1).min(values.len() - 1)].clamp(0.0, 1.0);
        let neighbor = left.max(right);
        let is_local_peak = intensity >= left && intensity >= right;
        let neighbor_guard = lerp(0.96, 0.84, spread_norm);
        let ridge = ((intensity - neighbor * neighbor_guard) / scale).clamp(0.0, 1.0);
        let focus = if is_local_peak {
            lerp(0.48, 0.78, 1.0 - spread_norm) + ridge * lerp(0.28, 0.62, 1.0 - spread_norm)
        } else {
            ridge * lerp(0.04, 0.18, spread_norm)
        };
        let emphasis = lerp(1.55, 2.65, gamma_norm);
        let emphasized = normalized.powf(emphasis) * focus;

        if emphasized > lerp(0.012, 0.05, gamma_norm) {
            strengths[index] = emphasized;
        }
    }

    strengths
}

fn normalize_setting(value: f32, min: f32, max: f32) -> f32 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t.clamp(0.0, 1.0)
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

fn frequency_to_midi(frequency_hz: f32) -> f32 {
    69.0 + 12.0 * (frequency_hz / 440.0).log2()
}

fn degree_suffix(degree: Option<u8>) -> String {
    degree
        .map(|value| format!(" • degree {}", value))
        .unwrap_or_default()
}

fn format_sample_count(value: usize) -> String {
    if value >= 1000 {
        format!("{:.1}k", value as f32 / 1000.0)
    } else {
        value.to_string()
    }
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
