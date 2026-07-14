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
//! ## Two passes, and why the first one was not enough
//!
//! The first pass named the roles and converted the obvious chrome — card fills,
//! titles, hints — and it did kill those four greys. But it left the *renderers'*
//! internals on literals, and the numbers told on it: calls fell 292 → 131 while
//! distinct colours only moved 129 → 126. It had removed **repeats of common
//! colours, not the diversity** — so the class of drift survived even though every
//! example named above was dead. Four fresh greys had already grown around
//! [`color::TEXT_HINT`] in the files the pass never reached.
//!
//! The second pass took the renderers (`pianoroll`, `waterfall`, `snail`, the
//! wheel, the staff) and the `pill()` API. Calls 131 → 55, distinct 126 → 95, and
//! — the metric that actually matters — **no two literals now sit within 25 units
//! of the same token**. There is no cluster left to collapse.
//!
//! The guard against a third pass being needed: `pill()` takes **no colours**. A
//! role that must be re-stated at every call site is a role that will drift, which
//! is exactly how one chip style became four. Prefer an API that cannot say the
//! wrong thing over a token that callers must remember to reach for.
//!
//! ## Token, or a file-local const?
//!
//! **Token if the role is spoken in ≥2 modules; a named `const` in the file if it
//! is spoken in one.** `ui::tokens` is a *shared* vocabulary — a global name used
//! once buys nothing and dilutes the list. So [`color::MARKER`] (waterfall +
//! fretboard) is a token, while `pianoroll::PLAYHEAD` is not: only the pitch roll
//! has a playhead. Promote a const when a second module needs it — `STATUS_ERROR`
//! made that trip when the Bluetooth warning started speaking it.
//!
//! Either way the literal gets a **name and a reason**. A file-local const is not
//! a lesser outcome; an anonymous `from_rgb` is.
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
    /// Text *inside* a plot: axis tick labels, row names, and the "nothing yet"
    /// placeholder. This is the dimmer-with-a-reason role [`TEXT_HINT`] invites —
    /// the reason being that plot internals must not compete with the data drawn
    /// over them, which is why they sit below hint text rather than beside it.
    /// Collapsed from six shades spanning `(110,116,126)`…`(128,134,143)`.
    ///
    /// Its value coincides with [`WIDGET_HOVER_STROKE`]. That is arithmetic, not
    /// meaning: one is plot ink, the other a stock widget's outline, and they are
    /// free to diverge. Do not merge them on the strength of the numbers.
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(120, 126, 136);
    /// Text inside a status badge (the small read-only `pill()` chips).
    pub const TEXT_BADGE: Color32 = Color32::from_rgb(201, 195, 184);
    /// Text of a badge in its *empty* state ("waiting for input", "—"). A distinct
    /// role because the chip is saying "no data" rather than reporting a value;
    /// it was previously two hand-passed pairs, `(184,188,196)` and
    /// `(150,156,165)`, for that one idea. Pairs with [`BADGE_FILL_MUTED`].
    pub const TEXT_BADGE_MUTED: Color32 = Color32::from_rgb(184, 188, 196);
    /// The label naming the note a chart is currently focused on — the snail's
    /// "bank focus D4", the scale wheel's winning root. Shared by two unrelated
    /// charts, which is why it is a token and not a const in either.
    pub const TEXT_ACTIVE_NOTE: Color32 = Color32::from_rgb(214, 206, 192);

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
    /// Bed under a *heat* field — the pitch roll, the resonator waterfall.
    /// Deliberately darker than [`PLOT_BG`]: those plots paint faint per-bin
    /// energy, and the dimmest live cell must still read against the bed. The two
    /// were `(20,23,28)` and `(20,23,29)`, which is one value written twice.
    pub const HEAT_BG: Color32 = Color32::from_rgb(20, 23, 28);
    /// Outline of a plot / meter well.
    pub const PLOT_STROKE: Color32 = Color32::from_rgb(72, 76, 82);
    /// Background of a status badge chip.
    pub const BADGE_FILL: Color32 = Color32::from_rgb(64, 68, 73);
    /// Background of an *empty* badge chip. See [`TEXT_BADGE_MUTED`].
    pub const BADGE_FILL_MUTED: Color32 = Color32::from_rgb(56, 61, 68);

    // ── Plot ink (grids, markers — the lines we paint inside a plot) ───────────
    //
    // Distinct from [`FRAME_STROKE`] / [`PLOT_STROKE`], which outline a well from
    // the outside. These sit *under* data and are tuned to stay beneath it.
    /// A faint line dividing rows or cells inside a plot.
    pub const GRID_LINE: Color32 = Color32::from_rgb(36, 40, 47);
    /// A landmark grid line: the octave C on the pitch roll, a meter's centre,
    /// the snail's rings, the wheel's spokes. Collapsed from `(55,60,67)`,
    /// `(58,64,72)`, `(59,64,72)`, `(66,72,82)`, `(70,75,83)`.
    pub const GRID_LINE_STRONG: Color32 = Color32::from_rgb(59, 64, 72);
    /// A cursor / hover marker painted over a plot — the waterfall's column
    /// highlight, the fretboard's hovered dot. Warm on purpose: it is the one
    /// thing in a plot that is *the user's*, not the signal's.
    pub const MARKER: Color32 = Color32::from_rgb(214, 200, 182);

    // ── Status ────────────────────────────────────────────────────────────────
    /// Audio input present but not running.
    pub const STATUS_IDLE: Color32 = Color32::from_rgb(154, 160, 168);
    /// Audio input live and healthy.
    pub const STATUS_LISTENING: Color32 = Color32::from_rgb(185, 194, 176);
    /// Something is wrong, or is about to be — an audio error, and the Bluetooth
    /// latency warning that predicts one.
    pub const STATUS_ERROR: Color32 = Color32::from_rgb(210, 166, 136);

    // ── Transport (shared by the drone panel and the controls' test note) ──────
    /// Fill of a Play / start action.
    pub const PLAY_FILL: Color32 = Color32::from_rgb(42, 78, 72);
    /// Outline and text of a Play / start action.
    pub const PLAY_STROKE: Color32 = Color32::from_rgb(111, 154, 142);
    /// Fill of a Stop action.
    pub const STOP_FILL: Color32 = Color32::from_rgb(120, 58, 52);
    /// Outline and text of a Stop action.
    pub const STOP_STROKE: Color32 = Color32::from_rgb(196, 122, 110);
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
