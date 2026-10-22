use eframe::egui::{
    self,
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
    pill_muted,
    spectrum_color,
};
use crate::audio::types::ResonatorReading;
#[cfg(any(target_os = "android", test))]
use crate::core_types::note::AccidentalStyle;
#[cfg(any(target_os = "android", test))]
use crate::core_types::pitch::PNote;
// Only the Android strip (`draw_mobile_snail_settings`) has pills; gate the import
// the same way its neighbours above are gated.
#[cfg(any(target_os = "android", test))]
use crate::ui::segmented::{
    RowCaption,
    SegmentedButton,
};
use crate::ui::snail::{
    self,
    SpiralChart,
};
use crate::ui::tokens::color;
#[cfg(any(target_os = "android", test))]
use crate::ui::tokens::space;
use crate::ui::waterfall;

impl App {
    pub(super) fn draw_resonator_snail_card(&mut self, ui: &mut Ui) {
        self.draw_resonator_snail_card_inner(ui, snail::SPIRAL_MIN_HEIGHT, false);
    }

    /// Mobile (Android) entry: the same snail card, but the spiral is sized to the
    /// phone's screen and the card grows a compact settings strip. The phone has
    /// no other place for these knobs, so the card doubles as the settings surface.
    #[cfg(any(target_os = "android", test))]
    pub(super) fn draw_mobile_snail_card(&mut self, ui: &mut Ui) {
        self.draw_resonator_snail_card_inner(ui, snail::SPIRAL_MIN_HEIGHT, true);
    }

