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

use super::{
    ALL_ROOTS,
    ALL_SCALES,
    ALL_TUNINGS,
    App,
    FFT_SIZE_PRESETS,
    WINDOW_SIZE_PRESETS,
    format_sample_count,
};
use crate::audio::{
    AnalysisSettings,
    AudioInputKind,
};
use crate::ui::theme::PANEL_FILL;

impl App {
    pub(super) fn draw_controls(&mut self, ui: &mut Ui) {
        let mut input_gain = self.audio.input_gain();
        let selected_input_id = self.audio.selected_input_id();
        let selected_input_kind = self.selected_input_kind(selected_input_id.as_deref());
        let has_system_input = self
            .audio_inputs
            .iter()
            .any(|option| option.kind == AudioInputKind::System);
        let has_microphone_input = self
            .audio_inputs
            .iter()
            .any(|option| option.kind == AudioInputKind::Microphone);

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(18))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
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
                                1.0_f32,
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
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Source")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );

                    let mic_button = egui::Button::new("Microphone")
                        .min_size(vec2(104.0, 28.0))
                        .fill(if selected_input_kind == AudioInputKind::Microphone {
                            Color32::from_rgb(112, 86, 72)
                        } else {
                            Color32::from_rgb(42, 46, 52)
                        })
                        .stroke(Stroke::new(
                            1.0_f32,
                            if selected_input_kind == AudioInputKind::Microphone {
                                Color32::from_rgb(207, 187, 166)
                            } else {
                                Color32::from_rgb(84, 89, 97)
                            },
                        ))
                        .corner_radius(CornerRadius::same(14));
                    if ui.add_enabled(has_microphone_input, mic_button).clicked() {
                        if let Some(input_id) = self.preferred_input_id(AudioInputKind::Microphone) {
                            self.audio.set_selected_input_id(Some(input_id));
                        }
                    }

                    let system_button = egui::Button::new("System")
                        .min_size(vec2(88.0, 28.0))
                        .fill(if selected_input_kind == AudioInputKind::System {
                            Color32::from_rgb(112, 86, 72)
                        } else {
                            Color32::from_rgb(42, 46, 52)
                        })
                        .stroke(Stroke::new(
                            1.0_f32,
                            if selected_input_kind == AudioInputKind::System {
                                Color32::from_rgb(207, 187, 166)
                            } else {
                                Color32::from_rgb(84, 89, 97)
                            },
                        ))
                        .corner_radius(CornerRadius::same(14));
                    if ui.add_enabled(has_system_input, system_button).clicked() {
                        if let Some(input_id) = self.preferred_input_id(AudioInputKind::System) {
                            self.audio.set_selected_input_id(Some(input_id));
                        }
                    }

                    ui.separator();

                    ui.label(
                        RichText::new("Device")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );

                    let selected_input_label = selected_input_id
                        .as_deref()
                        .and_then(|id| self.audio_inputs.iter().find(|option| option.id == id))
                        .map(|option| option.label.clone())
                        .unwrap_or_else(|| "Choose input device".to_owned());

                    egui::ComboBox::from_id_salt("audio_input_device")
                        .selected_text(selected_input_label)
                        .width(340.0)
                        .show_ui(ui, |ui| {
                            for option in &self.audio_inputs {
                                if ui
                                    .selectable_label(
                                        selected_input_id.as_deref() == Some(option.id.as_str()),
                                        &option.label,
                                    )
                                    .clicked()
                                {
                                    self.audio.set_selected_input_id(Some(option.id.clone()));
                                }
                            }
                        });

                    if ui.button("Refresh inputs").clicked() {
                        self.audio_inputs = self.audio.available_inputs();
                    }
                });

                if has_system_input {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Use the System source to capture monitor / loopback / Stereo Mix inputs",
                        )
                        .color(Color32::from_rgb(145, 151, 160))
                        .size(12.0),
                    );
                } else {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "System audio appears only if the OS exposes a monitor / loopback input device",
                        )
                        .color(Color32::from_rgb(145, 151, 160))
                        .size(12.0),
                    );
                }

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
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(52, 58, 66)))
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
}
