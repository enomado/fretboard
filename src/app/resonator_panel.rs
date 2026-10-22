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

#[cfg(any(target_os = "android", test))]
use super::mobile_split::Part;
use super::{
    App,
    card_header,
    pill,
    pill_muted,
    spectrum_color,
};
use crate::audio::types::{
    AnalysisSettings,
    ResonatorReading,
};
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
#[cfg(any(target_os = "android", test))]
use crate::ui::slider_row::SliderRow;
use crate::ui::snail::{
    self,
    SpiralChart,
};
use crate::ui::tokens::color;
#[cfg(any(target_os = "android", test))]
use crate::ui::tokens::space;
use crate::ui::{
    plot_text,
    theme,
    waterfall,
};

impl App {
    pub(super) fn draw_resonator_snail_card(&mut self, ui: &mut Ui) {
        self.draw_resonator_snail_card_inner(ui, false);
    }

    /// The phone's snail, a split panel (`mobile_split`): the screen is this card with
    /// the spiral sized to it, the config the knob strip — the phone has no other place
    /// for these knobs (no Controls/Config tabs beside it).
    #[cfg(any(target_os = "android", test))]
    pub(super) fn draw_mobile_snail_part(&mut self, ui: &mut Ui, part: Part) {
        match part {
            Part::Screen => self.draw_resonator_snail_card_inner(ui, true),
            Part::Config => self.draw_mobile_snail_settings(ui),
        }
    }

    /// The snail card. `mobile` = the phone's screen part: compact header, card margins
    /// for a finger's screen, the spiral sized to the screen (`draw_mobile_spiral`).
    fn draw_resonator_snail_card_inner(&mut self, ui: &mut Ui, mobile: bool) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
        let reading_ref = reading.as_ref();

