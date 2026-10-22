use eframe::egui::{
    self,
    Color32,
    FontId,
    Frame,
    Rect,
    RichText,
    Sense,
    Stroke,
    Ui,
    Vec2,
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
};
use crate::core_types::note::Accidental;
use crate::core_types::pitch::{
    Midi,
    PCNote,
};
use crate::core_types::scale::Scale;
use crate::core_types::tuning::{
    Fret,
    Tuning,
};
use crate::ui::fretboard::draw::{
    draw_fret_lines,
    draw_fretboard_scale,
    draw_string_lines_scale,
};
use crate::ui::fretboard::geometry::{
    FretConfig,
    Fretboard,
    NeckAxis,
};
use crate::ui::fretboard::positions::{
    VERTICAL_REACH,
    draw_positions,
};
use crate::ui::theme::{
    card_pad,
    fretboard_fill,
};
use crate::ui::tokens::color;
use crate::ui::wrap_row::separated;

impl App {
    pub(super) fn draw_fretboard_card(&self, ui: &mut Ui) {
        let tuning = self.tuning_kind.to_tuning();
        let root_pc = PCNote::from_note(self.root_note, Accidental::Natural);
        let scale = self.scale_kind.to_scale(root_pc);
        let tuner_targets = self.filtered_tuner_targets(&tuning, &scale);
        let resonator_targets = self.resonator_fretboard_targets(&tuning);
        let accidental = self.audio.analysis_settings().accidental;

        let (pad, radius) = card_pad(ui.ctx(), 14, 22);
        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                // Wraps on a phone: the three counters are ~470 dp in one row. Each
                // counter moves to the next row whole, not broken mid-phrase, and
                // the divider before it only where it stays on the line.
                ui.horizontal_wrapped(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(
                        RichText::new(format!("Scale tones: {}", scale.notes().len()))
                            .color(color::TEXT_HINT),
                    );
                    separated(ui, "resonator_notes", |ui| {
                        ui.label(
                            RichText::new(format!("Resonator notes: {}", resonator_targets.len()))
                                .color(color::TEXT_HINT),
                        );
                    });
                    separated(ui, "visible_frets", |ui| {
                        ui.label(RichText::new("Visible frets: 1-18").color(color::TEXT_HINT));
                    });
                });

                ui.add_space(10.0);

                let layout = NeckLayout::for_width(ui.available_width());
                let (component_rect, response) = ui.allocate_exact_size(layout.size, Sense::hover());
                // Named for accessibility, like the snail's spiral; also how a test
                // finds which way the neck was laid out.
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Fretboard"));

                let painter = ui.painter_at(component_rect);
                painter.rect_filled(component_rect, 20.0, fretboard_fill());

                let fretboard_rect = layout.neck_rect(component_rect);

                painter.rect_stroke(
                    fretboard_rect,
                    18.0,
                    Stroke::new(1.0_f32, color::ACCENT_FILL),
                    egui::StrokeKind::Inside,
                );

                let fretboard = Fretboard::new(
                    fretboard_rect,
                    layout.axis,
                    FretConfig::Log,
                    tuning,
                    VISIBLE_FRETS,
                );

                draw_fret_lines(&painter, fretboard_rect, &fretboard);
                draw_string_lines_scale(&painter, fretboard_rect, &fretboard, &scale, accidental);
                if !resonator_targets.is_empty() {
                    self.draw_resonator_targets(&painter, &fretboard, &resonator_targets);
                }
                draw_fretboard_scale(painter.clone(), &fretboard, &scale, accidental);
                draw_positions(&painter, &fretboard);
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
            color::TEXT_MUTED,
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
                let center = fretboard.note_center(string, fret);
                let rect = Rect::from_center_size(center, vec2(34.0, 22.0));

