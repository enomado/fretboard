use eframe::egui::{
    self,
    Color32,
    Frame,
    Margin,
    Stroke,
    Ui,
};

use super::{
    App,
    WorkspaceTab,
};
use crate::ui::theme::PANEL_FILL;

pub(super) struct WorkspaceBehavior<'a> {
    app: &'a mut App,
}

impl egui_tiles::Behavior<WorkspaceTab> for WorkspaceBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut WorkspaceTab,
    ) -> egui_tiles::UiResponse {
        let pane_rect = ui.max_rect();
        ui.painter().rect_filled(pane_rect, 0.0, PANEL_FILL);

        egui::ScrollArea::both()
            .id_salt(("workspace_pane_scroll", tile_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_size(pane_rect.size());

                match pane {
                    WorkspaceTab::Controls => self.app.draw_controls(ui),
                    WorkspaceTab::LiveAnalysis => self.app.draw_tuner_card(ui),
                    WorkspaceTab::Resonators => self.app.draw_resonator_card(ui),
                    WorkspaceTab::Waterfall => self.app.draw_resonator_waterfall_card(ui),
                    WorkspaceTab::Fretboard => self.app.draw_fretboard_card(ui),
                }
            });

        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &WorkspaceTab) -> egui::WidgetText {
        pane.label().into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        false
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..egui_tiles::SimplificationOptions::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        Color32::from_rgb(18, 22, 27)
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            Color32::from_rgb(112, 86, 72)
        } else {
            Color32::from_rgb(34, 38, 44)
        }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Stroke {
        Stroke::new(
            1.0_f32,
            if state.active {
                Color32::from_rgb(207, 187, 166)
            } else {
                Color32::from_rgb(76, 82, 90)
            },
        )
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(1.0_f32, Color32::from_rgb(56, 61, 69))
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            Color32::from_rgb(235, 227, 216)
        } else {
            Color32::from_rgb(188, 192, 198)
        }
    }
}

impl App {
    pub(super) fn render(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(8)))
            .show_inside(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));

                let mut workspace_tree = self.workspace_tree.take().unwrap_or_else(default_workspace_tree);
                workspace_tree.ui(&mut WorkspaceBehavior { app: self }, ui);
                self.workspace_tree = Some(workspace_tree);
            });
    }
}

pub(super) fn default_workspace_tree() -> egui_tiles::Tree<WorkspaceTab> {
    egui_tiles::Tree::new_tabs(
        "fretboard_workspace_tree",
        vec![
            WorkspaceTab::Controls,
            WorkspaceTab::Fretboard,
            WorkspaceTab::LiveAnalysis,
            WorkspaceTab::Resonators,
            WorkspaceTab::Waterfall,
        ],
    )
}