        let (pad, radius) = theme::card_pad(ui.ctx(), 14, 22);
        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                if mobile {
                    // The screen of a split panel: the card is the whole screen, down to
                    // the config, not a box floating over a gap (`mobile_split`).
                    ui.set_min_height(ui.available_height());
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Resonators")
                                .size(if mobile { 19.0 } else { 20.0 })
                                .color(color::TEXT_HEADING),
                        );
                        // The long subtitle is a single non-wrapping line — on the
                        // phone it would distend the frame past the screen's right
                        // edge, so the compact (mobile) header drops it.
                        if !mobile {
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

                ui.add_space(if mobile { 6.0 } else { 12.0 });
                let chart = SpiralChart {
                    title:             "Resonator spiral",
                    // Dropped where it does not fit beside the title (`plot_text::draw_title`).
                    subtitle:          "same snail, but driven by the resonator bank instead of FFT bins",
                    spectrum:          reading_ref.map(|value| value.spectrum.as_slice()),
                    waterfall:         reading_ref.map_or(&[][..], |value| value.waterfall.as_slice()),
                    note_labels:       reading_ref.map_or(&[][..], |value| value.note_labels.as_slice()),
                    active_note:       None,
                    waiting_message:   "Play a sustained note to charge the resonator bank",
                    empty_message:     "The resonator bank is empty",
                    active_note_label: "bank focus",
                };
                #[cfg(any(target_os = "android", test))]
                if mobile {
                    draw_mobile_spiral(ui, chart, &self.audio.analysis_settings());
                    return;
                }
                snail::draw_spiral_chart_sized(
                    ui,
                    chart,
                    &self.audio.analysis_settings(),
                    snail::SPIRAL_MIN_HEIGHT,
                    f32::INFINITY,
                );
            });
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
        // Every knob is a finger target, TOUCH_H tall (sliders: `ui::slider_row`).
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
            // egui sizes a slider by `interact_size.y`, not by `add_sized` (it wraps
            // itself in a `horizontal`): see `ui::slider_row`.
            changed |= theme::finger_sized(ui, |ui| {
                ui.spacing_mut().slider_width = 104.0;
                ui.add(egui::Slider::new(a4, 400.0..=466.0).show_value(false))
            })
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
        if theme::finger_sized(ui, |ui| {
            ui.checkbox(&mut settings.resonator.reassign, "Δφ reassign")
        })
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
                card_header(
                    ui,
                    "Resonator Bank",
                    egui::RichText::new("Continuous resonator state and current magnitudes")
                        .color(color::TEXT_HINT),
                    |ui| {
                        match reading_ref {
                            Some(reading) => pill(ui, &format!("{} bins", reading.spectrum.len())),
                            None => pill_muted(ui, "waiting for input"),
                        }
                    },
                );

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

        plot_text::draw_title(
            &painter,
            rect,
            "Bank waterfall + current frame",
            "continuous resonator state, no FFT window grid",
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

        let hint_rect = rect.shrink(14.0);
        let Some(reading) = reading else {
            plot_text::draw_hint(
                &painter,
                hint_rect,
                "The resonator bank starts filling as soon as audio reaches the input",
                13.0,
                color::TEXT_HINT,
            );
            return;
        };

        if reading.spectrum.is_empty() || reading.note_labels.is_empty() {
            plot_text::draw_hint(
                &painter,
                hint_rect,
                "No resonator bins available for the current frame",
                13.0,
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
                card_header(
                    ui,
                    "Resonator Waterfall",
                    egui::RichText::new("A full-screen spectrogram from the new resonator bank")
                        .color(color::TEXT_HINT),
                    |ui| {
                        match reading_ref {
                            Some(reading) => pill(ui, &format!("{} frames", reading.waterfall.len())),
                            None => pill_muted(ui, "waiting for input"),
                        }
                    },
                );

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

        let hint_rect = rect.shrink(14.0);
        let Some(reading) = reading else {
            plot_text::draw_hint(
                &painter,
                hint_rect,
                "Play audio to fill the resonator waterfall",
                15.0,
                color::TEXT_HINT,
            );
            return;
        };

        if reading.waterfall.is_empty() || reading.note_labels.is_empty() {
            plot_text::draw_hint(
                &painter,
                hint_rect,
                "Waiting for the resonator bank to accumulate frames",
                15.0,
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

/// The phone's spiral: round and as big as the screen part allows — a square of the
/// screen's width or of what is left of its height, whichever is less, centred. Under
/// `SPIRAL_FLOOR` (the picker unfolded over a short screen) the chart keeps the floor
/// and the screen scrolls, rather than shrinking into an unreadable dot.
///
/// The knobs are not in this card any more: they are the split's config
/// (`mobile_split`). Stacked above the spiral in one card they used to push it off a
/// 360×740 screen entirely. Live analysis draws its spiral on the phone this way too.
pub(super) fn draw_mobile_spiral(ui: &mut Ui, chart: SpiralChart<'_>, settings: &AnalysisSettings) {
    // A 360 dp-tall landscape screen leaves the snail's spiral ≈232 dp under the card's
    // header and Live analysis's ≈167 under its header and level meter; the old 240
    // pushed both off the bottom there.
    const SPIRAL_FLOOR: f32 = 150.0;
    let available = ui.available_size_before_wrap();
    let side = available.y.max(SPIRAL_FLOOR).min(available.x);
    ui.vertical_centered(|ui| {
        ui.allocate_ui(vec2(side, side), |ui| {
            snail::draw_spiral_chart_sized(ui, chart, settings, side, side);
        });
    });
}

/// One labelled knob row for the mobile settings strip — the canonical [`SliderRow`]
/// (caption, slider over `range` named after it, `readout`). True when the user moved
/// it. Rows are stacked by the caller's vertical layout (see
/// `draw_mobile_snail_settings` for why we don't wrap them).
#[cfg(any(target_os = "android", test))]
fn mobile_slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    readout: impl Fn(N) -> String,
) -> bool {
    ui.add(SliderRow::new(RowCaption::new(label), value, range, readout).width(140.0))
        .changed()
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
