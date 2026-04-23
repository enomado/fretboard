use eframe::egui::{
    self,
    Color32,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Stroke,
    Ui,
    vec2,
};

use super::{
    App,
    WorkspaceTab,
    pill,
};

impl App {
    pub(super) fn render(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(16, 20, 25))
                    .inner_margin(Margin::same(18)),
            )
            .show_inside(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));
                self.draw_header(ui);
                ui.add_space(14.0);
                self.draw_tab_bar(ui);
                ui.add_space(12.0);

                match self.active_tab {
                    WorkspaceTab::Controls => self.draw_controls(ui),
                    WorkspaceTab::LiveAnalysis => self.draw_tuner_card(ui),
                    WorkspaceTab::Resonators => self.draw_resonator_card(ui),
                    WorkspaceTab::Waterfall => self.draw_resonator_waterfall_card(ui),
                    WorkspaceTab::Fretboard => self.draw_fretboard_card(ui),
                }
            });
    }

    fn draw_header(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Fretboard Explorer")
                        .size(28.0)
                        .color(Color32::from_rgb(230, 223, 210))
                        .family(egui::FontFamily::Proportional),
                );
                ui.label(
                    RichText::new(format!(
                        "{} • {} • root {}",
                        self.tuning_kind.subtitle(),
                        self.scale_kind.label(),
                        self.root_label()
                    ))
                    .color(Color32::from_rgb(154, 160, 168)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(
                    ui,
                    "Muted",
                    Color32::from_rgb(152, 159, 168),
                    Color32::from_rgb(61, 67, 75),
                );
                pill(
                    ui,
                    "5th",
                    Color32::from_rgb(203, 182, 147),
                    Color32::from_rgb(72, 58, 47),
                );
                pill(
                    ui,
                    "Root",
                    Color32::from_rgb(214, 190, 168),
                    Color32::from_rgb(89, 64, 56),
                );
            });
        });
    }

    fn draw_tab_bar(&mut self, ui: &mut Ui) {
        Frame::new()
            .fill(Color32::from_rgb(22, 26, 31))
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(54, 59, 67)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for tab in WorkspaceTab::ALL {
                        let selected = self.active_tab == tab;
                        let button = egui::Button::new(tab.label())
                            .min_size(vec2(118.0, 32.0))
                            .fill(if selected {
                                Color32::from_rgb(112, 86, 72)
                            } else {
                                Color32::from_rgb(38, 43, 49)
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if selected {
                                    Color32::from_rgb(207, 187, 166)
                                } else {
                                    Color32::from_rgb(80, 86, 94)
                                },
                            ))
                            .corner_radius(CornerRadius::same(14));

                        if ui.add(button).clicked() {
                            self.active_tab = tab;
                        }
                    }
                });
            });
    }
}
