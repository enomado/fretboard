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
    Popup,
    PopupCloseBehavior,
    Response,
    ScrollArea,
    Sense,
    Shape,
    Stroke,
    StrokeKind,
    TextWrapMode,
    Ui,
    Widget,
    pos2,
    vec2,
};

use crate::ui::tokens::{
    color,
    radius,
    space,
};

// The pill's palette lives in `ui::tokens::color` (ACCENT_FILL / ACCENT_STROKE /
// IDLE_FILL / IDLE_STROKE / TEXT_VALUE / TEXT_CAPTION). This module used to own
// those constants and re-export them; that made two names for one value, which is
// the very thing tokens exist to prevent. Callers take them from `tokens`.

/// Fixed outer height of every pill. The whole point of the widget is that this
/// is a hard number we centre text against, not something egui derives from font
/// metrics per call site. Shared with captions/combos via the token so a row's
/// controls cannot end up on different bands.
const PILL_HEIGHT: f32 = space::ROW_H;
/// Corner radius — the canonical value from the screenshot (others had drifted).
const PILL_RADIUS: u8 = radius::PILL;
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
            (color::ACCENT_FILL, color::ACCENT_STROKE)
        } else {
            (color::IDLE_FILL, color::IDLE_STROKE)
        };
        Self {
            text,
            fill,
            stroke,
            text_color: color::TEXT_VALUE,
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
            fill: color::IDLE_FILL,
            stroke: color::IDLE_STROKE,
            text_color: color::TEXT_VALUE,
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
            text_color: color::TEXT_VALUE,
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

    /// Override the label colour (default [`color::TEXT_VALUE`]).
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
            lerp_rgb(self.text_color, color::PANEL_FILL, DISABLED_FADE)
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
                    lerp_rgb(self.fill, color::PANEL_FILL, DISABLED_FADE),
                    lerp_rgb(self.stroke, color::PANEL_FILL, DISABLED_FADE),
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
            color: color::TEXT_CAPTION,
            font_size: DEFAULT_FONT_SIZE,
        }
    }

    /// Override the caption colour (default [`color::TEXT_CAPTION`]).
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

/// The canonical dropdown — a pill-shaped combo box that shares the pills' height
/// band and baseline.
///
/// `egui::ComboBox` cannot: it derives its height from the font (galley 16.125 px
/// + 2×`button_padding.y` = 30.125 px, vs the pill's 28) and it centres the
/// *galley box*, which — per the module docs — sits ~1 px above the ink. Stacked
/// in a top-aligned `horizontal_wrapped` row, those two errors compound and land
/// the selected text ~2 px below the [`RowCaption`] beside it. No styling of
/// `ComboBox` can fix that: `button_padding` is applied symmetrically
/// (`Rect::shrink2`), so there is no way to bias the text up by the ink offset.
///
/// So the button half is painted here — same [`PILL_HEIGHT`], same
/// [`ink_band_mid`] rule, therefore the same baseline as pills and captions **by
/// construction**. Only the button is ours; the popup is still egui's
/// [`Popup::menu`], so keyboard handling, close-on-click and scrolling come along
/// for free.
pub struct PillCombo<'a> {
    id_salt:       &'a str,
    selected_text: String,
    /// Lower bound on the pill's total width. `None` = the theme's
    /// `spacing.combo_width`, which is what a bare `egui::ComboBox` would use —
    /// absence genuinely means "no call-site opinion, defer to the theme".
    min_width:     Option<f32>,
    font_size:     f32,
}

/// Width reserved for the caret box at the pill's right end, and the gap that
/// keeps the label clear of it.
const CARET_BOX: f32 = 12.0;
const CARET_GAP: f32 = 8.0;
/// The caret triangle itself, inside [`CARET_BOX`].
const CARET_W: f32 = 9.0;
const CARET_H: f32 = 5.0;

impl<'a> PillCombo<'a> {
    /// `id_salt` names the widget for egui's memory (popup open-state), exactly
    /// like `egui::ComboBox::from_id_salt`.
    pub fn new(id_salt: &'a str, selected_text: impl Into<String>) -> Self {
        Self {
            id_salt,
            selected_text: selected_text.into(),
            min_width: None,
            font_size: DEFAULT_FONT_SIZE,
        }
    }

