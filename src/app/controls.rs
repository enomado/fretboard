use eframe::egui::{
    self,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Stroke,
    Ui,
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
    input_rate_debug_label,
    input_source_debug_label,
    input_supports_monitor,
    monitor_output_debug_label,
    output_has_bluetooth_risk,
};
use crate::audio::types::{
    AnalysisSettings,
    AudioInputKind,
    PitchFrontend,
};
use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::PNote;
use crate::ui::segmented::{
    PillCombo,
    RowCaption,
    SegmentedButton,
};
use crate::ui::slider_row::SliderRow;
use crate::ui::theme;
use crate::ui::tokens::color;
use crate::ui::wrap_row::{
    keep_together,
    separated,
};

impl App {
    pub(super) fn draw_controls(&mut self, ui: &mut Ui) {
        self.refresh_audio_inputs_if_stale();

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
        let (pad, radius) = theme::card_pad(ui.ctx(), 16, 18);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                // Inner margin + the 1 dp stroke, both sides: egui sizes a frame with
                // its stroke, so margin alone made the card 2 dp wider than its pane.
                ui.set_min_width(frame_width - pad.sum().x - 2.0);

                ui.horizontal_wrapped(|ui| {
                    keep_together(ui, "source", |ui| {
                        ui.add(RowCaption::new("Source"));

                        // Canonical segmented toggle (see `ui::segmented`). `min_width`
                        // preserves the original 104/88 px pill widths.
                        let mic_button = SegmentedButton::new(
                            "Microphone",
                            selected_input_kind == AudioInputKind::Microphone,
                        )
                        .min_width(104.0);
                        if ui.add_enabled(has_microphone_input, mic_button).clicked() {
                            self.refresh_audio_inputs();
                            if let Some(input_id) = self.preferred_input_id(AudioInputKind::Microphone) {
                                self.audio.set_selected_input_id(Some(input_id));
                            }
                        }

                        let system_button = SegmentedButton::new(
                            "System",
                            selected_input_kind == AudioInputKind::System,
                        )
                        .min_width(88.0);
                        if ui.add_enabled(has_system_input, system_button).clicked() {
                            self.refresh_audio_inputs();
                            if let Some(input_id) = self.preferred_input_id(AudioInputKind::System) {
                                self.audio.set_selected_input_id(Some(input_id));
                            }
                        }
                    });

                    let selected_input_label = selected_input_id
                        .as_deref()
                        .and_then(|id| self.audio_inputs.iter().find(|option| option.id == id))
                        .map(|option| option.label.clone())
                        .unwrap_or_else(|| "Choose input device".to_owned());

                    separated(ui, "device", |ui| {
                        // Never wider than the row itself: 340 dp is more than a 360-dp
                        // phone's card has, and the combo ran past the screen edge.
                        PillCombo::new("audio_input_device", selected_input_label)
                            .caption(RowCaption::new("Device"))
                            .min_width(340.0_f32.min(ui.max_rect().width()))
                            .show(ui, |ui| {
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
                    });

                    // Shares the Source/Device row with the pills, so it has to be one
                    // too — a bare `ui.button` sits on a different height band and
                    // breaks the row's baseline.
                    if ui.add(SegmentedButton::action("Refresh inputs")).clicked() {
                        self.refresh_audio_inputs();
                    }
                });

                if has_system_input {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Use the System source to capture monitor / loopback / Stereo Mix inputs",
                        )
                        .color(color::TEXT_HINT)
                        .size(12.0),
                    );
                } else {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "System audio appears only if the OS exposes a monitor / loopback input device",
                        )
                        .color(color::TEXT_HINT)
                        .size(12.0),
                    );
                }

                ui.add_space(14.0);
                let mic_gain = SliderRow::new(RowCaption::new("Mic gain"), &mut input_gain, 1.0..=12.0, |v| {
                    format!("{v:.1}x")
                })
                .width(220.0)
                .logarithmic()
                .clamp_always()
                .trailing_fill();
                if ui.add(mic_gain).changed() {
                    self.audio.set_input_gain(input_gain);
                }

                ui.add_space(10.0);
                self.draw_input_level(ui, input_level, selected_input_kind);
                ui.add_space(10.0);
                // Wraps on a phone (~400 dp in one row); every item is a single widget.
                ui.horizontal_wrapped(|ui| {
                    let monitor_button = SegmentedButton::new(
                        if monitor_enabled { "Monitor on" } else { "Monitor off" },
                        monitor_enabled,
                    )
                    .min_width(104.0);

                    if ui.add_enabled(monitor_supported, monitor_button).clicked() {
                        monitor_enabled = !monitor_enabled;
                        self.audio.set_monitor_enabled(monitor_enabled);
                    }

                    let gain = SliderRow::new(RowCaption::new("Monitor gain"), &mut monitor_gain, 0.0..=1.0, |v| {
                        format!("{:>3.0}%", v * 100.0)
                    })
                    .width(140.0)
                    .clamp_always()
                    .trailing_fill();
                    if ui.add(gain).changed() {
                        self.audio.set_monitor_gain(monitor_gain);
                    }
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
                    .color(color::TEXT_HINT)
                    .size(12.0),
                );
                ui.label(
                    RichText::new(input_source_debug_label(selected_input_id.as_deref()))
                        .color(color::TEXT_HINT)
                        .size(12.0)
                        .monospace(),
                );
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    // Простейшая кнопка проверки вывода: дёргает одну фиксированную
                    // ноту через спикер. Полноценная игра нот — в панели «Drone».
                    let play_button = SegmentedButton::colored(
                        "Play test note",
                        color::PLAY_FILL,
                        color::PLAY_STROKE,
                    )
                    .min_width(116.0);
                    if ui.add(play_button).clicked() {
                        self.audio.play_test_note(self.test_note_midi);
                    }
                    ui.label(
                        RichText::new("Checks speaker output")
                            .color(color::TEXT_HINT)
                            .size(12.0),
                    );
                });
                ui.add_space(10.0);
                // Эталон A4 (камертон): задаёт стандарт строя для всего
                // движка (маппинг частота↔нота + банк резонаторов). Это
                // свойство анализа, а не фретборда — потому живёт в Controls.
                let mut settings = self.audio.analysis_settings();
                let a4 = SliderRow::new(
                    RowCaption::new("A4 pitch"),
                    &mut settings.concert_pitch_hz.0,
                    415.0..=466.0,
                    |v| format!("{v:.1} Hz"),
                )
                .width(180.0);
                if ui.add(a4).changed() {
                    self.audio.set_analysis_settings(settings);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    // Знаки альтерации: диезы (C#) или бемоли (Db) во ВСЕХ подписях
                    // нот приложения (улитки, ноты, резонаторы, тюнер, гриф).
                    ui.add(RowCaption::new("Accidentals"));
                    let mut settings = self.audio.analysis_settings();
                    let mut changed = false;
                    for (label, style) in [
                        ("Sharps (C#)", AccidentalStyle::Sharps),
                        ("Flats (Db)", AccidentalStyle::Flats),
                    ] {
                        let selected = settings.accidental == style;
                        let button = SegmentedButton::new(label, selected).min_width(84.0);
                        if ui.add(button).clicked() && settings.accidental != style {
                            settings.accidental = style;
                            changed = true;
                        }
                    }
                    if changed {
                        self.audio.set_analysis_settings(settings);
                    }
                });
                ui.add_space(8.0);
                Frame::new()
                    .fill(color::PLOT_BG)
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0_f32, color::FRAME_STROKE))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Signal path diagnostics")
                                .color(color::TEXT_CAPTION)
                                .strong()
                                .size(12.0),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "Input path: {}",
                                input_path_class_label(selected_input_id.as_deref())
                            ))
                            .color(color::TEXT_VALUE)
                            .size(12.0),
                        );
                        ui.label(
                            RichText::new(input_path_detail(selected_input_id.as_deref()))
                                .color(color::TEXT_HINT)
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(input_rate_debug_label(input_sample_rate))
                                .color(color::TEXT_HINT)
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
                            .color(color::TEXT_HINT)
                            .size(12.0)
                            .monospace(),
                        );
                        if output_has_bluetooth_risk(output_device_name.as_deref()) {
                            ui.label(
                                RichText::new(
                                    "Bluetooth output detected: playback monitor latency and clicks are much more likely.",
                                )
                                .color(color::STATUS_ERROR)
                                .size(12.0),
                            );
                        }
                    });
            });
    }

    pub(super) fn draw_fretboard_controls(&mut self, ui: &mut Ui) {
        let frame_width = ui.available_width();
        let (pad, radius) = theme::card_pad(ui.ctx(), 16, 18);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                // Inner margin + the 1 dp stroke, both sides: egui sizes a frame with
                // its stroke, so margin alone made the card 2 dp wider than its pane.
                ui.set_min_width(frame_width - pad.sum().x - 2.0);

                ui.horizontal_wrapped(|ui| {
                    let tuning =
                        PillCombo::new("tuning", self.tuning_kind.label()).caption(RowCaption::new("Tuning"));
                    tuning.show(ui, |ui| {
                        for tuning in ALL_TUNINGS {
                            ui.selectable_value(&mut self.tuning_kind, tuning, tuning.label());
                        }
                    });

                    // The caption and its seven pills as one item: on 360 dp "Root" was
                    // left at the end of the tuning's line, its notes on the next. The
                    // pills are a group of their own too: where caption and pills are
                    // wider than a line (≈355 dp, a 360 dp card has 314) the pills go
                    // under their caption whole, not six beside it and B alone below.
                    separated(ui, "root", |ui| {
                        ui.add(RowCaption::new("Root"));
                        keep_together(ui, "root_pills", |ui| {
                            // At the row's 10 dp spacing the seven pills alone are 316 dp,
                            // 2 more than that card; 6 dp between them is still a clear
                            // gap for a finger. Touch only: the desktop row is unchanged.
                            if theme::is_touch_screen(ui.ctx()) {
                                ui.spacing_mut().item_spacing.x = 6.0;
                            }
                            for (note, label) in ALL_ROOTS {
                                let button =
                                    SegmentedButton::new(label, self.root_note == note).min_width(30.0);
                                if ui.add(button).clicked() {
                                    self.root_note = note;
                                }
                            }
                        });
                    });

                    separated(ui, "scale", |ui| {
                        let scale = PillCombo::new("scale", self.scale_kind.label())
                            .caption(RowCaption::new("Scale"));
                        scale.show(ui, |ui| {
                            for scale in ALL_SCALES {
                                ui.selectable_value(&mut self.scale_kind, scale, scale.label());
                            }
                        });
                    });
                });
            });
    }

    pub(super) fn draw_general_config_card(&mut self, ui: &mut Ui) {
        self.draw_config_card(
            ui,
            "Config: General",
            "Window, smoothing, and analysis range",
            |settings, defaults| {
                settings.window_size = defaults.window_size;
                settings.spectrum_smoothing = defaults.spectrum_smoothing;
                settings.min_frequency = defaults.min_frequency;
                settings.max_frequency = defaults.max_frequency;
            },
            Self::draw_general_config_tab,
        );
    }

    pub(super) fn draw_fft1_config_card(&mut self, ui: &mut Ui) {
        self.draw_config_card(
            ui,
            "Config: FFT1",
            "Primary FFT size and display shaping",
            |settings, defaults| {
                settings.fft_size = defaults.fft_size;
                settings.note_spread = defaults.note_spread;
                settings.spectrum_gamma = defaults.spectrum_gamma;
                settings.note_gamma = defaults.note_gamma;
            },
            Self::draw_fft1_config_tab,
        );
    }

    pub(super) fn draw_resonator_fft_config_card(&mut self, ui: &mut Ui) {
        self.draw_config_card(
            ui,
            "Config: Resonator FFT",
            "Resonator bank range, density, and response",
            |settings, defaults| {
                settings.resonator.min_midi = defaults.resonator.min_midi;
                settings.resonator.max_midi = defaults.resonator.max_midi;
                settings.resonator.bins = defaults.resonator.bins;
                settings.resonator.alpha = defaults.resonator.alpha;
                settings.resonator.beta = defaults.resonator.beta;
                settings.resonator.gamma = defaults.resonator.gamma;
                settings.resonator.history = defaults.resonator.history;
                settings.resonator.update_ms = defaults.resonator.update_ms;
                settings.resonator.power = defaults.resonator.power;
                settings.resonator.reassign = defaults.resonator.reassign;
            },
            Self::draw_resonator_fft_config_tab,
        );
    }

    fn draw_config_card<Reset, Body>(
        &mut self,
        ui: &mut Ui,
        title: &str,
        subtitle: &str,
        reset: Reset,
        body: Body,
    ) where
        Reset: FnOnce(&mut AnalysisSettings, &AnalysisSettings),
        Body: FnOnce(&mut Self, &mut Ui, &mut AnalysisSettings, &mut bool),
    {
        let defaults = AnalysisSettings::default();
        let mut settings = self.audio.analysis_settings();
        let mut changed = false;
        let (pad, radius) = theme::card_pad(ui.ctx(), 14, 16);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::FRAME_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).color(color::TEXT_VALUE).strong());
                        ui.label(RichText::new(subtitle).color(color::TEXT_HINT).size(12.0));
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::finger_sized(ui, |ui| ui.button("Reset")).clicked() {
                            reset(&mut settings, &defaults);
                            changed = true;
                        }
                    });
                });

                ui.add_space(10.0);
                body(self, ui, &mut settings, &mut changed);
            });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    fn draw_general_config_tab(&mut self, ui: &mut Ui, settings: &mut AnalysisSettings, changed: &mut bool) {
        ui.horizontal_wrapped(|ui| {
            let window = PillCombo::new("analysis_window_size", format_sample_count(settings.window_size))
                .caption(RowCaption::new("Window"));
            window.show(ui, |ui| {
                for preset in WINDOW_SIZE_PRESETS {
                    if ui
                        .selectable_value(&mut settings.window_size, preset, format_sample_count(preset))
                        .changed()
                    {
                        *changed = true;
                    }
                }
            });

            let (moved, _) = separated(ui, "smooth", |ui| {
                let smooth = SliderRow::new(
                    RowCaption::new("Smooth"),
                    &mut settings.spectrum_smoothing,
                    0..=4,
                    |v| v.to_string(),
                )
                .width(120.0);
                ui.add(smooth).changed()
            });
            *changed |= moved;
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let min_hz = SliderRow::new(
                RowCaption::new("Min Hz"),
                &mut settings.min_frequency,
                16.0..=600.0,
                |v| format!("{v:.0}"),
            )
            .width(180.0)
            .logarithmic();
            *changed |= ui.add(min_hz).changed();

            ui.add_space(10.0);

            let max_hz = SliderRow::new(
                RowCaption::new("Max Hz"),
                &mut settings.max_frequency,
                300.0..=4_000.0,
                |v| format!("{v:.0}"),
            )
            .width(180.0)
            .logarithmic();
            *changed |= ui.add(max_hz).changed();
        });
    }

    fn draw_fft1_config_tab(&mut self, ui: &mut Ui, settings: &mut AnalysisSettings, changed: &mut bool) {
        ui.horizontal_wrapped(|ui| {
            let fft = PillCombo::new("analysis_fft_size", format_sample_count(settings.fft_size))
                .caption(RowCaption::new("FFT"));
            fft.show(ui, |ui| {
                for preset in FFT_SIZE_PRESETS {
                    if ui
                        .selectable_value(&mut settings.fft_size, preset, format_sample_count(preset))
                        .changed()
                    {
                        *changed = true;
                    }
                }
            });

            let (moved, _) = separated(ui, "spread", |ui| {
                let spread = SliderRow::new(
                    RowCaption::new("Note spread"),
                    &mut settings.note_spread,
                    0.15..=0.8,
                    |v| format!("{v:.2}"),
                )
                .width(150.0);
                ui.add(spread).changed()
            });
            *changed |= moved;
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let fft_gamma = SliderRow::new(
                RowCaption::new("FFT gamma"),
                &mut settings.spectrum_gamma,
                0.35..=1.2,
                |v| format!("{v:.2}"),
            );
            *changed |= ui.add(fft_gamma).changed();

            ui.add_space(10.0);

            let note_gamma = SliderRow::new(
                RowCaption::new("Note gamma"),
                &mut settings.note_gamma,
                0.35..=1.2,
                |v| format!("{v:.2}"),
            );
            *changed |= ui.add(note_gamma).changed();
        });
    }

    fn draw_resonator_fft_config_tab(
        &mut self,
        ui: &mut Ui,
        settings: &mut AnalysisSettings,
        changed: &mut bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            // Sliders bind a raw `u8` scratch; the validated `PNote` is rebuilt
            // on change (both ranges sit inside 0..=127, so `new` can't fail). The
            // readout spells the scratch, which is the note the slider stands on.
            let accidental = settings.accidental;
            let note_name = |midi: u8| accidental.midi_name(PNote::new(midi).unwrap());
            let mut min_midi = settings.resonator.min_midi.as_u8();
            // A sharp/flat name of octaves 0–8 is the widest the readout shows.
            const WIDEST_NOTE: &str = "C#0";
            let low = SliderRow::new(RowCaption::new("Range"), &mut min_midi, 12..=84, note_name)
                .width(140.0)
                .readout_sample(WIDEST_NOTE)
                .accessible_name("Range low");
            if ui.add(low).changed() {
                settings.resonator.min_midi = PNote::new(min_midi).unwrap();
                *changed = true;
            }

            // "to" alone names nothing: the slider says which end it is.
            let mut max_midi = settings.resonator.max_midi.as_u8();
            let to = RowCaption::new("to").color(color::TEXT_HINT);
            let high = SliderRow::new(to, &mut max_midi, 24..=108, note_name)
                .width(140.0)
                .readout_sample(WIDEST_NOTE)
                .accessible_name("Range high");
            if ui.add(high).changed() {
                settings.resonator.max_midi = PNote::new(max_midi).unwrap();
                *changed = true;
            }

            let (moved, _) = separated(ui, "bins", |ui| {
                let bins = SliderRow::new(
                    RowCaption::new("Bins / semitone"),
                    &mut settings.resonator.bins,
                    1..=12,
                    |v| v.to_string(),
                )
                .width(120.0);
                ui.add(bins).changed()
            });
            *changed |= moved;
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            // Логарифмический: перцептуально интересна нижняя декада
            // (0.001..0.1 — длинный «звон»), на линейной шкале она
            // занимала бы <1% хода слайдера.
            let alpha = SliderRow::new(
                RowCaption::new("Alpha"),
                &mut settings.resonator.alpha,
                0.001..=12.0,
                |v| format!("{v:.3}"),
            )
            .width(150.0)
            .logarithmic();
            *changed |= ui.add(alpha).changed();

            ui.add_space(10.0);

            let beta = SliderRow::new(
                RowCaption::new("Beta"),
                &mut settings.resonator.beta,
                0.001..=12.0,
                |v| format!("{v:.3}"),
            )
            .width(150.0)
            .logarithmic();
            *changed |= ui.add(beta).changed();

            ui.add_space(10.0);

            let gamma = SliderRow::new(
                RowCaption::new("Gamma"),
                &mut settings.resonator.gamma,
                0.15..=2.4,
                |v| format!("{v:.2}"),
            )
            .width(150.0);
            *changed |= ui.add(gamma).changed();
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(RowCaption::new("Mode"));
            for (label, power) in [("Magnitude", false), ("Power", true)] {
                let selected = settings.resonator.power == power;
                let button = SegmentedButton::new(label, selected).min_width(82.0);
                if ui.add(button).clicked() && settings.resonator.power != power {
                    settings.resonator.power = power;
                    *changed = true;
                }
            }

            ui.add_space(10.0);

            let history = SliderRow::new(
                RowCaption::new("History"),
                &mut settings.resonator.history,
                8..=240,
                |v| v.to_string(),
            )
            .width(120.0);
            *changed |= ui.add(history).changed();

            ui.add_space(10.0);

            let update = SliderRow::new(
                RowCaption::new("Update"),
                &mut settings.resonator.update_ms,
                8..=80,
                |v| format!("{v} ms"),
            )
            .width(120.0);
            *changed |= ui.add(update).changed();
        });

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(RowCaption::new("Detector"));
            // Which frontend decides the played note. The bank keeps running either way (it
            // draws the waterfall, the spiral and the salience heat); this only redirects the
            // note decision. See `audio::types::PitchFrontend`.
            for frontend in PitchFrontend::ALL {
                let selected = settings.resonator.frontend == frontend;
                let button = SegmentedButton::new(frontend.label(), selected).min_width(82.0);
                if ui.add(button).on_hover_text(frontend.description()).clicked()
                    && settings.resonator.frontend != frontend
                {
                    settings.resonator.frontend = frontend;
                    *changed = true;
                }
            }
        });

        ui.label(
            RichText::new(settings.resonator.frontend.description())
                .small()
                .color(color::TEXT_CAPTION),
        );

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let reassign = theme::finger_sized(ui, |ui| {
                ui.checkbox(
                    &mut settings.resonator.reassign,
                    RichText::new("Δφ reassign").color(color::TEXT_CAPTION).strong(),
                )
            });
            if reassign
                .on_hover_text(
                    "Place each partial at its true frequency via instantaneous-frequency \
                     (Δφ) reassignment, with a coherence gate that suppresses the \
                     negative-frequency image and noise. Off = plain per-bin magnitude \
                     at the nominal pitch (original behaviour).",
                )
                .changed()
            {
                *changed = true;
            }
        });

        // Collapsed crib sheet for every knob above: costs one header line until
        // opened. Lives here (desktop tab) only — the Android strip in
        // `resonator_panel.rs` has no room for it.
        ui.add_space(10.0);
        egui::CollapsingHeader::new(RichText::new("What do these knobs do?").color(color::TEXT_HINT))
            .default_open(false)
            .show(ui, |ui| {
                for (name, text) in RESONATOR_PARAM_HELP {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new(*name).color(color::TEXT_CAPTION).strong());
                        ui.label(RichText::new(*text).color(color::TEXT_HINT));
                    });
                    ui.add_space(4.0);
                }
            });
    }
}

