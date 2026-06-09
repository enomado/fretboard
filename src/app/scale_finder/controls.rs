//! Слайдеры панели Scale Finder: баланс четырёх методов (notes/tonal/root/spiral)
//! и ширина окна интеграции. Конфиг живёт прямо в `App.scale_finder` — панель
//! самодостаточна.

use eframe::egui::{
    self,
    Color32,
    RichText,
    Ui,
};

use super::super::App;
use super::{
    COLOR_PROFILE,
    COLOR_ROOT,
    COLOR_SET,
    COLOR_SPIRAL,
};
use crate::core_types::scale_detect::MethodWeights;

impl App {
    /// Слайдеры прямо в панели (конфиг живёт с панелью — панель самодостаточна):
    /// баланс четырёх методов + ширина окна интеграции. `blended()` нормирует на
    /// сумму весов, поэтому каждый вес — относительный вклад, сумма к 1 не обязана.
    pub(super) fn draw_scale_finder_controls(&mut self, ui: &mut Ui, captured_secs: f32) {
        let config = &mut self.scale_finder;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Method mix")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            weight_slider(ui, "notes", COLOR_SET, &mut config.weights.set);
            weight_slider(ui, "tonal", COLOR_PROFILE, &mut config.weights.profile);
            weight_slider(ui, "root", COLOR_ROOT, &mut config.weights.root);
            weight_slider(ui, "spiral", COLOR_SPIRAL, &mut config.weights.spiral);
            if ui.button("Reset").clicked() {
                config.weights = MethodWeights::default();
            }
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Window")
                    .color(Color32::from_rgb(205, 194, 176))
                    .strong(),
            );
            // Окно интеграции В СЕКУНДАХ (решалка копит свой буфер по времени).
            // Логарифмический, чтобы и доли секунды, и 20 c были под рукой.
            ui.add_sized(
                [200.0, 18.0],
                egui::Slider::new(&mut config.window_seconds, 0.3..=20.0)
                    .logarithmic(true)
                    .clamping(egui::SliderClamping::Always)
                    .trailing_fill(true)
                    .show_value(false),
            );
            ui.label(
                RichText::new(format!("{:.1}s", config.window_seconds))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
            );
            // Сколько реально накоплено: окно наполняется с открытия панели.
            ui.label(
                RichText::new(format!("· have {captured_secs:.1}s"))
                    .color(Color32::from_rgb(150, 156, 164))
                    .size(12.0),
            );
        });
    }
}

/// Один помеченный слайдер веса метода (0..1) с цветной меткой и числом.
fn weight_slider(ui: &mut Ui, label: &str, color: Color32, value: &mut f32) {
    ui.label(RichText::new(label).color(color).strong().size(12.0));
    ui.add_sized(
        [110.0, 18.0],
        egui::Slider::new(value, 0.0..=1.0)
            .clamping(egui::SliderClamping::Always)
            .trailing_fill(true)
            .show_value(false),
    );
    ui.label(
        RichText::new(format!("{value:.2}"))
            .color(Color32::from_rgb(226, 216, 201))
            .monospace(),
    );
}
