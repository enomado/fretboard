use std::sync::Arc;

use eframe::egui::os::OperatingSystem;
use eframe::egui::{
    Color32,
    Context,
    CornerRadius,
    FontData,
    FontDefinitions,
    FontFamily,
    FontId,
    Margin,
    Stroke,
    Style,
    TextStyle,
    Visuals,
    vec2,
};

use crate::ui::tokens::{
    color,
    space,
};

/// Name of the custom font family that carries the music glyphs (clef,
/// accidentals, noteheads). Registered by [`install_fonts`].
const MUSIC_FAMILY: &str = "music";

/// A [`FontId`] in the embedded Noto Music family at the given pixel size — the
/// only way to reach the clef/accidental glyphs. See [`install_fonts`].
pub fn music_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MUSIC_FAMILY.into()))
}

/// Register the embedded Noto Music font (SIL OFL 1.1) as a named family so the
/// staff panel can draw a real treble clef (U+1D11E) and ♯/♭/♮ accidentals.
/// Keeps egui's default families for all normal text; only adds "music".
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto_music".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/NotoMusic-Regular.ttf"
        ))),
    );
    fonts.families.insert(
        FontFamily::Name(MUSIC_FAMILY.into()),
        vec!["noto_music".to_owned()],
    );
    // Also chain Noto Music as the *last* fallback of the normal text families so
    // ♯/♭/♮ (U+266D..F) rendered in plain UI labels — the key-signature picker's
    // "E♭  3♭", the note pills — resolve to a real glyph instead of tofu boxes.
    // It must go last: egui derives a row's height from the family's *first*
    // font, so appending Noto Music (whose em is ~2.35× tall) does not inflate
    // line heights; it is consulted only for glyphs Ubuntu/Hack lack.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto_music".to_owned());
    }
    // And Hack (already bundled for Monospace) after it, for punctuation neither
    // Ubuntu-Light nor Noto Music carries: the prime of "SWIPE′" (U+2032) on the
    // layer pills was drawn as a tofu box. Appended after Noto Music, so every
    // glyph that already rendered keeps its font; only former tofu changes.
    let hack = fonts
        .families
        .get(&FontFamily::Monospace)
        .and_then(|faces| faces.first())
        .expect("egui's default Monospace family starts with its bundled Hack")
        .clone();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .push(hack);
    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &Context) {
    let mut style: Style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(10.0, 7.0);
    style.spacing.combo_width = 160.0;
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style.visuals = visuals();
    ctx.set_global_style(style);
}

/// Is the app on a touch screen — the phone — rather than under a mouse?
///
/// Panels shared by the desktop workspace and the phone shell ask this where a finger
/// needs something a pointer does not: a touch-sized target, a pinch instead of a
/// modifier+wheel, words that name the gesture the user actually has. Read off egui's
/// `os()`, which eframe sets from the build target (Android on the phone), so it is a
/// fact about the device, not a setting — and a phone-sized kittest harness says
/// `set_os(Android)` to get the phone's panels on the host.
pub fn is_touch_screen(ctx: &Context) -> bool {
    matches!(ctx.os(), OperatingSystem::Android | OperatingSystem::IOS)
}

/// The least height a control may have here: a finger's [`space::TOUCH_H`] on a touch
/// screen, else 0 — the widget's own size, so the desktop is untouched. For
/// `Button::min_size` and the like on panels both shells draw.
pub fn min_target_h(ctx: &Context) -> f32 {
    if is_touch_screen(ctx) { space::TOUCH_H } else { 0.0 }
}

