use eframe::egui::{
    Color32,
    Context,
    CornerRadius,
    FontId,
    Stroke,
    Style,
    TextStyle,
    Visuals,
    vec2,
};

pub const PANEL_FILL: Color32 = Color32::from_rgb(24, 27, 31);

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
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(214, 194, 171));
    visuals.widgets.noninteractive.bg_fill = PANEL_FILL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(58, 63, 71));
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(36, 40, 46);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(78, 82, 90));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(14);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 53, 61);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(120, 126, 136));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(232, 227, 217));
    visuals.widgets.hovered.corner_radius = CornerRadius::same(14);
    visuals.widgets.active.bg_fill = Color32::from_rgb(116, 89, 73);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(213, 190, 162));
    visuals.widgets.active.corner_radius = CornerRadius::same(14);
    visuals
}

pub fn fretboard_fill() -> Color32 {
    Color32::from_rgb(42, 31, 24)
}
