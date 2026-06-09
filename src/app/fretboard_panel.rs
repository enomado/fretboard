use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    FontId,
    Frame,
    Margin,
    Rect,
    RichText,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use super::{
    App,
    FRETBOARD_HEIGHT,
    FRETBOARD_MARGIN_BOTTOM,
    FRETBOARD_MARGIN_LEFT,
    FRETBOARD_MARGIN_RIGHT,
    FRETBOARD_MARGIN_TOP,
    HoveredNote,
    ResonatorTarget,
    TunerTarget,
    frequency_to_midi,
    midi_to_frequency,
    rangef_to_range,
};
use crate::core_types::note::Accidental;
use crate::core_types::pitch::{
    Interval,
    PCNote,
};
use crate::core_types::scale::Scale;
use crate::core_types::tuning::{
    Fret,
    GString,
    Tuning,
};
use crate::fretboard::{
    FretConfig,
    Fretboard,
};
use crate::ui::theme::{
    PANEL_FILL,
    fretboard_fill,
};
use crate::ui::{
    draw_fret_lines,
    draw_fretboard_scale,
    draw_positions,
    draw_string_lines_scale,
};

impl App {
    pub(super) fn draw_fretboard_card(&self, ui: &mut Ui) {
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_targets = self.filtered_tuner_targets(&tuning, &scale);
        let resonator_targets = self.resonator_fretboard_targets(&tuning);

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Scale tones: {}", scale.notes().len()))
                            .color(Color32::from_rgb(143, 150, 160)),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Resonator notes: {}", resonator_targets.len()))
                            .color(Color32::from_rgb(143, 150, 160)),
                    );
                    ui.separator();
                    ui.label(RichText::new("Visible frets: 1-18").color(Color32::from_rgb(143, 150, 160)));
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
                    Stroke::new(1.0_f32, Color32::from_rgb(112, 88, 66)),
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
                if !resonator_targets.is_empty() {
                    self.draw_resonator_targets(&painter, &fretboard, &resonator_targets);
                }
                draw_fretboard_scale(painter.clone(), &fretboard, &scale);
                draw_positions(&painter, fretboard_rect, &fretboard);
                if !tuner_targets.is_empty() {
                    self.draw_tuner_targets(&painter, &fretboard, &tuner_targets);
                }
                if let Some(pointer_pos) = response.hover_pos()
                    && let Some(hovered) = self.hovered_note(pointer_pos, &fretboard, &scale)
                {
                    self.draw_hovered_note(&painter, component_rect, &hovered);
                }

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
            Stroke::new(2.0_f32, Color32::from_rgb(214, 200, 182)),
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
            Stroke::new(1.0_f32, Color32::from_rgb(88, 92, 98)),
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

    pub(super) fn filtered_tuner_targets(&self, tuning: &Tuning, _scale: &Scale) -> Vec<TunerTarget> {
        let Some(reading) = self.audio.reading() else {
            return Vec::new();
        };
        let detected_midi = frequency_to_midi(reading.frequency_hz).round() as u8;
        let detected_frequency = midi_to_frequency(detected_midi as f32);
        let cents = 1200.0 * (reading.frequency_hz / detected_frequency).log2();
        let mut matches = Vec::new();

        for string in 1..=tuning.string_count() {
            let open = tuning.note(GString(string));
            for fret in 0..=18 {
                let note = open.add(Interval(fret as i32));
                if note.as_u8() != detected_midi {
                    continue;
                }

                matches.push(TunerTarget {
                    string,
                    fret,
                    note_name: note.to_anote().name(),
                    frequency_hz: reading.frequency_hz,
                    cents,
                });
            }
        }

        matches.sort_by_key(|target| (target.fret, target.string));
        matches
    }

