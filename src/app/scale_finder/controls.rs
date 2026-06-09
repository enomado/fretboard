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
    pub(super) fn draw_scale_finder_controls(&mut self, ui: &mut Ui, frame_ms: u64) {
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
            ui.add_sized(
                [200.0, 18.0],
                egui::Slider::new(&mut config.window_frames, 1..=120)
                    .clamping(egui::SliderClamping::Always)
                    .trailing_fill(true)
                    .show_value(false),
            );
            // Кадры → секунды по интервалу обновления банка (resonator.update_ms).
            let seconds = config.window_frames as f32 * frame_ms as f32 / 1000.0;
            ui.label(
                RichText::new(format!("{} fr · ~{:.1}s", config.window_frames, seconds))
                    .color(Color32::from_rgb(226, 216, 201))
                    .monospace(),
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