/// A card's inner margin and corner radius: the card's own (`desktop_pad`,
/// `desktop_radius`) under a mouse; on a touch screen [`space::TOUCH_CARD_PAD`] and a
/// radius to match.
///
/// On a 360 dp phone a 14–16 dp card margin, inside the shell's own 8, cost a tenth of the
/// width (Stas, 2026-10-22: «марджины меньше»). The radius shrinks with it: content is
/// square-cornered, and at a 6 dp inset a 22 dp curve would cut across its corners — a
/// radius `r` clears an inset `m` while `m ≥ r·(1 − 1/√2)`.
pub fn card_pad(ctx: &Context, desktop_pad: i8, desktop_radius: u8) -> (Margin, CornerRadius) {
    if is_touch_screen(ctx) {
        (Margin::same(space::TOUCH_CARD_PAD), CornerRadius::same(10))
    } else {
        (Margin::same(desktop_pad), CornerRadius::same(desktop_radius))
    }
}

fn visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(color::TEXT_DEFAULT);
    visuals.panel_fill = color::APP_BG;
    visuals.window_fill = color::PANEL_FILL;
    visuals.extreme_bg_color = color::WELL_BG;
    visuals.code_bg_color = color::CODE_BG;
    visuals.selection.bg_fill = color::SELECTION_FILL;
    visuals.selection.stroke = Stroke::new(1.0_f32, color::SELECTION_STROKE);
    visuals.widgets.noninteractive.bg_fill = color::PANEL_FILL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, color::FRAME_STROKE);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);
    visuals.widgets.inactive.bg_fill = color::WIDGET_IDLE_FILL;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, color::WIDGET_IDLE_STROKE);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(14);
    visuals.widgets.hovered.bg_fill = color::WIDGET_HOVER_FILL;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, color::WIDGET_HOVER_STROKE);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, color::WIDGET_HOVER_TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(14);
    visuals.widgets.active.bg_fill = color::WIDGET_ACTIVE_FILL;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, color::WIDGET_ACTIVE_STROKE);
    visuals.widgets.active.corner_radius = CornerRadius::same(14);
    visuals
}

pub fn fretboard_fill() -> Color32 {
    Color32::from_rgb(42, 31, 24)
}

/// Intonation feedback colour: green (in tune) → red (far off) by absolute cents.
/// Bolder than the app's muted `cents_color` because it is the violin trainer's
/// primary feedback — shared by the staff panel and the pitch-roll panel so both
/// grade intonation with one palette.
pub fn intonation_color(cents: f32) -> Color32 {
    match cents.abs() {
        a if a < 5.0 => Color32::from_rgb(90, 220, 110),
        a if a < 12.0 => Color32::from_rgb(160, 214, 92),
        a if a < 22.0 => Color32::from_rgb(232, 204, 84),
        a if a < 35.0 => Color32::from_rgb(236, 150, 72),
        _ => Color32::from_rgb(232, 96, 84),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        Context,
        FontId,
    };

    /// REGRESSION: the symbols the UI writes in plain labels have a glyph in the
    /// app's fonts — and are not drawn as tofu boxes.
    ///
    /// egui 0.37 moved its emoji fonts behind `monochrome_emoji_fonts`, so ▶/■ on
    /// the Play/Stop pills (drone, corpus replay) silently became tofu; ● (recording),
    /// ′ (SWIPE′), → and the verdict marks never had one. The phone-layout kittest
    /// catches glyphs on screen by default, but these only appear while playing,
    /// recording or after a take — states that test does not reach.
    #[test]
    fn ui_symbols_have_a_glyph() {
        let ctx = Context::default();
        super::install_fonts(&ctx);
        // Font definitions take effect at the start of the next frame.
        let _ = ctx.run_ui(Default::default(), |_| {});
        for (font, symbols) in [
            (FontId::proportional(14.0), "▶■●′→√×♯♭♮"),
            (FontId::monospace(14.0), "√×"),
        ] {
            for c in symbols.chars() {
                assert!(
                    ctx.fonts_mut(|fonts| fonts.has_glyph(&font, c)),
                    "{c:?} (U+{:04X}) has no glyph in {:?}",
                    c as u32,
                    font.family
                );
            }
        }
    }
}