    /// Shared body for both the desktop tab and the mobile card. `min_height` is
    /// the desktop spiral's height floor; `with_settings` switches to the mobile
    /// body — spiral sized to the screen plus the knob strip (Android only).
    fn draw_resonator_snail_card_inner(&mut self, ui: &mut Ui, min_height: f32, with_settings: bool) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonators")
                                .size(if with_settings { 19.0 } else { 20.0 })
                                .color(color::TEXT_HEADING),
                        );
                        // The long subtitle is a single non-wrapping line — on the
                        // phone it would distend the frame past the screen's right
                        // edge, so the compact (mobile) header drops it.
                        if !with_settings {
                            ui.label(
                                egui::RichText::new(
                                    "Alexandre Francois's Resonate bank, streamed into our pitch spiral",
                                )
                                .color(color::TEXT_HINT),
                            );
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(ui, &format!("{} bins", reading.spectrum.len()));
                        } else {
                            pill_muted(ui, "waiting for input");
                        }
                    });
                });

                ui.add_space(if with_settings { 6.0 } else { 12.0 });
                let chart = SpiralChart {
                    title:             "Resonator spiral",
                    // The chart paints title (left) and subtitle (right) on one
                    // line; on the narrow phone they collide, so mobile drops
                    // the subtitle just like the card header above.
                    subtitle:          if with_settings {
                        ""
                    } else {
                        "same snail, but driven by the resonator bank instead of FFT bins"
                    },
                    spectrum:          reading_ref.map(|value| value.spectrum.as_slice()),
                    waterfall:         reading_ref.map_or(&[][..], |value| value.waterfall.as_slice()),
                    note_labels:       reading_ref.map_or(&[][..], |value| value.note_labels.as_slice()),
                    active_note:       None,
                    waiting_message:   "Play a sustained note to charge the resonator bank",
                    empty_message:     "The resonator bank is empty",
                    active_note_label: "bank focus",
                };
                #[cfg(any(target_os = "android", test))]
                if with_settings {
                    self.draw_mobile_snail_body(ui, chart);
                    return;
                }
                snail::draw_spiral_chart_sized(
                    ui,
                    chart,
                    &self.audio.analysis_settings(),
                    min_height,
                    f32::INFINITY,
                );
            });
    }

    /// The phone's snail: the spiral first, the knobs after it. The knobs used to
    /// sit *above* the spiral, and nine rows of them (~400 dp) left a 360×740
    /// portrait screen — and every landscape one — with the spiral, the thing the
    /// card is for, entirely below the bottom edge.
    ///
    /// - Portrait: the spiral fills the leftover height but never exceeds the
    ///   width (the chart is round; taller is dead space), with a floor so a short
    ///   screen still gets a usable chart; the knobs follow below and the outer
    ///   scroll area reaches them.
    /// - Landscape (the screen is wider than tall): spiral as a square on the
    ///   left, at most 60% of the width, knobs in a column beside it — stacking
    ///   them would again push one of the two off a 360 dp-tall screen.
    #[cfg(any(target_os = "android", test))]
    fn draw_mobile_snail_body(&mut self, ui: &mut Ui, chart: SpiralChart<'_>) {
        // The floor the phone card always had: keeps the spiral usable on a short
        // screen; anything taller just fills the leftover height.
        const SPIRAL_FLOOR: f32 = 240.0;
        let settings = self.audio.analysis_settings();
        let available = ui.available_size_before_wrap();
        // Orientation of the *screen*, not of the leftover space: with the panel
        // picker unfolded a portrait phone has little height left, and the knob
        // column beside the spiral would be ~150 dp wide — too narrow for its rows.
        let screen = ui.ctx().content_rect();
        if screen.width() > screen.height() {
            let side = available.y.max(SPIRAL_FLOOR).min(available.x * 0.6);
            ui.horizontal_top(|ui| {
                ui.allocate_ui(vec2(side, side), |ui| {
                    snail::draw_spiral_chart_sized(ui, chart, &settings, side, side);
                });
                ui.vertical(|ui| self.draw_mobile_snail_settings(ui));
            });
        } else {
            snail::draw_spiral_chart_sized(ui, chart, &settings, SPIRAL_FLOOR.min(available.x), available.x);
            ui.add_space(8.0);
            self.draw_mobile_snail_settings(ui);
        }
    }

    /// Compact, touch-friendly knobs that shape the snail. One knob per row,
    /// stacked by the ambient vertical layout. Sliders mutate a cloned
    /// `AnalysisSettings` and only push it back when something changed.
    ///
    /// NB: do NOT wrap these in `horizontal_wrapped`. Each knob is itself a fixed
    /// `horizontal` (label+slider+readout); nesting a non-wrapping `horizontal`
    /// inside a wrapped row makes egui reserve a full row's width per knob and
    /// keep measuring additively, which **doubled the card's `max_rect`** (376→813
    /// dp here) and floated the round snail clean off the screen's right edge.
    /// A plain vertical stack sizes to the real content width.
    #[cfg(any(target_os = "android", test))]
    fn draw_mobile_snail_settings(&mut self, ui: &mut Ui) {
        // Every knob is a finger target: the slider's hit area is its whole
        // allocated rect, so the row is TOUCH_H tall even though the rail is thin.
        // At the old 20 dp the −/+ nudges were ~18×26 — a miss for a fingertip.
        const KNOB_H: f32 = space::TOUCH_H;
        let mut settings = self.audio.analysis_settings();
        let mut changed = false;

        // Эталон A4 (камертон): тот же стандарт строя, что и на десктопе, но в
        // мобильной полосе — иначе на Android его негде задать (вкладочной
        // раскладки с `draw_fretboard_controls` тут нет). Плавно, 400..466 Гц.
        //
        // Подпись — своей строкой: «подпись − слайдер + значение» одним рядом
        // шире 360-dp экрана (≈380 dp), и значение уезжало за правый край.
        let a4_caption = ui.label(
            egui::RichText::new("Pitch A4")
                .color(color::TEXT_CAPTION)
                .strong(),
        );
        knob_row(ui, |ui| {
            // Слайдер ведёт грубо по всему 66-герцовому диапазону; пальцем в
            // точный академ. строй (440/442/443) по нему не попасть, поэтому
            // рядом — кнопочки −/+ для тонкой настройки. Они нуджат камертон со
            // снапом на сетку STEP, так что повторные тапы шагают по круглым
            // значениям. Клампим тем же [400, 466], что и `sanitize()`, иначе
            // выход за границы тихо срежется при `set_analysis_settings`.
            const STEP: f32 = 0.5;
            let snap = |hz: f32| (hz / STEP).round() * STEP;
            let a4 = &mut settings.concert_pitch_hz.0;
            let nudge = |label| SegmentedButton::action(label).min_width(KNOB_H).height(KNOB_H);
            if ui.add(nudge("−")).clicked() {
                *a4 = snap(*a4 - STEP).clamp(400.0, 466.0);
                changed = true;
            }
            changed |= ui
                .add_sized(
                    [104.0, KNOB_H],
                    egui::Slider::new(a4, 400.0..=466.0).show_value(false),
                )
                .labelled_by(a4_caption.id)
                .changed();
            if ui.add(nudge("+")).clicked() {
                *a4 = snap(*a4 + STEP).clamp(400.0, 466.0);
                changed = true;
            }
            knob_value(ui, format!("{:.1} Hz", *a4));
        });
        changed |= mobile_slider(ui, "Spread", &mut settings.note_spread, 0.15..=0.8, |v| {
            format!("{v:.2}")
        });
        changed |= mobile_slider(ui, "Glow", &mut settings.note_gamma, 0.35..=1.2, |v| {
            format!("{v:.2}")
        });
        changed |= mobile_slider(ui, "Trail", &mut settings.resonator.history, 8..=240, |v| {
            v.to_string()
        });
        changed |= mobile_slider(ui, "Bins", &mut settings.resonator.bins, 1..=12, |v| {
            v.to_string()
        });

        // Octave window the resonator listens on — these bounds also decide which
        // octaves are visible in the snail. Sliders work in octave space (one
        // octave = 12 semitones); the validated `PNote` is rebuilt as
        // `(oct + 1) * 12` MIDI. Octave ranges mirror the desktop resonator
        // clamps (min 12..=84 → C0..C6, max 24..=108 → C1..C8) expressed in
        // octaves; `sanitized()` keeps the `max ≥ min + 6` invariant afterwards.
        // `min_midi` is clamped ≥ 12 and `max_midi` ≥ 24, so the `u8` subtraction
        // can't underflow and the rebuilt MIDI (≤ 108) can't overflow.
        let mut low_oct = settings.resonator.min_midi.as_u8() / 12 - 1;
        if mobile_slider(ui, "Low oct", &mut low_oct, 0..=6, |o| format!("C{o}")) {
            settings.resonator.min_midi = PNote::new((low_oct + 1) * 12).unwrap();
            changed = true;
        }
        let mut high_oct = settings.resonator.max_midi.as_u8() / 12 - 1;
        if mobile_slider(ui, "High oct", &mut high_oct, 1..=8, |o| format!("C{o}")) {
            settings.resonator.max_midi = PNote::new((high_oct + 1) * 12).unwrap();
            changed = true;
        }

        // Δφ reassignment toggle (super-resolution + image/noise gate). Off falls
        // back to plain per-bin magnitude — the safety net if reassignment misbehaves.
        if ui
            .checkbox(&mut settings.resonator.reassign, "Δφ reassign")
            .changed()
        {
            changed = true;
        }

        // Знаки альтерации: диезы/бемоли во всех подписях нот (улитка, резонаторы).
        knob_row(ui, |ui| {
            ui.add(RowCaption::new("Notes"));
            for (label, style) in [("C#", AccidentalStyle::Sharps), ("Db", AccidentalStyle::Flats)] {
                let selected = settings.accidental == style;
                if ui
                    .add(SegmentedButton::new(label, selected).height(KNOB_H))
                    .clicked()
                    && settings.accidental != style
                {
                    settings.accidental = style;
                    changed = true;
                }
            }
        });

        if changed {
            self.audio.set_analysis_settings(settings);
        }
    }

    pub(super) fn draw_resonator_bank_card(&mut self, ui: &mut Ui) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonator Bank")
                                .size(20.0)
                                .color(color::TEXT_HEADING),
                        );
                        ui.label(
                            egui::RichText::new("Continuous resonator state and current magnitudes")
                                .color(color::TEXT_HINT),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(ui, &format!("{} bins", reading.spectrum.len()));
                        } else {
                            pill_muted(ui, "waiting for input");
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_resonator_bank_panel(ui, reading_ref);
            });
    }

    fn draw_resonator_bank_panel(&self, ui: &mut Ui, reading: Option<&ResonatorReading>) {
        let available_size = ui.available_size_before_wrap();
        let desired_size = vec2(available_size.x, available_size.y.max(244.0));
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, color::PLOT_BG);
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, color::PLOT_STROKE),
            egui::StrokeKind::Inside,
        );

        painter.text(
            pos2(rect.left() + 14.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "Bank waterfall + current frame",
            FontId::proportional(15.0),
            color::TEXT_BADGE,
        );
        painter.text(
            pos2(rect.right() - 14.0, rect.top() + 12.0),
            egui::Align2::RIGHT_TOP,
            "continuous resonator state, no FFT window grid",
            FontId::proportional(12.0),
            color::TEXT_HINT,
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
                "The resonator bank starts filling as soon as audio reaches the input",
                FontId::proportional(13.0),
                color::TEXT_HINT,
            );
            return;
        };

        if reading.spectrum.is_empty() || reading.note_labels.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No resonator bins available for the current frame",
                FontId::proportional(13.0),
                color::TEXT_HINT,
            );
            return;
        }

        let bins_per_label = if reading.note_labels.len() > 1 {
            (reading.spectrum.len().saturating_sub(1) as f32 / (reading.note_labels.len() - 1) as f32)
                .max(1.0)
        } else {
            1.0
        };
        waterfall::draw_pitch_labeled_waterfall(
            &painter,
            waterfall_rect,
            &reading.waterfall,
            &reading.note_labels,
            bins_per_label,
            None,
        );

        let bar_width = bars_rect.width() / reading.spectrum.len().max(1) as f32;
        for (index, value) in reading.spectrum.iter().enumerate() {
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
            color::TEXT_HINT,
        );
    }

    pub(super) fn draw_resonator_waterfall_card(&mut self, ui: &mut Ui) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
        let reading_ref = reading.as_ref();

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonator Waterfall")
                                .size(20.0)
                                .color(color::TEXT_HEADING),
                        );
                        ui.label(
                            egui::RichText::new("A full-screen spectrogram from the new resonator bank")
                                .color(color::TEXT_HINT),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(ui, &format!("{} frames", reading.waterfall.len()));
                        } else {
                            pill_muted(ui, "waiting for input");
                        }
                    });
                });

                ui.add_space(12.0);
                self.draw_resonator_waterfall_panel(ui, reading_ref);
            });
    }

    fn draw_resonator_waterfall_panel(&self, ui: &mut Ui, reading: Option<&ResonatorReading>) {
        let available_size = ui.available_size_before_wrap();
        let desired_size = vec2(available_size.x, available_size.y.max(260.0));
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 18.0, color::HEAT_BG);
        painter.rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, color::PLOT_STROKE),
            egui::StrokeKind::Inside,
        );

        let Some(reading) = reading else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Play audio to fill the resonator waterfall",
                FontId::proportional(15.0),
                color::TEXT_HINT,
            );
            return;
        };

        if reading.waterfall.is_empty() || reading.note_labels.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for the resonator bank to accumulate frames",
                FontId::proportional(15.0),
                color::TEXT_HINT,
            );
            return;
        }

        let chart_rect = Rect::from_min_max(
            pos2(rect.left() + 16.0, rect.top() + 20.0),
            pos2(rect.right() - 16.0, rect.bottom() - 44.0),
        );

        let bins_per_label = if reading.note_labels.len() > 1 {
            (reading.spectrum.len().saturating_sub(1) as f32 / (reading.note_labels.len() - 1) as f32)
                .max(1.0)
        } else {
            1.0
        };
        waterfall::draw_pitch_labeled_waterfall(
            &painter,
            chart_rect,
            &reading.waterfall,
            &reading.note_labels,
            bins_per_label,
            None,
        );

        painter.text(
            pos2(rect.left() + 18.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            "newest frame at the bottom, resonator bins across the pitch axis",
            FontId::proportional(12.0),
            color::TEXT_HINT,
        );
        painter.text(
            pos2(rect.right() - 18.0, rect.bottom() - 14.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{} active bins", reading.spectrum.len()),
            FontId::proportional(12.0),
            color::TEXT_HINT,
        );
    }
}

