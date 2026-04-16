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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceTabState {
    Hidden,
    Open,
    Active,
}

pub(super) struct WorkspaceBehavior<'a> {
    app: &'a mut App,
}

impl egui_tiles::Behavior<WorkspaceTab> for WorkspaceBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut WorkspaceTab,
    ) -> egui_tiles::UiResponse {
        match pane {
            WorkspaceTab::Controls => self.app.draw_controls(ui),
            WorkspaceTab::LiveAnalysis => self.app.draw_tuner_card(ui),
            WorkspaceTab::Fretboard => self.app.draw_fretboard_card(ui),
        }

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
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<WorkspaceTab>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        tiles.set_visible(tile_id, false);
        false
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..egui_tiles::SimplificationOptions::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        Color32::from_rgb(22, 26, 31)
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            Color32::from_rgb(30, 35, 41)
        } else {
            Color32::from_rgb(24, 28, 33)
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
            1.0,
            if state.active {
                Color32::from_rgb(207, 187, 166)
            } else {
                Color32::from_rgb(70, 76, 84)
            },
        )
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(1.0, Color32::from_rgb(56, 61, 69))
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            Color32::from_rgb(233, 225, 214)
        } else {
            Color32::from_rgb(154, 160, 168)
        }
    }
}

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
                let mut workspace_tree = self.workspace_tree.take().unwrap_or_else(default_workspace_tree);
                self.draw_workspace_toolbar(ui, &mut workspace_tree);
                ui.add_space(12.0);

                if visible_workspace_tabs(&workspace_tree) == 0 {
                    Frame::new()
                        .fill(Color32::from_rgb(22, 26, 31))
                        .corner_radius(CornerRadius::same(18))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(54, 59, 67)))
                        .inner_margin(Margin::same(24))
                        .show(ui, |ui| {
                            ui.set_min_height(260.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(52.0);
                                ui.label(
                                    RichText::new("No tabs open")
                                        .size(22.0)
                                        .color(Color32::from_rgb(226, 216, 201)),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(
                                        "Use the buttons above to add Controls, Live analysis or Fretboard",
                                    )
                                    .color(Color32::from_rgb(145, 151, 160)),
                                );
                            });
                        });
                } else {
                    workspace_tree.ui(&mut WorkspaceBehavior { app: self }, ui);
                }

                self.workspace_tree = Some(workspace_tree);
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

    fn draw_workspace_toolbar(&mut self, ui: &mut Ui, workspace_tree: &mut egui_tiles::Tree<WorkspaceTab>) {
        let mut reset_workspace = false;
        let active_tiles = workspace_tree.active_tiles();

        Frame::new()
            .fill(Color32::from_rgb(22, 26, 31))
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0, Color32::from_rgb(54, 59, 67)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Tabs")
                            .color(Color32::from_rgb(226, 216, 201))
                            .strong(),
                    );

                    for tab in WorkspaceTab::ALL {
                        let state = workspace_tab_state(workspace_tree, &active_tiles, tab);
                        let (fill, stroke) = match state {
                            WorkspaceTabState::Active => {
                                (Color32::from_rgb(112, 86, 72), Color32::from_rgb(207, 187, 166))
                            }
                            WorkspaceTabState::Open => {
                                (Color32::from_rgb(62, 67, 74), Color32::from_rgb(124, 131, 141))
                            }
                            WorkspaceTabState::Hidden => {
                                (Color32::from_rgb(38, 43, 49), Color32::from_rgb(80, 86, 94))
                            }
                        };
                        let button = egui::Button::new(tab.label())
                            .min_size(vec2(102.0, 28.0))
                            .fill(fill)
                            .stroke(Stroke::new(1.0, stroke))
                            .corner_radius(CornerRadius::same(14));

                        if ui.add(button).clicked() {
                            open_or_focus_tab(workspace_tree, tab);
                        }
                    }

                    ui.separator();
                    ui.label(
                        RichText::new("Active tab is highlighted, open tabs are muted, hidden tabs are dark")
                            .color(Color32::from_rgb(145, 151, 160))
                            .size(12.0),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset workspace").clicked() {
                            reset_workspace = true;
                        }
                    });
                });
            });

        if reset_workspace {
            *workspace_tree = default_workspace_tree();
        }
    }
}

pub(super) fn default_workspace_tree() -> egui_tiles::Tree<WorkspaceTab> {
    let mut tiles = egui_tiles::Tiles::default();
    let fretboard = tiles.insert_pane(WorkspaceTab::Fretboard);
    let live_analysis = tiles.insert_pane(WorkspaceTab::LiveAnalysis);
    let controls = tiles.insert_pane(WorkspaceTab::Controls);

    let fretboard_tabs = tiles.insert_tab_tile(vec![fretboard]);
    let live_analysis_tabs = tiles.insert_tab_tile(vec![live_analysis]);
    let controls_tabs = tiles.insert_tab_tile(vec![controls]);
    let sidebar = tiles.insert_new(egui_tiles::Tile::Container(egui_tiles::Container::new_vertical(
        vec![live_analysis_tabs, controls_tabs],
    )));
    let root = tiles.insert_new(egui_tiles::Tile::Container(
        egui_tiles::Container::new_horizontal(vec![fretboard_tabs, sidebar]),
    ));

    egui_tiles::Tree::new("fretboard_workspace_tree", root, tiles)
}

fn visible_workspace_tabs(tree: &egui_tiles::Tree<WorkspaceTab>) -> usize {
    WorkspaceTab::ALL
        .into_iter()
        .filter(|tab| workspace_tile_id(tree, *tab).is_some_and(|tile_id| tree.is_visible(tile_id)))
        .count()
}

fn workspace_tab_state(
    tree: &egui_tiles::Tree<WorkspaceTab>,
    active_tiles: &[egui_tiles::TileId],
    tab: WorkspaceTab,
) -> WorkspaceTabState {
    let Some(tile_id) = workspace_tile_id(tree, tab) else {
        return WorkspaceTabState::Hidden;
    };

    if !tree.is_visible(tile_id) {
        WorkspaceTabState::Hidden
    } else if active_tiles.contains(&tile_id) {
        WorkspaceTabState::Active
    } else {
        WorkspaceTabState::Open
    }
}

fn workspace_tile_id(tree: &egui_tiles::Tree<WorkspaceTab>, tab: WorkspaceTab) -> Option<egui_tiles::TileId> {
    tree.tiles.find_pane(&tab)
}

fn open_or_focus_tab(tree: &mut egui_tiles::Tree<WorkspaceTab>, tab: WorkspaceTab) {
    let Some(tile_id) = workspace_tile_id(tree, tab) else {
        return;
    };

    tree.set_visible(tile_id, true);
    let _ = tree.make_active(|candidate, _| candidate == tile_id);
}
