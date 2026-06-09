use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    FontId,
    Frame,
    Margin,
    Rect,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use super::{
    App,
    LiveChartKind,
    TunerTarget,
    audio_status_color,
    audio_status_label,
    cents_color,
    input_level_label,
    input_source_debug_label,
    pill,
    spectrum_color,
    waiting_prompt,
};
use crate::audio::{
    AudioInputKind,
    TunerReading,
};
use crate::core_types::note::Accidental;
use crate::core_types::pitch::PCNote;
use crate::ui::snail::{
    self,
    SpiralChart,
};
use crate::ui::theme::PANEL_FILL;
use crate::ui::waterfall;

impl App {
    pub(super) fn draw_input_scope_card(&mut self, ui: &mut Ui) {
        let status = self.audio.status();
        let input_level = self.audio.input_level();
        let waveform = self.audio.input_waveform();
        let selected_input_id = self.audio.selected_input_id();
        let selected_input_kind = self.selected_input_kind(selected_input_id.as_deref());

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Input Scope")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            egui::RichText::new(audio_status_label(&status, selected_input_kind))
                                .color(audio_status_color(&status)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        pill(
                            ui,
                            &format!("{} samples", waveform.len()),
                            Color32::from_rgb(201, 195, 184),
                            Color32::from_rgb(64, 68, 73),
                        );
                    });
                });

                ui.add_space(12.0);
                self.draw_input_level(ui, input_level, selected_input_kind);
                ui.add_space(12.0);
                self.draw_input_scope_panel(ui, &waveform);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(input_source_debug_label(selected_input_id.as_deref()))
                        .color(Color32::from_rgb(145, 151, 160))
                        .size(12.0)
                        .monospace(),
                );
            });
    }

    pub(super) fn draw_tuner_card(&mut self, ui: &mut Ui) {
        let status = self.audio.status();
        let reading = self.audio.reading();
        let input_level = self.audio.input_level();
        let selected_input_id = self.audio.selected_input_id();
        let selected_input_kind = self.selected_input_kind(selected_input_id.as_deref());
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_targets = self.filtered_tuner_targets(&tuning, &scale);
        let target = tuner_targets.first();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Live analysis")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            egui::RichText::new(audio_status_label(&status, selected_input_kind))
                                .color(audio_status_color(&status)),
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
                                    1.0_f32,
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
                self.draw_input_level(ui, input_level, selected_input_kind);
                ui.add_space(12.0);
                match self.live_chart {
                    LiveChartKind::Tuner => self.draw_tuner_meter(ui, target, selected_input_kind),
                    LiveChartKind::Fft => self.draw_spectrum(ui, reading.as_ref()),
                    LiveChartKind::Spiral => self.draw_spiral_spectrogram(ui, reading.as_ref()),
                }
            });
    }

    pub(super) fn draw_input_level(&self, ui: &mut Ui, input_level: f32, input_kind: AudioInputKind) {
        let desired_size = vec2(ui.available_width(), 28.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 14.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
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
            input_level_label(input_kind),
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

    fn draw_input_scope_panel(&self, ui: &mut Ui, waveform: &[f32]) {
        let available_size = ui.available_size_before_wrap();
        let desired_size = vec2(available_size.x, available_size.y.max(220.0));
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        painter.text(
            pos2(rect.left() + 14.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "Raw waveform preview",
            FontId::proportional(15.0),
            Color32::from_rgb(201, 195, 184),
        );
        painter.text(
            pos2(rect.right() - 14.0, rect.top() + 12.0),
            egui::Align2::RIGHT_TOP,
            "recent mono samples after input gain",
            FontId::proportional(12.0),
            Color32::from_rgb(152, 158, 165),
        );

        let plot_rect = Rect::from_min_max(
            pos2(rect.left() + 14.0, rect.top() + 42.0),
            pos2(rect.right() - 14.0, rect.bottom() - 14.0),
        );
        let center_y = plot_rect.center().y;
        painter.line_segment(
            [
                pos2(plot_rect.left(), center_y),
                pos2(plot_rect.right(), center_y),
            ],
            Stroke::new(1.0_f32, Color32::from_rgb(70, 75, 83)),
        );

        if waveform.len() < 2 {
            painter.text(
                plot_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for input samples",
                FontId::proportional(14.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        }

        let points: Vec<_> = waveform
            .iter()
            .enumerate()
            .map(|(index, sample)| {
                let x = egui::remap(
                    index as f32,
                    0.0..=(waveform.len().saturating_sub(1) as f32),
                    plot_rect.left()..=plot_rect.right(),
                );
                let y = egui::remap(
                    sample.clamp(-1.0, 1.0),
                    -1.0..=1.0,
                    plot_rect.bottom()..=plot_rect.top(),
                );
                pos2(x, y)
            })
            .collect();

        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.5_f32, Color32::from_rgb(120, 204, 238)),
        ));
    }

    fn draw_tuner_meter(&self, ui: &mut Ui, reading: Option<&TunerTarget>, input_kind: AudioInputKind) {
        let desired_size = vec2(ui.available_width(), 120.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        let center_x = rect.center().x;
        let meter_y = rect.bottom() - 30.0;
        painter.line_segment(
            [
                pos2(rect.left() + 18.0, meter_y),
                pos2(rect.right() - 18.0, meter_y),
            ],
            Stroke::new(2.0_f32, Color32::from_rgb(89, 92, 98)),
        );

        for cents in [-50.0_f32, -25.0, 0.0, 25.0, 50.0] {
            let x = egui::remap(cents, -50.0..=50.0, (rect.left() + 22.0)..=(rect.right() - 22.0));
            let height = if cents == 0.0 { 18.0 } else { 10.0 };
            painter.line_segment(
                [pos2(x, meter_y - height), pos2(x, meter_y + 2.0)],
                Stroke::new(1.0_f32, Color32::from_rgb(117, 122, 128)),
            );
        }

        match reading {
            Some(reading) => {
                painter.text(
                    pos2(rect.left() + 18.0, rect.top() + 18.0),
                    egui::Align2::LEFT_TOP,
                    reading.note_name.name(),
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
                    Stroke::new(3.0_f32, cents_color(cents)),
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
                    format!("S{} • F{}", reading.string.0, reading.fret.0),
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
                    waiting_prompt(input_kind),
                    FontId::proportional(13.0),
                    Color32::from_rgb(139, 143, 149),
                );
            }
        }

        painter.line_segment(
            [pos2(center_x, meter_y - 24.0), pos2(center_x, meter_y + 6.0)],
            Stroke::new(1.0_f32, Color32::from_rgb(177, 167, 150)),
        );
    }

    fn draw_spectrum(&self, ui: &mut Ui, reading: Option<&TunerReading>) {
        let desired_size = vec2(ui.available_width(), 244.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(29, 32, 37));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        painter.text(
            pos2(rect.left() + 14.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "Spectrum + note waterfall",
            FontId::proportional(15.0),
            Color32::from_rgb(201, 195, 184),
        );

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
            waterfall::draw_waterfall(&painter, freq_rect, &reading.waterfall);

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
            waterfall::draw_note_waterfall(
                &painter,
                note_rect,
                &reading.note_waterfall,
                &reading.note_labels,
                None,
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
        snail::draw_spiral_chart(
            ui,
            SpiralChart {
                title:             "Spiral spectrogram",
                subtitle:          "octaves wrap onto the same pitch angle",
                spectrum:          reading.map(|value| value.spiral_spectrum.as_slice()),
                waterfall:         reading.map_or(&[][..], |value| value.spiral_waterfall.as_slice()),
                note_labels:       reading.map_or(&[][..], |value| value.note_labels.as_slice()),
                active_note:       None,
                waiting_message:   "Play a sustained note to light up the spiral",
                empty_message:     "The note spectrum is empty",
                active_note_label: "active note",
            },
            &self.audio.analysis_settings(),
        );
    }
}