    /// Keep the pill from collapsing below `w` logical px. It still grows to fit a
    /// long selection — same semantics as `egui::ComboBox::width`.
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = Some(w);
        self
    }

    /// Override the label font size (default matches `TextStyle::Button`, 14 px).
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Draw the pill and, while open, its popup. `contents` fills the popup — put
    /// `selectable_value` / `selectable_label` rows in it, as with `ComboBox`.
    /// Returns the *button's* response (the popup's own is egui's business).
    pub fn show(self, ui: &mut Ui, contents: impl FnOnce(&mut Ui)) -> Response {
        let font_id = FontId::proportional(self.font_size);
        let enabled = ui.is_enabled();
        let text_color = if enabled {
            color::TEXT_VALUE
        } else {
            lerp_rgb(color::TEXT_VALUE, color::PANEL_FILL, DISABLED_FADE)
        };

        // Laid out unwrapped so a long selection widens the pill rather than
        // wrapping it — matching `ComboBox` under `TextWrapMode::Extend`, which is
        // what a horizontal row gives it today.
        let galley = ui
            .painter()
            .layout_no_wrap(self.selected_text, font_id.clone(), text_color);
        let band_mid = ink_band_mid(ui, &font_id);

        let min_width = self.min_width.unwrap_or_else(|| ui.spacing().combo_width);
        let width = (galley.size().x + 2.0 * H_PAD + CARET_GAP + CARET_BOX).max(min_width);

        // `allocate_space` hands back an auto id we deliberately drop: the popup's
        // open-state must survive across frames under a *stable* id, so the button
        // interacts under the salt instead.
        let (_auto_id, rect) = ui.allocate_space(vec2(width, PILL_HEIGHT));
        let id = ui.make_persistent_id(self.id_salt);
        let response = ui.interact(rect, id, Sense::click());

        let popup_id = id.with("popup");
        let is_open = Popup::is_id_open(ui.ctx(), popup_id);

        if ui.is_rect_visible(rect) {
            // An open popup keeps the pill lit, so the button does not go dark the
            // moment the pointer leaves it for the list.
            let lit = enabled && (response.hovered() || is_open);
            let (fill, stroke) = if !enabled {
                (
                    lerp_rgb(color::IDLE_FILL, color::PANEL_FILL, DISABLED_FADE),
                    lerp_rgb(color::IDLE_STROKE, color::PANEL_FILL, DISABLED_FADE),
                )
            } else if lit {
                (
                    lerp_rgb(color::IDLE_FILL, Color32::WHITE, HOVER_LIGHTEN),
                    lerp_rgb(color::IDLE_STROKE, Color32::WHITE, HOVER_LIGHTEN),
                )
            } else {
                (color::IDLE_FILL, color::IDLE_STROKE)
            };

            let radius = CornerRadius::same(PILL_RADIUS);
            let painter = ui.painter();
            painter.rect_filled(rect, radius, fill);
            painter.rect_stroke(rect, radius, Stroke::new(1.0, stroke), StrokeKind::Inside);

            // The label is left-aligned (a dropdown's selection reads from the left),
            // but its *vertical* placement is the pill rule verbatim: ink band centred
            // on the rect's centre.
            painter.galley(
                pos2(rect.left() + H_PAD, rect.center().y - band_mid),
                galley,
                text_color,
            );

            let caret_x = rect.right() - H_PAD - CARET_BOX * 0.5;
            let caret_y = rect.center().y;
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(caret_x - CARET_W * 0.5, caret_y - CARET_H * 0.5),
                    pos2(caret_x + CARET_W * 0.5, caret_y - CARET_H * 0.5),
                    pos2(caret_x, caret_y + CARET_H * 0.5),
                ],
                text_color,
                Stroke::NONE,
            ));
        }

        let popup_height = ui.spacing().combo_height;
        Popup::menu(&response)
            .id(popup_id)
            .width(rect.width())
            .close_behavior(PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                ui.set_min_width(ui.available_width());

                // The app's global 14 px widget rounding + 10 px item spacing turn
                // these short rows into oversized hover "pills" that balloon above and
                // below the row and appear to float as the pointer moves. Every combo
                // popup wants the same snug, squared-off override, so it lives here
                // rather than being re-typed per call site.
                let style = ui.style_mut();
                style.spacing.item_spacing.y = 2.0;
                style.spacing.button_padding.y = 3.0;
                for widget in [
                    &mut style.visuals.widgets.hovered,
                    &mut style.visuals.widgets.active,
                    &mut style.visuals.widgets.inactive,
                ] {
                    widget.corner_radius = CornerRadius::same(6);
                }
                // A popup is often as narrow as its button; wrapping would break the
                // labels absurdly early, so let them set the width instead.
                style.wrap_mode = Some(TextWrapMode::Extend);

                ScrollArea::vertical().max_height(popup_height).show(ui, contents);
            });

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
