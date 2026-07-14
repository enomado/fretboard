//! Design tokens — the one place a raw colour / radius / size literal may live.
//!
//! Everything else in the app names a **role** (`color::TEXT_HINT`) instead of a
//! value (`Color32::from_rgb(145, 151, 160)`). Before this module the codebase had
//! **292 `from_rgb` calls with 129 distinct colours** for maybe a dozen real roles,
//! and it had drifted exactly the way the pill styling had: four near-identical
//! greys — `(145,151,160)`, `(152,158,165)`, `(150,156,164)`, `(139,143,149)` —
//! all meaning "secondary hint text", differing by amounts no one could see and no
//! one had chosen. They are now one [`color::TEXT_HINT`].
//!
//! ## Scope: chrome, not data
//!
//! Tokens cover **UI chrome** — text roles, surfaces, pill palette, geometry, the
//! type scale. They deliberately do NOT cover *data* colour, which is a different
//! thing wearing the same type: `theme::intonation_color` (green→red by cents),
//! the spectrum/waterfall ramps, fretboard note tints. Those encode meaning per
//! value; flattening them into named chrome roles would be a category error. They
//! stay as functions near the code that reasons about them.
//!
//! ## Why roles and not `Visuals`
//!
//! egui's `Visuals`/`widgets.*` already styles *stock* widgets and [`super::theme`]
//! sets it. These tokens are for everything we paint ourselves, where `Visuals` has
//! no opinion. egui 0.35's `Classes` would eventually be the idiomatic home for
//! this, but its engine is a stub — `widget_style(&self, _classes, state)` ignores
//! the `classes` argument entirely and there is no class→style resolver. Revisit at
//! 0.36+.

use eframe::egui::Color32;

/// Semantic colour roles for UI chrome.
pub mod color {
    use eframe::egui::Color32;

    // ── Text ───────────────────────────────────────────────────────────────────
    /// Card and panel titles ("Live analysis", "Input Scope").
    pub const TEXT_HEADING: Color32 = Color32::from_rgb(228, 220, 208);
    /// Numeric readouts beside a control ("442.0 Hz", "1.4x", "70%").
    pub const TEXT_VALUE: Color32 = Color32::from_rgb(226, 216, 201);
    /// The caption that introduces a row of controls ("Source", "Root", "Mode").
    pub const TEXT_CAPTION: Color32 = Color32::from_rgb(205, 194, 176);
    /// Secondary explanatory text. Collapsed from four near-identical greys — see
    /// the module docs; if something here needs to be dimmer, that is a new role
    /// with a reason, not a fifth shade.
    pub const TEXT_HINT: Color32 = Color32::from_rgb(145, 151, 160);
    /// Text inside a status badge (the small read-only `pill()` chips).
    pub const TEXT_BADGE: Color32 = Color32::from_rgb(201, 195, 184);

    // ── Pills (see `super::super::segmented`) ──────────────────────────────────
    /// Fill of a *selected* pill.
    pub const ACCENT_FILL: Color32 = Color32::from_rgb(112, 86, 72);
    /// Outline of a *selected* pill.
    pub const ACCENT_STROKE: Color32 = Color32::from_rgb(207, 187, 166);
    /// Fill of an *idle* pill.
    pub const IDLE_FILL: Color32 = Color32::from_rgb(42, 46, 52);
    /// Outline of an *idle* pill.
    pub const IDLE_STROKE: Color32 = Color32::from_rgb(84, 89, 97);

    // ── egui `Visuals` palette (stock widgets: combos, sliders, plain buttons) ──
    //
    // NOTE — unresolved drift, same class as the four greys: there are **three**
    // near-identical accents in the app — [`ACCENT_FILL`] `(112,86,72)` on pills,
    // [`SELECTION_FILL`] `(121,92,74)`, and [`WIDGET_ACTIVE_FILL`] `(116,89,73)`.
    // Likewise [`IDLE_FILL`] `(42,46,52)` vs [`WIDGET_IDLE_FILL`] `(36,40,46)`.
    // They are named rather than merged because collapsing them changes pixels on
    // stock widgets, which is a design call, not a mechanical one.
    /// Default text colour (`Visuals::override_text_color`).
    pub const TEXT_DEFAULT: Color32 = Color32::from_rgb(220, 215, 205);
    /// The window background behind all cards (`Visuals::panel_fill`).
    pub const APP_BG: Color32 = Color32::from_rgb(16, 20, 25);
    /// Sunken wells: text edit / combo interiors (`Visuals::extreme_bg_color`).
    pub const WELL_BG: Color32 = Color32::from_rgb(18, 22, 27);
    /// Inline code background (`Visuals::code_bg_color`).
    pub const CODE_BG: Color32 = Color32::from_rgb(30, 34, 40);
    /// Selected-text / selected-item highlight.
    pub const SELECTION_FILL: Color32 = Color32::from_rgb(121, 92, 74);
    /// Outline of the selection highlight.
    pub const SELECTION_STROKE: Color32 = Color32::from_rgb(214, 194, 171);
    /// Outline of a non-interactive frame.
    pub const FRAME_STROKE: Color32 = Color32::from_rgb(58, 63, 71);
    /// Stock widget, resting.
    pub const WIDGET_IDLE_FILL: Color32 = Color32::from_rgb(36, 40, 46);
    /// Stock widget outline, resting.
    pub const WIDGET_IDLE_STROKE: Color32 = Color32::from_rgb(78, 82, 90);
    /// Stock widget, hovered.
    pub const WIDGET_HOVER_FILL: Color32 = Color32::from_rgb(48, 53, 61);
    /// Stock widget outline, hovered.
    pub const WIDGET_HOVER_STROKE: Color32 = Color32::from_rgb(120, 126, 136);
    /// Stock widget text, hovered.
    pub const WIDGET_HOVER_TEXT: Color32 = Color32::from_rgb(232, 227, 217);
    /// Stock widget, pressed / active.
    pub const WIDGET_ACTIVE_FILL: Color32 = Color32::from_rgb(116, 89, 73);
    /// Stock widget outline, pressed / active.
    pub const WIDGET_ACTIVE_STROKE: Color32 = Color32::from_rgb(213, 190, 162);

