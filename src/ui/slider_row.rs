//! The canonical knob row: a caption, a slider and the value it leaves, as one item.
//!
//! Before this widget the trio was typed out at every knob — `drone_panel::slider_row`,
//! the snail's `mobile_slider`, about fifteen inline in `controls.rs` and the Scale
//! Finder — as three separate widgets, and on a phone that broke two ways:
//!
//! - in a wrapping row the three wrap independently: a caption ended one line and its
//!   slider opened the next ("Min Hz" | slider, "Monitor gain" | slider);
//! - egui's `Slider` wraps itself in a `ui.horizontal`, so `add_sized([w, 44])` never
//!   reached it: its rect — and hit area — stayed `max(body text, interact_size.y)` ≈ 20
//!   dp whatever size the call site asked for. Even the snail's "44 dp" knobs were 20.
//!
//! So the row is allocated as one fixed-width rect (a wrapping row moves it whole), at
//! the row band of the pills ([`row_band`]: 28 under a mouse, a finger's 44 on a touch
//! screen), and the slider gets its height the only way egui gives one —
//! `interact_size.y` ([`theme::finger_sized`]). The readout keeps one width for the
//! whole range, measured off the range's ends, so dragging never re-wraps the row.

use std::ops::RangeInclusive;

use eframe::egui::emath::Numeric;
use eframe::egui::{
    Align,
    Layout,
    Response,
    RichText,
    Slider,
    SliderClamping,
    TextStyle,
    Ui,
    Widget,
    WidgetInfo,
    vec2,
};

use crate::ui::segmented::{
    RowCaption,
    row_band,
};
use crate::ui::theme;
use crate::ui::tokens::color;

/// The slider's width when the line has room for it.
const DEFAULT_SLIDER_W: f32 = 160.0;
/// The least a slider shrinks to on a line too narrow for its preferred width — still a
/// usable throw for a thumb.
const MIN_SLIDER_W: f32 = 80.0;

/// Caption + slider over `range` + `readout` of its value: one row item, a finger target
/// on a touch screen. `ui.add` returns the slider's response (`changed()` when moved).
///
/// The slider is named after its caption for accessibility (TalkBack reads "Spread", not
/// an anonymous slider), unless [`Self::accessible_name`] says otherwise.
pub struct SliderRow<'a, N, F> {
    caption:         RowCaption<'a>,
    value:           &'a mut N,
    range:           RangeInclusive<N>,
    readout:         F,
    /// Preferred slider width; a narrower line gets less, down to [`MIN_SLIDER_W`].
    width:           f32,
    logarithmic:     bool,
    clamp_always:    bool,
    trailing_fill:   bool,
    /// The slider's accessible name when its caption is not one ("to" between two range
    /// ends); `None` = the caption names it.
    accessible_name: Option<&'a str>,
    /// The widest readout the range can show, for a format whose width is not bounded by
    /// the range's ends (note names: "C#4" between "C0" and "C8"); `None` = the ends.
    readout_sample:  Option<&'a str>,
}

impl<'a, N: Numeric, F: Fn(N) -> String> SliderRow<'a, N, F> {
    pub fn new(caption: RowCaption<'a>, value: &'a mut N, range: RangeInclusive<N>, readout: F) -> Self {
        Self {
            caption,
            value,
            range,
            readout,
            width: DEFAULT_SLIDER_W,
            logarithmic: false,
            clamp_always: false,
            trailing_fill: false,
            accessible_name: None,
            readout_sample: None,
        }
    }

    /// Reserve the readout's width for `sample`, the widest text it can show, when that
    /// is not one of the range's ends (see the field).
    pub fn readout_sample(mut self, sample: &'a str) -> Self {
        self.readout_sample = Some(sample);
        self
    }

    /// Preferred slider width in logical px (default 160).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// A logarithmic slider (`Slider::logarithmic`).
    pub fn logarithmic(mut self) -> Self {
        self.logarithmic = true;
        self
    }

    /// Clamp the value into the range even when it was set from elsewhere
    /// (`SliderClamping::Always`).
    pub fn clamp_always(mut self) -> Self {
        self.clamp_always = true;
        self
    }

    /// Fill the rail left of the handle (`Slider::trailing_fill`).
    pub fn trailing_fill(mut self) -> Self {
        self.trailing_fill = true;
        self
    }

    /// Name the slider `name` for accessibility instead of after its caption.
    pub fn accessible_name(mut self, name: &'a str) -> Self {
        self.accessible_name = Some(name);
        self
    }
}

impl<N: Numeric, F: Fn(N) -> String> Widget for SliderRow<'_, N, F> {
    fn ui(self, ui: &mut Ui) -> Response {
        let band = row_band(ui);
        let gap = ui.spacing().item_spacing.x;
        let font = TextStyle::Monospace.resolve(ui.style());
        // Widest of the readouts the range can show: its two ends — a number's width only
        // grows with its digits, so they bound it — or the caller's sample where they do
        // not; the current value too, so a too-narrow sample cannot cut it.
        let readout_w = [*self.range.start(), *self.range.end(), *self.value]
            .into_iter()
            .map(|v| (self.readout)(v))
            .chain(self.readout_sample.map(str::to_owned))
            .map(|text| {
                ui.painter()
                    .layout_no_wrap(text, font.clone(), color::TEXT_VALUE)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        let fixed = self.caption.width(ui) + gap + gap + readout_w;
        // `max_rect` is the whole line even in a wrapping row, where `available_width`
        // would be what is left of the current one: the row moves to a fresh line whole,
        // and only a line too narrow for it squeezes the slider.
        let slider_w = self.width.min(ui.max_rect().width() - fixed).max(MIN_SLIDER_W);

        let Self {
            caption,
            value,
            range,
            readout,
            logarithmic,
            clamp_always,
            trailing_fill,
            accessible_name,
            ..
        } = self;
        ui.allocate_ui_with_layout(
            vec2(fixed + slider_w, band),
            Layout::left_to_right(Align::Center),
            |ui| {
                let caption = ui.add(caption);
                let response = theme::finger_sized(ui, |ui| {
                    ui.spacing_mut().slider_width = slider_w;
                    let mut slider = Slider::new(&mut *value, range)
                        .show_value(false)
                        .logarithmic(logarithmic)
                        .trailing_fill(trailing_fill);
                    if clamp_always {
                        slider = slider.clamping(SliderClamping::Always);
                    }
                    ui.add(slider)
                });
                let response = match accessible_name {
                    Some(name) => {
                        let enabled = ui.is_enabled();
                        let at = value.to_f64();
                        response.widget_info(|| WidgetInfo::slider(enabled, at, name));
                        response
                    }
                    None => response.labelled_by(caption.id),
                };
                ui.allocate_ui_with_layout(
                    vec2(readout_w, band),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new(readout(*value))
                                .color(color::TEXT_VALUE)
                                .monospace(),
                        )
                    },
                );
                response
            },
        )
        .inner
    }
}
