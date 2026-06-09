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
#[cfg(target_os = "android")]
use crate::core_types::pitch::PNote;
use crate::audio::ResonatorReading;
use crate::ui::snail::{
    self,
    SpiralChart,
};
use crate::ui::theme::PANEL_FILL;
use crate::ui::waterfall;

impl App {
    pub(super) fn draw_resonator_snail_card(&mut self, ui: &mut Ui) {
        self.draw_resonator_snail_card_inner(ui, snail::SPIRAL_MIN_HEIGHT, false);
    }

    /// Mobile (Android) entry: the same snail card, but it sizes the spiral to
    /// the leftover screen height (small floor) and grows a compact settings
    /// strip in the dead space the round snail leaves. The phone shows only this
    /// one card, so it doubles as the settings surface.
    #[cfg(target_os = "android")]
    pub(super) fn draw_mobile_snail_card(&mut self, ui: &mut Ui) {
        // Floor of 240 keeps the spiral usable on a short (landscape) phone while
        // a tall portrait screen just fills the remaining height; the outer
        // scroll area (see `render`) is the safety net if even 240 doesn't fit.
        self.draw_resonator_snail_card_inner(ui, 240.0_f32.min(snail::SPIRAL_MIN_HEIGHT), true);
    }

    /// Shared body for both the desktop tab and the mobile card. `min_height` is
    /// the spiral's height floor; `with_settings` injects the mobile knob strip
    /// (only ever set on Android).
    fn draw_resonator_snail_card_inner(&mut self, ui: &mut Ui, min_height: f32, with_settings: bool) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
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
                                .size(if with_settings { 19.0 } else { 20.0 })
                                .color(Color32::from_rgb(228, 220, 208)),
                        );
                        // The long subtitle is a single non-wrapping line — on the
                        // phone it would distend the frame past the screen's right
                        // edge, so the compact (mobile) header drops it.
                        if !with_settings {
                            ui.label(
                                egui::RichText::new(
                                    "Alexandre Francois's Resonate bank, streamed into our pitch spiral",
                                )
                                .color(Color32::from_rgb(152, 158, 165)),
                            );
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(reading) = reading_ref {
                            pill(
                                ui,
                                &format!("{} bins", reading.spectrum.len()),
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

                ui.add_space(if with_settings { 6.0 } else { 12.0 });
                #[cfg(target_os = "android")]
                if with_settings {
                    self.draw_mobile_snail_settings(ui);
                    ui.add_space(8.0);
                }

                // Mobile caps the snail's height to the available width so the
                // round chart stays inside the narrow screen instead of filling
                // (possibly unbounded) leftover height. Desktop stays rubbery.
                let max_height = if with_settings { ui.available_width() } else { f32::INFINITY };
                snail::draw_spiral_chart_sized(
                    ui,
                    SpiralChart {
                        title:             "Resonator spiral",
                        // The chart paints title (left) and subtitle (right) on one
                        // line; on the narrow phone they collide, so mobile drops
                        // the subtitle just like the card header above.
                        subtitle:          if with_settings { "" } else { "same snail, but driven by the resonator bank instead of FFT bins" },
                        spectrum:          reading_ref.map(|value| value.spectrum.as_slice()),
                        waterfall:         reading_ref.map_or(&[][..], |value| value.waterfall.as_slice()),
                        note_labels:       reading_ref.map_or(&[][..], |value| value.note_labels.as_slice()),
                        active_note:       None,
                        waiting_message:   "Play a sustained note to charge the resonator bank",
                        empty_message:     "The resonator bank is empty",
                        active_note_label: "bank focus",
                    },
                    &self.audio.analysis_settings(),
                    min_height,
                    max_height,
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
    #[cfg(target_os = "android")]
    fn draw_mobile_snail_settings(&mut self, ui: &mut Ui) {
        let mut settings = self.audio.analysis_settings();
        let mut changed = false;

        // Эталон A4 (камертон): тот же стандарт строя, что и на десктопе, но в
        // мобильной полосе — иначе на Android его негде задать (вкладочной
        // раскладки с `draw_fretboard_controls` тут нет). Плавно, 400..466 Гц.
        mobile_slider(ui, "Pitch A4", &mut changed, |ui, c| {
            // Слайдер ведёт грубо по всему 66-герцовому диапазону; пальцем в
            // точный академ. строй (440/442/443) по нему не попасть, поэтому
            // рядом — кнопочки −/+ для тонкой настройки. Они нуджат камертон со
            // снапом на сетку STEP, так что повторные тапы шагают по круглым
            // значениям. Клампим тем же [400, 466], что и `sanitize()`, иначе
            // выход за границы тихо срежется при `set_analysis_settings`.
            const STEP: f32 = 0.5;
            let snap = |hz: f32| (hz / STEP).round() * STEP;
            if ui.small_button("−").clicked() {
                settings.concert_pitch_hz = snap(settings.concert_pitch_hz - STEP).clamp(400.0, 466.0);
                *c = true;
            }
            *c |= ui
                .add_sized(
                    [104.0, 20.0],
                    egui::Slider::new(&mut settings.concert_pitch_hz, 400.0..=466.0).show_value(false),
                )
                .changed();
            if ui.small_button("+").clicked() {
                settings.concert_pitch_hz = snap(settings.concert_pitch_hz + STEP).clamp(400.0, 466.0);
                *c = true;
            }
            format!("{:.1} Hz", settings.concert_pitch_hz)
        });
        mobile_slider(ui, "Spread", &mut changed, |ui, c| {
            *c |= ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut settings.note_spread, 0.15..=0.8).show_value(false))
                .changed();
            format!("{:.2}", settings.note_spread)
        });
        mobile_slider(ui, "Glow", &mut changed, |ui, c| {
            *c |= ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut settings.note_gamma, 0.35..=1.2).show_value(false))
                .changed();
            format!("{:.2}", settings.note_gamma)
        });
        mobile_slider(ui, "Trail", &mut changed, |ui, c| {
            *c |= ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut settings.resonator.history, 8..=240).show_value(false))
                .changed();
            settings.resonator.history.to_string()
        });
        mobile_slider(ui, "Bins", &mut changed, |ui, c| {
            *c |= ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut settings.resonator.bins, 1..=12).show_value(false))
                .changed();
            settings.resonator.bins.to_string()
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
        mobile_slider(ui, "Low oct", &mut changed, |ui, c| {
            if ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut low_oct, 0..=6).show_value(false))
                .changed()
            {
                settings.resonator.min_midi = PNote::new((low_oct + 1) * 12).unwrap();
                *c = true;
            }
            format!("C{low_oct}")
        });
        let mut high_oct = settings.resonator.max_midi.as_u8() / 12 - 1;
        mobile_slider(ui, "High oct", &mut changed, |ui, c| {
            if ui
                .add_sized([140.0, 20.0], egui::Slider::new(&mut high_oct, 1..=8).show_value(false))
                .changed()
            {
                settings.resonator.max_midi = PNote::new((high_oct + 1) * 12).unwrap();
                *c = true;
            }
            format!("C{high_oct}")
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
                                &format!("{} bins", reading.spectrum.len()),
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

    fn draw_resonator_bank_panel(&self, ui: &mut Ui, reading: Option<&ResonatorReading>) {
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
                "The resonator bank starts filling as soon as audio reaches the input",
                FontId::proportional(13.0),
                Color32::from_rgb(139, 143, 149),
            );
            return;
        };

        if reading.spectrum.is_empty() || reading.note_labels.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No resonator bins available for the current frame",
                FontId::proportional(13.0),
                Color32::from_rgb(139, 143, 149),
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
            Color32::from_rgb(152, 158, 165),
        );
    }

    pub(super) fn draw_resonator_waterfall_card(&mut self, ui: &mut Ui) {
        self.audio.request_resonator(); // потребитель банка → держим его «нужным»
        let reading = self.audio.resonator_reading();
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
                                &format!("{} frames", reading.waterfall.len()),
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

    fn draw_resonator_waterfall_panel(&self, ui: &mut Ui, reading: Option<&ResonatorReading>) {
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

        if reading.waterfall.is_empty() || reading.note_labels.is_empty() {
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
            Color32::from_rgb(166, 170, 176),
        );
        painter.text(
            pos2(rect.right() - 18.0, rect.bottom() - 14.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{} active bins", reading.spectrum.len()),
            FontId::proportional(12.0),
            Color32::from_rgb(166, 170, 176),
        );
    }
}

/// One labelled knob row for the mobile settings strip: a caption, the slider
/// drawn by `body`, and the numeric readout `body` returns. `body` flips
/// `changed` via its `&mut bool` when the user moved the control. The trio sits
/// on one fixed `horizontal` row; rows are stacked by the caller's vertical
/// layout (see `draw_mobile_snail_settings` for why we don't wrap them).
#[cfg(target_os = "android")]
fn mobile_slider(ui: &mut Ui, label: &str, changed: &mut bool, body: impl FnOnce(&mut Ui, &mut bool) -> String) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .color(Color32::from_rgb(205, 194, 176))
                .strong(),
        );
        let value = body(ui, changed);
        ui.label(
            egui::RichText::new(value)
                .color(Color32::from_rgb(226, 216, 201))
                .monospace(),
        );
    });
}