// Plain-English help for the resonator config tab, one entry per knob, in the
// same order they appear on screen.
const RESONATOR_PARAM_HELP: &[(&str, &str)] = &[
    (
        "Range",
        "Pitch span the resonator bank covers. Every semitone in the span gets \
         its own resonators, so widening the range adds bins (CPU).",
    ),
    (
        "Bins / semitone",
        "Resonator density on the nominal pitch grid. More bins resolve closely \
         spaced partials better but cost CPU. With Δφ reassign on, the on-screen \
         grid is already finer than the bank, so this mostly affects tracking, \
         not display resolution.",
    ),
    (
        "Alpha",
        "Bandwidth vs. ring time of each resonator (scales the input-stage \
         filter coefficient; ring time grows as 1/Alpha, slider is logarithmic, \
         1.0 = the per-frequency default). Lower = narrower band, longer ring: \
         notes sustain on screen and pitch sharpens, but response slows — at A4 \
         roughly 6 ms at 1.0, 0.6 s at 0.01, 6 s at 0.001. Higher = snappier \
         attack, blurrier pitch.",
    ),
    (
        "Beta",
        "Smoothing of the displayed envelope (the second filter stage), scaled \
         the same way as Alpha. Lower = bars rise and decay smoothly and linger; \
         higher = they follow the resonator immediately and jitter more.",
    ),
    (
        "Gamma",
        "Display contrast. Each frame is normalised to its loudest bin, then \
         raised to this power: below 1 lifts quiet bins, above 1 keeps only the \
         peaks.",
    ),
    (
        "Mode",
        "Magnitude draws linear amplitude; Power draws its square, stretching \
         the contrast between strong and weak partials.",
    ),
    (
        "History",
        "How many published frames the waterfall / spiral trail keeps. Trail \
         duration ≈ History × Update.",
    ),
    (
        "Update",
        "How often a fresh frame is published to the UI, in milliseconds. Lower \
         = smoother motion, slightly more work per second.",
    ),
    (
        "Δφ reassign",
        "Re-places each bin's energy at its measured instantaneous frequency: \
         sub-bin pitch accuracy, plus a coherence gate that mutes noise and \
         mirror images. Off = plain per-bin magnitudes at nominal pitch.",
    ),
];
