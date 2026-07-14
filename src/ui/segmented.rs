//! The canonical pill / segmented toggle button — a single source of truth for
//! every binary or enum selector in the app (Source Microphone/System,
//! Accidentals, Root, resonator Mode, Monitor on/off, …). The look is the one on
//! the reference screenshot: a warm accent fill with a light outline when
//! selected, a dark fill with a muted outline when idle. Before this widget the
//! same fill/stroke/radius were hand-typed at a dozen call sites and had already
//! drifted (corner radius 12 vs 13 vs 14, some sites dropped the outline
//! entirely). Route every new selector through here so they all read identically.
//!
//! ## Why custom-painted instead of a styled `egui::Button`
//!
//! One reason: vertical text centring. `egui::Button` centres the *galley box* —
//! and for this font that box is **not** symmetric around the glyphs. Measured
//! with `src/bin/font_probe.rs` (Ubuntu-Light, proportional 14 px):
//!
//! ```text
//! font_ascent = 13.0625   font_height = 16.1250   baseline = 13.0
//! real ink of "Microphone" spans  2.0 .. 16.0   (h/i reach 2, p reaches 16)
//! ```
//!
//! So the line box reserves ~2 px of dead leading **above** the ink and ~0 below.
//! Centring that box therefore drops the glyphs ~1 px low: in a 28 px pill
//! "Microphone" lands with 8 px above and 6 px below. That is the bug — it lives
//! in the font's metrics, not in egui's arithmetic.
//!
//! We instead centre the font's **ink band** (tallest ascender … deepest
//! descender), measured at the live `pixels_per_point` from the constant
//! [`INK_REF`] string — deliberately NOT each label's own ink. Per-label
//! measuring would put "Magnitude" (descender) and "Power" (none) on baselines
//! 1 px apart inside the same segmented row; one constant reference guarantees a
//! single shared baseline.
//!
//! The trade-off is real and was chosen deliberately: descender labels
//! ("Microphone", "System", "Magnitude") now sit dead centre, while
//! descender-less ones ("Power", root notes) sit ~1 px high. It cannot be split
//! any finer — epaint rounds every galley to a whole physical pixel
//! (`round_text_to_pixels`, on by default), so near the centre the only
//! renderable origins are 5.0 and 6.0 with nothing in between. Flipping the rule
//! back is a one-line change: set [`INK_REF`] to `"X"`.
//!
//! [`SegmentedButton::text_dy`] remains as a hand-nudge escape hatch.

use eframe::egui::{
    Color32,
    CornerRadius,
    FontId,
    Response,
    Sense,
    Stroke,
    StrokeKind,
    Ui,
    Widget,
    pos2,
    vec2,
};

use crate::ui::theme::PANEL_FILL;

/// Warm fill of a *selected* pill (the reference screenshot's brown).
pub const PILL_ACCENT_FILL: Color32 = Color32::from_rgb(112, 86, 72);
/// Light outline of a *selected* pill.
pub const PILL_ACCENT_STROKE: Color32 = Color32::from_rgb(207, 187, 166);
/// Dark fill of an *idle* (unselected) pill.
pub const PILL_IDLE_FILL: Color32 = Color32::from_rgb(42, 46, 52);
/// Muted outline of an *idle* pill.
pub const PILL_IDLE_STROKE: Color32 = Color32::from_rgb(84, 89, 97);
/// Default label colour on a pill (matches the app's warm off-white value text).
pub const PILL_TEXT: Color32 = Color32::from_rgb(226, 216, 201);
/// Colour of the caption that introduces a row of pills ("Source", "Root", …).
pub const PILL_CAPTION: Color32 = Color32::from_rgb(205, 194, 176);

/// Fixed outer height of every pill. The whole point of the widget is that this
/// is a hard number we centre text against, not something egui derives from font
/// metrics per call site.
const PILL_HEIGHT: f32 = 28.0;
/// Corner radius — the canonical value from the screenshot (others had drifted).
const PILL_RADIUS: u8 = 14;
/// Horizontal breathing room on each side of the label when the pill is sized to
/// its text (i.e. when `min_width` does not dominate).
const H_PAD: f32 = 14.0;
/// Matches `TextStyle::Button` in [`crate::ui::theme`].
const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Reference string spanning the font's full ink band: a cap plus the tallest
/// ascender (`h`) down to the deepest descenders (`g j p q y`). Constant on
/// purpose — see the module docs on why we never measure the label itself.
/// Changing this to `"X"` switches the widget to the classic caps-band rule
/// (descender-less labels dead centre, longer ones ~1 px low).
const INK_REF: &str = "Xhgjpqy";
/// How far hover lightens the fill/outline toward white (0 = none, 1 = white).
const HOVER_LIGHTEN: f32 = 0.10;
/// How far a disabled pill fades toward the panel background.
const DISABLED_FADE: f32 = 0.5;