    // ── Docking tabs (`egui_tiles`, painted by the library from these) ─────────
    /// The tab strip's background.
    pub const TAB_BAR_BG: Color32 = Color32::from_rgb(18, 22, 27);
    /// An inactive tab — deliberately darker than [`IDLE_FILL`] so the strip reads
    /// as background rather than as a row of buttons.
    pub const TAB_IDLE_FILL: Color32 = Color32::from_rgb(34, 38, 44);
    /// An inactive tab's outline.
    pub const TAB_IDLE_STROKE: Color32 = Color32::from_rgb(76, 82, 90);
    /// The hairline under the tab strip.
    pub const TAB_HLINE: Color32 = Color32::from_rgb(56, 61, 69);
    /// Active tab label.
    pub const TAB_TEXT_ACTIVE: Color32 = Color32::from_rgb(235, 227, 216);
    /// Inactive tab label.
    pub const TAB_TEXT_IDLE: Color32 = Color32::from_rgb(188, 192, 198);

    // ── Surfaces ───────────────────────────────────────────────────────────────
    /// Background of a card / panel frame.
    pub const PANEL_FILL: Color32 = Color32::from_rgb(24, 27, 31);
    /// Outline of a card / panel frame.
    pub const CARD_STROKE: Color32 = Color32::from_rgb(61, 66, 74);
    /// Background of a plot / meter well (spectrum, level bar, snail).
    pub const PLOT_BG: Color32 = Color32::from_rgb(29, 32, 37);
    /// Outline of a plot / meter well.
    pub const PLOT_STROKE: Color32 = Color32::from_rgb(72, 76, 82);
    /// Background of a status badge chip.
    pub const BADGE_FILL: Color32 = Color32::from_rgb(64, 68, 73);
}

/// Corner radii. These name the values that exist today rather than unifying them:
/// `CARD` (18) and `PANEL` (22) really are different frames, and collapsing them
/// would change pixels, which is a design call and not a mechanical one.
pub mod radius {
    /// Pills, and the level meter that has to match their band.
    pub const PILL: u8 = 14;
    /// A frame nested inside a card (e.g. the signal-path diagnostics box).
    pub const INNER: u8 = 12;
    /// Controls / drone card frames.
    pub const CARD: u8 = 18;
    /// Live-analysis / resonator panel frames.
    pub const PANEL: u8 = 22;
    /// The drone note keyboard's keys — deliberately square-ish, not pill-like.
    pub const KEY: u8 = 6;
}

/// Vertical rhythm and padding.
pub mod space {
    /// The height band every row-level control shares — pills, captions, combos.
    /// This is what makes a row's baselines line up; see `super::super::segmented`.
    pub const ROW_H: f32 = 28.0;
    /// Default gap between items in a row (egui `item_spacing`).
    pub const GAP: f32 = 10.0;
    /// Inner margin of a card frame.
    pub const CARD_PAD: f32 = 16.0;
}

/// The type scale, in logical px.
pub mod font {
    /// Card titles.
    pub const HEADING: f32 = 24.0;
    /// Default body text.
    pub const BODY: f32 = 15.0;
    /// Control labels and pill text — the size row baselines are computed against.
    pub const BUTTON: f32 = 14.0;
    /// Hints, secondary captions, debug readouts.
    pub const CAPTION: f32 = 12.0;
}

/// Blend two colours in gamma space. Lives here because tinting a token (hover
/// lighten, disabled fade) is itself a token-level operation — callers should
/// derive from a role, never hand-pick a nearby literal, which is how the four
/// greys happened in the first place.
pub fn lerp_rgb(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
