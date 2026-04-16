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
    SPIRAL_PITCH_LABELS,
    TunerTarget,
    audio_status_color,
    audio_status_label,
    cents_color,
    degree_suffix,
    input_level_label,
    pill,
    pitch_class_angle,
    pitch_class_color,
    spectrum_color,
    spiral_contrast_strengths,
    spiral_note_color,
    spiral_point_fractional,
    waiting_prompt,
    waterfall_color,
};
use crate::audio::{
    AnalysisSettings,
    AudioInputKind,
    TunerReading,
};
use crate::core_types::note::Accidental;
use crate::core_types::pitch::PCNote;
use crate::ui::theme::PANEL_FILL;

impl App {
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
            .stroke(Stroke::new(1.0, Color32::from_rgb(61, 66, 74)))
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
                self.draw_input_level(ui, input_level, selected_input_kind);
                ui.add_space(12.0);
                match self.live_chart {
                    LiveChartKind::Tuner => self.draw_tuner_meter(ui, target, selected_input_kind),
                    LiveChartKind::Fft => self.draw_spectrum(ui, target, reading.as_ref()),
                    LiveChartKind::Spiral => self.draw_spiral_spectrogram(ui, reading.as_ref()),
                }
            });
    }

    fn draw_input_level(&self, ui: &mut Ui, input_level: f32, input_kind: AudioInputKind) {
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

    fn draw_tuner_meter(&self, ui: &mut Ui, reading: Option<&TunerTarget>, input_kind: AudioInputKind) {
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
                    waiting_prompt(input_kind),
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
        let desired_size = vec2(ui.available_width().max(280.0), 244.0);
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
            let settings: AnalysisSettings = self.audio.analysis_settings();
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
}
