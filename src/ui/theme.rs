use std::sync::Arc;

use eframe::egui::{
    Color32,
    Context,
    CornerRadius,
    FontData,
    FontDefinitions,
    FontFamily,
    FontId,
    Stroke,
    Style,
    TextStyle,
    Visuals,
    vec2,
};

pub const PANEL_FILL: Color32 = Color32::from_rgb(24, 27, 31);

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
    fonts
        .families
        .insert(FontFamily::Name(MUSIC_FAMILY.into()), vec![
            "noto_music".to_owned(),
        ]);
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

fn visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(220, 215, 205));
    visuals.panel_fill = Color32::from_rgb(16, 20, 25);
    visuals.window_fill = PANEL_FILL;
    visuals.extreme_bg_color = Color32::from_rgb(18, 22, 27);
    visuals.code_bg_color = Color32::from_rgb(30, 34, 40);
    visuals.selection.bg_fill = Color32::from_rgb(121, 92, 74);
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(214, 194, 171));
    visuals.widgets.noninteractive.bg_fill = PANEL_FILL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(58, 63, 71));
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 40, 46);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(78, 82, 90));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(14);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 53, 61);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(120, 126, 136));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(232, 227, 217));
    visuals.widgets.hovered.corner_radius = CornerRadius::same(14);
    visuals.widgets.active.bg_fill = Color32::from_rgb(116, 89, 73);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(213, 190, 162));
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