/// One labelled knob row for the mobile settings strip: a caption, a slider over
/// `range` (named after the caption, so TalkBack reads "Spread", not an anonymous
/// slider) and the `readout` of the value it leaves. True when the user moved it.
/// The trio sits on one fixed [`knob_row`]; rows are stacked by the caller's
/// vertical layout (see `draw_mobile_snail_settings` for why we don't wrap them).
#[cfg(any(target_os = "android", test))]
fn mobile_slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    readout: impl Fn(N) -> String,
) -> bool {
    let mut changed = false;
    knob_row(ui, |ui| {
        let caption = ui.label(egui::RichText::new(label).color(color::TEXT_CAPTION).strong());
        changed = ui
            .add_sized(
                [140.0, space::TOUCH_H],
                egui::Slider::new(value, range).show_value(false),
            )
            .labelled_by(caption.id)
            .changed();
        knob_value(ui, readout(*value));
    });
    changed
}

/// A touch-height row of the mobile strip with its items centred on it. A plain
/// `ui.horizontal` centres each item on the row height known *when that item is
/// placed*, so a caption placed before a 44 dp slider sat on the old 20 dp band —
/// visibly above the knob. Claiming the full band up front fixes it by construction.
#[cfg(any(target_os = "android", test))]
fn knob_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), space::TOUCH_H),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    );
}

/// The numeric readout at the end of a knob row.
#[cfg(any(target_os = "android", test))]
fn knob_value(ui: &mut Ui, value: String) {
    ui.label(egui::RichText::new(value).color(color::TEXT_VALUE).monospace());
}