    fn resonator_fretboard_targets(&self, tuning: &Tuning) -> Vec<ResonatorTarget> {
        let Some(reading) = self.audio.resonator_reading() else {
            return Vec::new();
        };
        if reading.spectrum.is_empty() || reading.note_labels.is_empty() {
            return Vec::new();
        }

        let note_strengths = resonator_note_strengths(&reading.spectrum, reading.note_labels.len());
        let peak = note_strengths.iter().copied().fold(0.0_f32, f32::max);
        if peak <= 0.0 {
            return Vec::new();
        }

        let mean = note_strengths.iter().copied().sum::<f32>() / note_strengths.len().max(1) as f32;
        let threshold = (mean * 1.75 + 0.06).clamp(0.18, 0.52).min(peak * 0.92);
        let min_midi = self.audio.analysis_settings().resonator_min_midi;
        let mut matches = Vec::new();

        for string in 1..=tuning.string_count() {
            let open = tuning.note(GString(string));
            for fret in 0..=18 {
                let note = open.add(Interval(fret as i32));
                let midi = note.as_u8() as usize;
                let Some(note_index) = midi.checked_sub(min_midi) else {
                    continue;
                };
                let Some(strength) = note_strengths.get(note_index).copied() else {
                    continue;
                };
                if strength < threshold {
                    continue;
                }

                let normalized = if peak > threshold {
                    ((strength - threshold) / (peak - threshold)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                matches.push(ResonatorTarget {
                    string,
                    fret,
                    strength: normalized,
                });
            }
        }

        matches.sort_by(|left, right| {
            right
                .strength
                .total_cmp(&left.strength)
                .then_with(|| left.fret.cmp(&right.fret))
                .then_with(|| left.string.cmp(&right.string))
        });
        matches
    }

    fn draw_resonator_targets(
        &self,
        painter: &egui::Painter,
        fretboard: &Fretboard,
        targets: &[ResonatorTarget],
    ) {
        for target in targets {
            let center = pos2(
                fretboard.fret_pos(Fret(target.fret)),
                fretboard.string_pos(GString(target.string)),
            );
            let strength = target.strength.clamp(0.0, 1.0);
            let glow_alpha = (34.0 + strength * 96.0).round() as u8;
            let core_alpha = (80.0 + strength * 150.0).round() as u8;
            let radius = 12.0 + strength * 8.0;
            let glow = Color32::from_rgba_unmultiplied(239, 167, 102, glow_alpha);
            let core = Color32::from_rgba_unmultiplied(255, 213, 149, core_alpha);

            painter.circle_filled(center, radius + 7.0, glow);
            painter.circle_stroke(center, radius, Stroke::new(1.4 + strength * 1.2, core));
            painter.circle_filled(center, 2.5 + strength * 2.0, core);
        }
    }

    fn draw_tuner_targets(&self, painter: &egui::Painter, fretboard: &Fretboard, targets: &[TunerTarget]) {
        for target in targets {
            let center = pos2(
                fretboard.fret_pos(Fret(target.fret)),
                fretboard.string_pos(GString(target.string)),
            );
            painter.circle_stroke(
                center,
                18.0,
                Stroke::new(2.0_f32, Color32::from_rgb(216, 205, 187)),
            );
            painter.circle_stroke(
                center,
                24.0,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(216, 205, 187, 96)),
            );
        }
    }
}

fn resonator_note_strengths(spectrum: &[f32], note_count: usize) -> Vec<f32> {
    if spectrum.is_empty() || note_count == 0 {
        return Vec::new();
    }
    if note_count == 1 {
        return vec![spectrum.iter().copied().fold(0.0_f32, f32::max)];
    }

    let bins_per_note = (spectrum.len().saturating_sub(1) as f32 / (note_count - 1) as f32).max(1.0);

    (0..note_count)
        .map(|note_index| {
            let center = note_index as f32 * bins_per_note;
            let start = (center - bins_per_note * 0.5)
                .floor()
                .max(0.0) as usize;
            let end = (center + bins_per_note * 0.5)
                .ceil()
                .min(spectrum.len().saturating_sub(1) as f32) as usize;

            spectrum[start..=end].iter().copied().fold(0.0_f32, f32::max)
        })
        .collect()
}
