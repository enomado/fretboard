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
use super::ranking::{
    COLOR_PROFILE,
    COLOR_ROOT,
    COLOR_SET,
    COLOR_SPIRAL,
};
use crate::core_types::scale_detect::ensemble::MethodWeights;
use crate::ui::segmented::RowCaption;
use crate::ui::slider_row::SliderRow;
use crate::ui::theme;
use crate::ui::tokens::color;

impl App {
    /// Слайдеры прямо в панели (конфиг живёт с панелью — панель самодостаточна):
    /// баланс четырёх методов + ширина окна интеграции. `blended()` нормирует на
    /// сумму весов, поэтому каждый вес — относительный вклад, сумма к 1 не обязана.
    pub(super) fn draw_scale_finder_controls(&mut self, ui: &mut Ui, captured_secs: f32) {
        let config = &mut self.scale_finder;
        ui.horizontal_wrapped(|ui| {
            ui.add(RowCaption::new("Method mix"));
            weight_slider(ui, "notes", COLOR_SET, &mut config.weights.set);
            weight_slider(ui, "tonal", COLOR_PROFILE, &mut config.weights.profile);
            weight_slider(ui, "root", COLOR_ROOT, &mut config.weights.root);
            weight_slider(ui, "spiral", COLOR_SPIRAL, &mut config.weights.spiral);
            if theme::finger_sized(ui, |ui| ui.button("Reset")).clicked() {
                config.weights = MethodWeights::default();
            }
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            // Окно интеграции В СЕКУНДАХ (решалка копит свой буфер по времени).
            // Логарифмический, чтобы и доли секунды, и 20 c были под рукой.
            let window = SliderRow::new(
                RowCaption::new("Window"),
                &mut config.window_seconds,
                0.3..=20.0,
                |v| format!("{v:.1}s"),
            )
            .width(200.0)
            .logarithmic()
            .clamp_always()
            .trailing_fill();
            ui.add(window);
            // Сколько реально накоплено: окно наполняется с открытия панели.
            // Неразрывно: перенос посреди («· have | 0.0s») клал вторую строку
            // надписи под ряд окна, поверх его слайдера.
            ui.add(
                egui::Label::new(
                    RichText::new(format!("· have {captured_secs:.1}s"))
                        .color(color::TEXT_HINT)
                        .size(12.0),
                )
                .extend(),
            );
        });
    }
}

/// Один помеченный слайдер веса метода (0..1) с цветной меткой и числом.
fn weight_slider(ui: &mut Ui, label: &str, color: Color32, value: &mut f32) {
    let caption = RowCaption::new(label).color(color).font_size(12.0);
    ui.add(
        SliderRow::new(caption, value, 0.0..=1.0, |v| format!("{v:.2}"))
            .width(110.0)
            .clamp_always()
            .trailing_fill(),
    );
}