/// A canonically-styled toggle / action pill. Build with [`SegmentedButton::new`]
/// for a two-state selector, or [`SegmentedButton::colored`] for a one-off accent
/// action, then hand it to [`Ui::add`] / [`Ui::add_enabled`] like any widget.
pub struct SegmentedButton<'a> {
    text:       &'a str,
    fill:       Color32,
    stroke:     Color32,
    text_color: Color32,
    /// Lower bound on width. The pill still grows to fit long labels; this only
    /// stops short labels from collapsing (keeps a row of pills tidy).
    min_width:  f32,
    font_size:  f32,
    /// Hand-tuned vertical bias in logical px (`+` = down). The caps-band centring
    /// is already optical; this exists only to nudge if a font asks for it.
    text_dy:    f32,
}

impl<'a> SegmentedButton<'a> {
    /// Canonical two-state toggle: `selected` picks the warm accent, otherwise the
    /// dark idle look. This is THE constructor for enum / binary selectors.
    pub fn new(text: &'a str, selected: bool) -> Self {
        let (fill, stroke) = if selected {
            (PILL_ACCENT_FILL, PILL_ACCENT_STROKE)
        } else {
            (PILL_IDLE_FILL, PILL_IDLE_STROKE)
        };
        Self {
            text,
            fill,
            stroke,
            text_color: PILL_TEXT,
            min_width: 0.0,
            font_size: DEFAULT_FONT_SIZE,
            text_dy: 0.0,
        }
    }

    /// A neutral action pill that carries no selected/unselected state — e.g. the
    /// drone panel's "Clear" / "+Octave" quick actions. Reads as an idle pill.
    /// Prefer [`Self::new`] whenever the pill actually reflects a state, so the
    /// call site does not have to lie with a hard-coded `false`.
    pub fn action(text: &'a str) -> Self {
        Self {
            text,
            fill: PILL_IDLE_FILL,
            stroke: PILL_IDLE_STROKE,
            text_color: PILL_TEXT,
            min_width: 0.0,
            font_size: DEFAULT_FONT_SIZE,
            text_dy: 0.0,
        }
    }

    /// A one-off accent action that is not a toggle (e.g. the green "Play test
    /// note" button). Same geometry and centring as a toggle, different colours.
    pub fn colored(text: &'a str, fill: Color32, stroke: Color32) -> Self {
        Self {
            text,
            fill,
            stroke,
            text_color: PILL_TEXT,
            min_width: 0.0,
            font_size: DEFAULT_FONT_SIZE,
            text_dy: 0.0,
        }
    }

    /// Keep short pills from collapsing below `w` logical px.
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }

    /// Override the label colour (default [`PILL_TEXT`]).
    pub fn text_color(mut self, c: Color32) -> Self {
        self.text_color = c;
        self
    }

    /// Override the label font size (default matches `TextStyle::Button`, 14 px).
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Hand-nudge the text baseline, `+` = down, in logical px. Use only to trim a
    /// font-specific optical offset the ink-band rule does not already handle.
    /// Note that epaint rounds the result to a whole physical pixel, so a nudge
    /// smaller than one physical pixel may render as no change at all.
    pub fn text_dy(mut self, dy: f32) -> Self {
        self.text_dy = dy;
        self
    }
}