                if rect.contains(pointer_pos) {
                    let note = fretboard.tuning.note(string).add(fret.semitones());
                    let degree = scale.degree(note.to_pc().1);

                    return Some(HoveredNote {
                        string,
                        fret,
                        note_name: note.to_anote(),
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
            Stroke::new(2.0_f32, color::MARKER),
            egui::StrokeKind::Outside,
        );
        painter.circle_filled(hovered.center, 3.0, color::MARKER);

        let degree_label = hovered
            .degree
            .map(|degree| format!("degree {}", degree.0))
            .unwrap_or_else(|| "outside scale".to_owned());

        let tooltip_rect = Rect::from_min_size(
            pos2(component_rect.left() + 14.0, component_rect.top() + 14.0),
            vec2(200.0, 58.0),
        );

        painter.rect_filled(
            tooltip_rect,
            14.0,
            color::PANEL_FILL.gamma_multiply(236.0 / 255.0),
        );
        painter.rect_stroke(
            tooltip_rect,
            14.0,
            Stroke::new(1.0_f32, color::IDLE_STROKE),
            egui::StrokeKind::Inside,
        );
        painter.text(
            pos2(tooltip_rect.left() + 12.0, tooltip_rect.top() + 11.0),
            egui::Align2::LEFT_TOP,
            hovered
                .note_name
                .name_styled(self.audio.analysis_settings().accidental),
            FontId::proportional(17.0),
            color::TEXT_HEADING,
        );
        painter.text(
            pos2(tooltip_rect.left() + 12.0, tooltip_rect.top() + 34.0),
            egui::Align2::LEFT_TOP,
            format!(
                "string {}  •  fret {}  •  {}",
                hovered.string.0, hovered.fret.0, degree_label
            ),
            FontId::proportional(12.0),
            color::TEXT_HINT,
        );
    }

    pub(super) fn filtered_tuner_targets(&self, tuning: &Tuning, _scale: &Scale) -> Vec<TunerTarget> {
        let Some(reading) = self.audio.reading() else {
            return Vec::new();
        };
        let reference_hz = self.audio.analysis_settings().concert_pitch_hz;
        let detected_midi = reading.frequency_hz.to_midi(reference_hz).0.round() as u8;
        let detected_frequency = Midi(detected_midi as f32).to_hz(reference_hz).0;
        let cents = 1200.0 * (reading.frequency_hz.0 / detected_frequency).log2();
        let mut matches = Vec::new();

        for string in tuning.iter_strings() {
            let open = tuning.note(string);
            for fret in (0..=18).map(Fret) {
                let note = open.add(fret.semitones());
                if note.as_u8() != detected_midi {
                    continue;
                }

                matches.push(TunerTarget {
                    string,
                    fret,
                    note_name: note.to_anote(),
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
        // Lower bound of the resonator's note buckets, as a raw index for the
        // `checked_sub` below (the strengths Vec is indexed midi - min_midi).
        let min_midi = self.audio.analysis_settings().resonator.min_midi.as_u8() as usize;
        let mut matches = Vec::new();

        for string in tuning.iter_strings() {
            let open = tuning.note(string);
            for fret in (0..=18).map(Fret) {
                let note = open.add(fret.semitones());
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
            let center = fretboard.note_center(target.string, target.fret);
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
            let center = fretboard.note_center(target.string, target.fret);
            painter.circle_stroke(center, 18.0, Stroke::new(2.0_f32, color::MARKER));
            painter.circle_stroke(
                center,
                24.0,
                Stroke::new(1.0_f32, color::MARKER.gamma_multiply(96.0 / 255.0)),
            );
        }
    }
}

/// Frets 1–18, as the card's counter says.
const VISIBLE_FRETS: std::ops::Range<Fret> = Fret(1)..Fret(19);

/// Below this card width the neck turns vertical. At 600 dp the horizontal neck's
/// 18 log-spaced frets already sit closer than a note pill is wide from about the
/// 7th fret on; on a phone in portrait (a ~314 dp card) the top frets are 7 dp
/// apart, four of them to one 30 dp pill. Landscape phones and desktop panes
/// stay horizontal.
const VERTICAL_BELOW_W: f32 = 600.0;
/// Length of the vertical neck, nut → bridge. The log spacing shrinks towards the
/// bridge, and the closest pair (frets 17–18) is 0.032 of this length apart, so
/// 660 dp keeps them ~21 dp apart: more than a note pill is tall (16).
const VERTICAL_NECK_LEN: f32 = 660.0;
/// Vertical neck margins: two label rows above the nut (string name, open note),
/// fret numbers on the left, the position brackets on the right, the footer below.
const VERTICAL_MARGIN_TOP: f32 = 44.0;
/// Room for a two-digit fret number ("18", ~15 dp in 12 px monospace) and its gap.
const VERTICAL_MARGIN_LEFT: f32 = 24.0;
const VERTICAL_MARGIN_BOTTOM: f32 = 40.0;

/// How the fretboard card lays its neck out in a given width: the axis, the size
/// of the whole drawing, and the neck's insets inside it.
struct NeckLayout {
    axis:   NeckAxis,
    size:   Vec2,
    /// Insets of the neck rectangle: left, right, top, bottom.
    margin: [f32; 4],
}

impl NeckLayout {
    fn for_width(width: f32) -> Self {
        if width < VERTICAL_BELOW_W {
            Self {
                axis:   NeckAxis::Vertical,
                size:   vec2(
                    width,
                    VERTICAL_MARGIN_TOP + VERTICAL_NECK_LEN + VERTICAL_MARGIN_BOTTOM,
                ),
                margin: [
                    VERTICAL_MARGIN_LEFT,
                    VERTICAL_REACH,
                    VERTICAL_MARGIN_TOP,
                    VERTICAL_MARGIN_BOTTOM,
                ],
            }
        } else {
            Self {
                axis:   NeckAxis::Horizontal,
                size:   vec2(width, FRETBOARD_HEIGHT),
                margin: [
                    FRETBOARD_MARGIN_LEFT,
                    FRETBOARD_MARGIN_RIGHT,
                    FRETBOARD_MARGIN_TOP,
                    FRETBOARD_MARGIN_BOTTOM,
                ],
            }
        }
    }

    /// The neck rectangle inside the card's drawing `component`.
    fn neck_rect(&self, component: Rect) -> Rect {
        let [left, right, top, bottom] = self.margin;
        Rect::from_min_max(
            pos2(component.left() + left, component.top() + top),
            pos2(component.right() - right, component.bottom() - bottom),
        )
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
            let start = (center - bins_per_note * 0.5).floor().max(0.0) as usize;
            let end = (center + bins_per_note * 0.5)
                .ceil()
                .min(spectrum.len().saturating_sub(1) as f32) as usize;

            spectrum[start..=end].iter().copied().fold(0.0_f32, f32::max)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        Rect,
        pos2,
    };

    use super::{
        NeckLayout,
        VISIBLE_FRETS,
    };
    use crate::app::ALL_TUNINGS;
    use crate::ui::fretboard::draw::NOTE_PILL;
    use crate::ui::fretboard::geometry::{
        FretConfig,
        Fretboard,
        NeckAxis,
    };

    /// Every pair of note pills on the neck laid out for a card `width` wide that
    /// overlap, for every tuning the card offers.
    fn overlapping_pills(width: f32) -> Vec<String> {
        let layout = NeckLayout::for_width(width);
        let component = Rect::from_min_size(pos2(0.0, 0.0), layout.size);
        let mut overlaps = Vec::new();
        for kind in ALL_TUNINGS {
            let neck = Fretboard::new(
                layout.neck_rect(component),
                layout.axis,
                FretConfig::Log,
                kind.to_tuning(),
                VISIBLE_FRETS,
            );
            let pills: Vec<_> = neck
                .iter_strings()
                .flat_map(|s| neck.iter_frets().map(move |f| (s, f)))
                .map(|(s, f)| (s, f, Rect::from_center_size(neck.note_center(s, f), NOTE_PILL)))
                .collect();
            for (i, (s, f, a)) in pills.iter().enumerate() {
                for (t, g, b) in &pills[i + 1..] {
                    if a.intersect(*b).area() > 0.0 {
                        overlaps.push(format!("{}: {s:?}/{f:?} with {t:?}/{g:?}", kind.label()));
                    }
                }
            }
        }
        overlaps
    }

    /// REGRESSION: on a phone in portrait the neck is vertical and no two notes are
    /// drawn on top of each other — 18 log-spaced frets in a ~314 dp wide horizontal
    /// neck put the top frets 7 dp apart under 30 dp pills, four notes to one pill.
    /// 314/366 = the card on a 360/412 dp phone, six strings included (guitar).
    #[test]
    fn phone_neck_is_vertical_and_its_notes_do_not_overlap() {
        for width in [314.0, 366.0] {
            let overlaps = overlapping_pills(width);
            assert!(overlaps.is_empty(), "card {width} dp: {overlaps:?}");
            assert_eq!(NeckLayout::for_width(width).axis, NeckAxis::Vertical);
        }
    }

    /// The desktop card and a landscape phone (a ~694 dp card) keep the horizontal
    /// neck the panel always had.
    #[test]
    fn wide_card_keeps_the_horizontal_neck() {
        for width in [694.0, 900.0] {
            assert_eq!(NeckLayout::for_width(width).axis, NeckAxis::Horizontal);
        }
    }
}
