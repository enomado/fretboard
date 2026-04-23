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
    pill,
    spectrum_color,
};
use crate::audio::TunerReading;
use crate::ui::theme::PANEL_FILL;

impl App {
    pub(super) fn draw_resonator_snail_card(&mut self, ui: &mut Ui) {
        let reading = self.audio.reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonators")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Alexandre Francois's Resonate bank, streamed into our pitch spiral",
                            )
                            .color(Color32::from_rgb(152, 158, 165)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(
                                ui,
                                &format!("{} bins", reading.resonator_spectrum.len()),
                                Color32::from_rgb(201, 195, 184),
                                Color32::from_rgb(64, 68, 73),
                            );
                        } else {
                            pill(
                                ui,
                                "waiting for input",
                                Color32::from_rgb(184, 188, 196),
                                Color32::from_rgb(56, 61, 68),
                            );
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_spiral_chart(
                    ui,
                    "Resonator spiral",
                    "same snail, but driven by the resonator bank instead of FFT bins",
                    reading_ref.map(|value| value.resonator_spectrum.as_slice()),
                    reading_ref.map_or(&[][..], |value| value.resonator_waterfall.as_slice()),
                    reading_ref.map_or(&[][..], |value| value.resonator_note_labels.as_slice()),
                    None,
                    "Play a sustained note to charge the resonator bank",
                    "The resonator bank is empty",
                    "bank focus",
                );
            });
    }

    pub(super) fn draw_resonator_bank_card(&mut self, ui: &mut Ui) {
        let reading = self.audio.reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonator Bank")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            egui::RichText::new("Continuous resonator state and current magnitudes")
                                .color(Color32::from_rgb(152, 158, 165)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(
                                ui,
                                &format!("{} bins", reading.resonator_spectrum.len()),
                                Color32::from_rgb(201, 195, 184),
                                Color32::from_rgb(64, 68, 73),
                            );
                        } else {
                            pill(
                                ui,
                                "waiting for input",
                                Color32::from_rgb(184, 188, 196),
                                Color32::from_rgb(56, 61, 68),
                            );
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_resonator_bank_panel(ui, reading_ref);
            });
    }

    fn draw_resonator_bank_panel(&self, ui: &mut Ui, reading: Option<&TunerReading>) {
        let available_size = ui.available_size_before_wrap();
        let desired_size = vec2(available_size.x, available_size.y.max(244.0));
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
            "Bank waterfall + current frame",
            FontId::proportional(15.0),
            Color32::from_rgb(201, 195, 184),
        );
        painter.text(
            pos2(rect.right() - 14.0, rect.top() + 12.0),
            egui::Align2::RIGHT_TOP,
            "continuous resonator state, no FFT window grid",
            FontId::proportional(12.0),
            Color32::from_rgb(152, 158, 165),
        );

        let content_top = rect.top() + 42.0;
        let content_bottom = rect.bottom() - 14.0;
        let content_height = content_bottom - content_top;
        let bars_height = (content_height * 0.28).clamp(46.0, 92.0);
        let waterfall_height = (content_height - bars_height - 26.0).max(84.0);
        let waterfall_rect = Rect::from_min_max(
            pos2(rect.left() + 14.0, content_top),
            pos2(rect.right() - 14.0, content_top + waterfall_height),
        );
        let bars_rect = Rect::from_min_max(
            pos2(rect.left() + 14.0, waterfall_rect.bottom() + 26.0),
            pos2(rect.right() - 14.0, content_bottom),
        );

        let Some(reading) = reading else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "The resonator bank starts filling as soon as the tuner locks onto audio",
                FontId::proportional(13.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        };

        if reading.resonator_spectrum.is_empty() || reading.resonator_note_labels.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No resonator bins available for the current frame",
                FontId::proportional(13.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        }

        let bins_per_label = if reading.resonator_note_labels.len() > 1 {
            (reading.resonator_spectrum.len().saturating_sub(1) as f32
                / (reading.resonator_note_labels.len() - 1) as f32)
                .max(1.0)
        } else {
            1.0
        };
        self.draw_pitch_labeled_waterfall(
            &painter,
            waterfall_rect,
            &reading.resonator_waterfall,
            &reading.resonator_note_labels,
            bins_per_label,
            Some(reading.note_name.as_str()),
        );

        let bar_width = bars_rect.width() / reading.resonator_spectrum.len().max(1) as f32;
        for (index, value) in reading.resonator_spectrum.iter().enumerate() {
            let x0 = bars_rect.left() + index as f32 * bar_width;
            let x1 = x0 + bar_width.max(1.0);
            let height = bars_rect.height() * value.clamp(0.0, 1.0);
            let bar_rect = Rect::from_min_max(
                pos2(x0, bars_rect.bottom() - height),
                pos2(x1, bars_rect.bottom()),
            );
            painter.rect_filled(bar_rect, 0.0, spectrum_color(*value));
        }

        painter.text(
            pos2(bars_rect.left(), bars_rect.top() - 12.0),
            egui::Align2::LEFT_BOTTOM,
            "current resonator magnitudes",
            FontId::proportional(11.0),
            Color32::from_rgb(152, 158, 165),
        );
    }

    pub(super) fn draw_resonator_waterfall_card(&mut self, ui: &mut Ui) {
        let reading = self.audio.reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonator Waterfall")
                                .size(20.0)
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        ui.label(
                            egui::RichText::new("A full-screen spectrogram from the new resonator bank")
                                .color(Color32::from_rgb(152, 158, 165)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(
                                ui,
                                &format!("focus {}", reading.note_name),
                                Color32::from_rgb(224, 214, 198),
                                Color32::from_rgb(78, 61, 54),
                            );
                            ui.add_space(8.0);
                            pill(
                                ui,
                                &format!("{} frames", reading.resonator_waterfall.len()),
                                Color32::from_rgb(201, 195, 184),
                                Color32::from_rgb(64, 68, 73),
                            );
                        } else {
                            pill(
                                ui,
                                "waiting for input",
                                Color32::from_rgb(184, 188, 196),
                                Color32::from_rgb(56, 61, 68),
                            );
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_resonator_waterfall_panel(ui, reading_ref);
            });
    }

    fn draw_resonator_waterfall_panel(&self, ui: &mut Ui, reading: Option<&TunerReading>) {
        let available_size = ui.available_size_before_wrap();
        let desired_size = vec2(available_size.x, available_size.y.max(260.0));
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, Color32::from_rgb(20, 23, 29));
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, Color32::from_rgb(72, 76, 82)),
            egui::StrokeKind::Inside,
        );

        let Some(reading) = reading else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Play audio to fill the resonator waterfall",
                FontId::proportional(15.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        };

        if reading.resonator_waterfall.is_empty() || reading.resonator_note_labels.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for the resonator bank to accumulate frames",
                FontId::proportional(15.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        }

        let chart_rect = Rect::from_min_max(
            pos2(rect.left() + 16.0, rect.top() + 20.0),
            pos2(rect.right() - 16.0, rect.bottom() - 44.0),
        );

        let bins_per_label = if reading.resonator_note_labels.len() > 1 {
            (reading.resonator_spectrum.len().saturating_sub(1) as f32
                / (reading.resonator_note_labels.len() - 1) as f32)
                .max(1.0)
        } else {
            1.0
        };
        self.draw_pitch_labeled_waterfall(
            &painter,
            chart_rect,
            &reading.resonator_waterfall,
            &reading.resonator_note_labels,
            bins_per_label,
            Some(reading.note_name.as_str()),
        );

        painter.text(
            pos2(rect.left() + 18.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "newest frame at the bottom, resonator bins across the pitch axis",
            FontId::proportional(12.0),
            Color32::from_rgb(166, 170, 176),
        );
        painter.text(
            pos2(rect.right() - 18.0, rect.bottom() - 14.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{} active bins", reading.resonator_spectrum.len()),
            FontId::proportional(12.0),
            Color32::from_rgb(166, 170, 176),
        );
    }
}