impl Widget for SegmentedButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let font_id = FontId::proportional(self.font_size);

        // Hover only tints the fill/outline, never the text, so the label colour
        // is fixed here (depends solely on enabled-state) and we lay the galley out
        // exactly once, before allocation, to measure and later paint it.
        let text_color = if enabled {
            self.text_color
        } else {
            lerp_rgb(self.text_color, PANEL_FILL, DISABLED_FADE)
        };
        let galley = ui
            .painter()
            .layout_no_wrap(self.text.to_owned(), font_id.clone(), text_color);

        let band_mid = ink_band_mid(ui, &font_id);
        let width = (galley.size().x + 2.0 * H_PAD).max(self.min_width);
        let (rect, response) = ui.allocate_exact_size(vec2(width, PILL_HEIGHT), Sense::click());

        if ui.is_rect_visible(rect) {
            let hovered = enabled && response.hovered();
            let (fill, stroke) = if !enabled {
                (
                    lerp_rgb(self.fill, PANEL_FILL, DISABLED_FADE),
                    lerp_rgb(self.stroke, PANEL_FILL, DISABLED_FADE),
                )
            } else if hovered {
                (
                    lerp_rgb(self.fill, Color32::WHITE, HOVER_LIGHTEN),
                    lerp_rgb(self.stroke, Color32::WHITE, HOVER_LIGHTEN),
                )
            } else {
                (self.fill, self.stroke)
            };

            let radius = CornerRadius::same(PILL_RADIUS);
            let painter = ui.painter();
            painter.rect_filled(rect, radius, fill);
            painter.rect_stroke(rect, radius, Stroke::new(1.0, stroke), StrokeKind::Inside);

            // No manual pixel-snapping here: epaint's tessellator already rounds
            // every galley origin to the physical pixel grid (`round_text_to_pixels`,
            // on by default), so rounding again would only risk double-rounding.
            let pos = pos2(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - band_mid + self.text_dy,
            );
            painter.galley(pos, galley, text_color);
        }

        response
    }
}

/// The caption that introduces a row of pills — "Source", "Device", "Root", "Mode".
///
/// This exists because `ui.horizontal_wrapped` **cannot vertically centre its
/// items**: a wrapping layout does not know a row's height until the row is closed,
/// so it top-aligns instead. A plain `ui.label("Source")` is only ~16 px tall next
/// to a 28 px pill, so it ends up sitting ~4 px *above* the pill's text — a gap
/// wide enough to read as "the text isn't centred", and one that no amount of
/// tuning *inside* the pill can fix.
///
/// So the caption claims the same [`PILL_HEIGHT`] band as a pill and places its
/// text with the very same ink-band rule ([`ink_band_mid`]). Caption and pills then
/// share one baseline **by construction**, wrapping or not, at any font size or
/// `pixels_per_point` — rather than by a hand-tuned offset that would rot.
pub struct RowCaption<'a> {
    text:      &'a str,
    color:     Color32,
    font_size: f32,
}

impl<'a> RowCaption<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            color: PILL_CAPTION,
            font_size: DEFAULT_FONT_SIZE,
        }
    }

    /// Override the caption colour (default [`PILL_CAPTION`]).
    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    /// Override the caption font size (default matches `TextStyle::Button`, 14 px).
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }
}

impl Widget for RowCaption<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let font_id = FontId::proportional(self.font_size);
        let galley = ui
            .painter()
            .layout_no_wrap(self.text.to_owned(), font_id.clone(), self.color);
        let band_mid = ink_band_mid(ui, &font_id);

        // Same height band as a pill — that is the whole point. Width hugs the text.
        let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, PILL_HEIGHT), Sense::hover());

        if ui.is_rect_visible(rect) {
            let pos = pos2(rect.left(), rect.center().y - band_mid);
            ui.painter().galley(pos, galley, self.color);
        }
        response
    }
}

/// Midpoint of the font's ink band at `font_id`, in galley-local coordinates —
/// i.e. how far below a galley's origin the glyphs are optically centred.
///
/// Measured live rather than baked in as a constant for two reasons: the band
/// moves with `font_size`, and the rasterised ink extents quantise to the
/// physical pixel grid, so the correct value differs between a 1× and a 2×
/// display. Measuring the same [`INK_REF`] every frame tracks both for free.
/// The galley is memoised by egui, so this costs a cache lookup after frame one.
fn ink_band_mid(ui: &Ui, font_id: &FontId) -> f32 {
    let refg = ui
        .painter()
        .layout_no_wrap(INK_REF.to_owned(), font_id.clone(), Color32::WHITE);
    (refg.mesh_bounds.min.y + refg.mesh_bounds.max.y) * 0.5
}

/// Per-channel linear-ish blend of two sRGB colours in gamma space. Good enough
/// for the subtle hover-lighten and disabled-fade tints; not a colour-managed
/// lerp (we do not need one for a ±10% nudge).
fn lerp_rgb(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
