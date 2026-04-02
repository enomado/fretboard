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

pub const PANEL_FILL: Color32 = Color32::from_rgb(20, 26, 34);

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
    visuals.override_text_color = Some(Color32::from_rgb(229, 221, 205));
    visuals.panel_fill = Color32::from_rgb(12, 17, 24);
    visuals.window_fill = PANEL_FILL;
    visuals.extreme_bg_color = Color32::from_rgb(14, 18, 24);
    visuals.code_bg_color = Color32::from_rgb(28, 35, 44);
    visuals.selection.bg_fill = Color32::from_rgb(173, 75, 54);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(255, 218, 188));
    visuals.widgets.noninteractive.bg_fill = PANEL_FILL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(54, 63, 79));
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(33, 41, 52);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(78, 90, 105));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(14);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 61, 76);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(131, 150, 171));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(242, 236, 224));
    visuals.widgets.hovered.corner_radius = CornerRadius::same(14);
    visuals.widgets.active.bg_fill = Color32::from_rgb(173, 75, 54);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(255, 217, 177));
    visuals.widgets.active.corner_radius = CornerRadius::same(14);
    visuals
}

pub fn fretboard_fill() -> Color32 {
    Color32::from_rgb(47, 31, 19)
}
