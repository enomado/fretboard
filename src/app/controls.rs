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
    audio_status_color,
    audio_status_label,
    format_sample_count,
    input_path_class_label,
    input_path_detail,
    input_source_debug_label,
    input_supports_monitor,
    monitor_output_debug_label,
    output_has_bluetooth_risk,
};
use crate::audio::{
    AnalysisSettings,
    AudioInputKind,
};
use crate::ui::theme::PANEL_FILL;

impl App {
    pub(super) fn draw_controls(&mut self, ui: &mut Ui) {
        let mut input_gain = self.audio.input_gain();
        let input_level = self.audio.input_level();
        let mut monitor_enabled = self.audio.monitor_enabled();
        let mut monitor_gain = self.audio.monitor_gain();
        let status = self.audio.status();
        let selected_input_id = self.audio.selected_input_id();
        let selected_input_kind = self.selected_input_kind(selected_input_id.as_deref());
        let monitor_supported = input_supports_monitor(selected_input_id.as_deref());
        let input_sample_rate = self.audio.current_input_sample_rate();
        let monitor_output_sample_rate = self.audio.monitor_output_sample_rate();
        let output_device_name = self.audio.default_output_device_name();
        let frame_width = ui.available_width();
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
                ui.set_min_width(frame_width - 32.0);

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

                ui.add_space(10.0);
                self.draw_input_level(ui, input_level, selected_input_kind);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let monitor_button = egui::Button::new(if monitor_enabled {
                        "Monitor on"
                    } else {
                        "Monitor off"
                    })
                    .min_size(vec2(104.0, 28.0))
                    .fill(if monitor_enabled {
                        Color32::from_rgb(112, 86, 72)
                    } else {
                        Color32::from_rgb(42, 46, 52)
                    })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if monitor_enabled {
                            Color32::from_rgb(207, 187, 166)
                        } else {
                            Color32::from_rgb(84, 89, 97)
                        },
                    ))
                    .corner_radius(CornerRadius::same(14));

                    if ui.add_enabled(monitor_supported, monitor_button).clicked() {
                        monitor_enabled = !monitor_enabled;
                        self.audio.set_monitor_enabled(monitor_enabled);
                    }

                    ui.label(
                        RichText::new("Monitor gain")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );

                    let slider = egui::Slider::new(&mut monitor_gain, 0.0..=1.0)
                        .clamping(egui::SliderClamping::Always)
                        .trailing_fill(true)
                        .show_value(false);
                    if ui.add_sized([140.0, 18.0], slider).changed() {
                        self.audio.set_monitor_gain(monitor_gain);
                    }

                    ui.label(
                        RichText::new(format!("{:>3.0}%", monitor_gain * 100.0))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(audio_status_label(&status, selected_input_kind))
                        .color(audio_status_color(&status))
                        .size(12.0),
                );
                ui.label(
                    RichText::new(if monitor_supported {
                        "Monitor plays the selected input back through the default output"
                    } else {
                        "Monitor is disabled for monitor / loopback system inputs"
                    })
                    .color(Color32::from_rgb(145, 151, 160))
                    .size(12.0),
                );
                ui.label(
                    RichText::new(input_source_debug_label(selected_input_id.as_deref()))
                        .color(Color32::from_rgb(145, 151, 160))
                        .size(12.0)
                        .monospace(),
                );
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Test note")
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                    if ui
                        .add_sized(
                            [180.0, 18.0],
                            egui::Slider::new(&mut self.test_note_midi, 12..=84).show_value(false),
                        )
                        .changed()
                    {
                        self.test_note_midi = self.test_note_midi.clamp(12, 84);
                    }
                    ui.label(
                        RichText::new(midi_label(self.test_note_midi))
                            .color(Color32::from_rgb(226, 216, 201))
                            .monospace(),
                    );
                    let play_button = egui::Button::new("Play note")
                        .min_size(vec2(92.0, 28.0))
                        .fill(Color32::from_rgb(42, 78, 72))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(111, 154, 142)))
                        .corner_radius(CornerRadius::same(14));
                    if ui.add(play_button).clicked() {
                        self.audio.play_test_note(self.test_note_midi);
                    }
                });
                ui.add_space(8.0);
                Frame::new()
                    .fill(Color32::from_rgb(28, 32, 37))
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(52, 58, 66)))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Signal path diagnostics")
                                .color(Color32::from_rgb(205, 194, 176))
                                .strong()
                                .size(12.0),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "Input path: {}",
                                input_path_class_label(selected_input_id.as_deref())
                            ))
                            .color(Color32::from_rgb(226, 216, 201))
                            .size(12.0),
                        );
                        ui.label(
                            RichText::new(input_path_detail(selected_input_id.as_deref()))
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(format!("Input rate: {} Hz", input_sample_rate))
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0)
                                .monospace(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Monitor output: {}",
                                monitor_output_debug_label(
                                    output_device_name.as_deref(),
                                    monitor_output_sample_rate,
                                )
                            ))
                            .color(Color32::from_rgb(145, 151, 160))
                            .size(12.0)
                            .monospace(),
                        );
                        if output_has_bluetooth_risk(output_device_name.as_deref()) {
                            ui.label(
                                RichText::new(
                                    "Bluetooth output detected: playback monitor latency and clicks are much more likely.",
                                )
                                .color(Color32::from_rgb(210, 166, 136))
                                .size(12.0),
                            );
                        }
                    });
            });
    }

    pub(super) fn draw_general_config_card(&mut self, ui: &mut Ui) {
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
                            RichText::new("Config: General")
                                .color(Color32::from_rgb(226, 216, 201))
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Window, smoothing, and analysis range")
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset").clicked() {
                            settings.window_size = defaults.window_size;
                            settings.spectrum_smoothing = defaults.spectrum_smoothing;
                            settings.min_frequency = defaults.min_frequency;
                            settings.max_frequency = defaults.max_frequency;
                            changed = true;
                        }
                    });
                });

                ui.add_space(10.0);
                self.draw_general_config_tab(ui, &mut settings, &mut changed);
            });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    pub(super) fn draw_fft1_config_card(&mut self, ui: &mut Ui) {
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
                            RichText::new("Config: FFT1")
                                .color(Color32::from_rgb(226, 216, 201))
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Primary FFT size and display shaping")
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset").clicked() {
                            settings.fft_size = defaults.fft_size;
                            settings.note_spread = defaults.note_spread;
                            settings.spectrum_gamma = defaults.spectrum_gamma;
                            settings.note_gamma = defaults.note_gamma;
                            changed = true;
                        }
                    });
                });

                ui.add_space(10.0);
                self.draw_fft1_config_tab(ui, &mut settings, &mut changed);
            });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    pub(super) fn draw_resonator_fft_config_card(&mut self, ui: &mut Ui) {
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
                            RichText::new("Config: Resonator FFT")
                                .color(Color32::from_rgb(226, 216, 201))
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Resonator bank range, density, and response")
                                .color(Color32::from_rgb(145, 151, 160))
                                .size(12.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset").clicked() {
                            settings.resonator_min_midi = defaults.resonator_min_midi;
                            settings.resonator_max_midi = defaults.resonator_max_midi;
                            settings.resonator_bins = defaults.resonator_bins;
                            settings.resonator_alpha = defaults.resonator_alpha;
                            settings.resonator_beta = defaults.resonator_beta;
                            settings.resonator_gamma = defaults.resonator_gamma;
                            changed = true;
                        }
                    });
                });

                ui.add_space(10.0);
                self.draw_resonator_fft_config_tab(ui, &mut settings, &mut changed);
            });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    fn draw_general_config_tab(&mut self, ui: &mut Ui, settings: &mut AnalysisSettings, changed: &mut bool) {
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
                            .selectable_value(&mut settings.window_size, preset, format_sample_count(preset))
                            .changed()
                        {
                            *changed = true;
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
                *changed = true;
            }
            ui.label(
                RichText::new(settings.spectrum_smoothing.to_string())
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Min Hz")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [180.0, 18.0],
                    egui::Slider::new(&mut settings.min_frequency, 16.0..=600.0)
                        .logarithmic(true)
                        .show_value(false),
                )
                .changed()
            {
                *changed = true;
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
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.0}", settings.max_frequency))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });
    }

    fn draw_fft1_config_tab(&mut self, ui: &mut Ui, settings: &mut AnalysisSettings, changed: &mut bool) {
        ui.horizontal_wrapped(|ui| {
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
                            .selectable_value(&mut settings.fft_size, preset, format_sample_count(preset))
                            .changed()
                        {
                            *changed = true;
                        }
                    }
                });

            ui.separator();

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
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.2}", settings.note_spread))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("FFT gamma")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [160.0, 18.0],
                    egui::Slider::new(&mut settings.spectrum_gamma, 0.35..=1.2).show_value(false),
                )
                .changed()
            {
                *changed = true;
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
                    [160.0, 18.0],
                    egui::Slider::new(&mut settings.note_gamma, 0.35..=1.2).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.2}", settings.note_gamma))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });
    }

    fn draw_resonator_fft_config_tab(
        &mut self,
        ui: &mut Ui,
        settings: &mut AnalysisSettings,
        changed: &mut bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Range")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );

            if ui
                .add_sized(
                    [140.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_min_midi, 12..=84).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(midi_label(settings.resonator_min_midi))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );

            ui.label(
                RichText::new("to")
                    .color(Color32::from_rgb(145, 151, 160))
                    .strong(),
            );

            if ui
                .add_sized(
                    [140.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_max_midi, 24..=108).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(midi_label(settings.resonator_max_midi))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );

            ui.separator();

            ui.label(
                RichText::new("Bins / semitone")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [120.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_bins, 1..=12).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(settings.resonator_bins.to_string())
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Alpha")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [150.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_alpha, 0.2..=4.0).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.2}", settings.resonator_alpha))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );

            ui.add_space(10.0);

            ui.label(
                RichText::new("Beta")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [150.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_beta, 0.2..=4.0).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.2}", settings.resonator_beta))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );

            ui.add_space(10.0);

            ui.label(
                RichText::new("Gamma")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            if ui
                .add_sized(
                    [150.0, 18.0],
                    egui::Slider::new(&mut settings.resonator_gamma, 0.35..=1.2).show_value(false),
                )
                .changed()
            {
                *changed = true;
            }
            ui.label(
                RichText::new(format!("{:.2}", settings.resonator_gamma))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
        });
    }
}

fn midi_label(midi: usize) -> String {
    const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let note_index = midi % 12;
    let octave = midi as i32 / 12 - 1;
    format!("{}{}", NOTE_NAMES[note_index], octave)
}
