use eframe::egui::{
    self,
    Color32,
    Frame,
    Margin,
    Stroke,
    Ui,
    vec2,
};

use super::{
    App,
    WorkspaceTab,
};
use crate::ui::tokens::color;

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
        let pane_padding = 8;
        let pane_padding_f = f32::from(pane_padding);
        let pane_rect = ui.max_rect();
        ui.painter().rect_filled(pane_rect, 0.0, color::PANEL_FILL);

        egui::ScrollArea::both()
            .id_salt(("workspace_pane_scroll", tile_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::same(pane_padding))
                    .show(ui, |ui| {
                        let min_size = pane_rect.size() - vec2(pane_padding_f * 2.0, pane_padding_f * 2.0);
                        ui.set_min_size(vec2(min_size.x.max(0.0), min_size.y.max(0.0)));

                        self.app.draw_workspace_tab(ui, *pane);
                    });
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
        // Каждая вкладка несёт «крестик». Закрытие = `tiles.remove(id)`
        // (так же делает встроенная кнопка egui_tiles); снова открыть —
        // через меню «Panels» в верхней панели. См. `open_workspace_tab`.
        true
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..egui_tiles::SimplificationOptions::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        color::TAB_BAR_BG
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        // Активный таб носит тот же акцент, что и выбранная пилюля (`ui::segmented`),
        // а вот idle у табов свой — темнее пилюльного, чтобы полоса табов читалась
        // как фон, а не как ряд кнопок.
        if state.active {
            color::ACCENT_FILL
        } else {
            color::TAB_IDLE_FILL
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
                color::ACCENT_STROKE
            } else {
                color::TAB_IDLE_STROKE
            },
        )
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(1.0_f32, color::TAB_HLINE)
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<WorkspaceTab>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            color::TAB_TEXT_ACTIVE
        } else {
            color::TAB_TEXT_IDLE
        }
    }
}

impl App {
    /// Единый диспетчер «панель → её отрисовка». Один источник истины для набора
    /// панелей: им пользуется и десктопный воркспейс (в `pane_ui`), и мобильный
    /// путь (одна панель на весь экран). Добавил панель в `WorkspaceTab` —
    /// добавь ветку здесь, и она сразу доступна на обеих платформах.
    pub(super) fn draw_workspace_tab(&mut self, ui: &mut Ui, tab: WorkspaceTab) {
        match tab {
            WorkspaceTab::Controls => self.draw_controls(ui),
            WorkspaceTab::FretboardControls => self.draw_fretboard_controls(ui),
            WorkspaceTab::InputScope => self.draw_input_scope_card(ui),
            WorkspaceTab::ConfigGeneral => self.draw_general_config_card(ui),
            WorkspaceTab::ConfigFft1 => self.draw_fft1_config_card(ui),
            WorkspaceTab::ConfigResonatorFft => self.draw_resonator_fft_config_card(ui),
            WorkspaceTab::LiveAnalysis => self.draw_tuner_card(ui),
            WorkspaceTab::ScaleFinder => self.draw_scale_finder_card(ui),
            WorkspaceTab::ResonatorBank => self.draw_resonator_bank_card(ui),
            WorkspaceTab::ResonatorSnail => self.draw_resonator_snail_card(ui),
            WorkspaceTab::ResonatorWaterfall => self.draw_resonator_waterfall_card(ui),
            WorkspaceTab::Fretboard => self.draw_fretboard_card(ui),
            WorkspaceTab::Drone => self.draw_drone_card(ui),
            WorkspaceTab::Staff => self.draw_staff_card(ui),
            WorkspaceTab::PitchRoll => self.draw_pitch_roll_card(ui),
        }
    }

    /// Мобильная лента вкладок — тот самый «конфиг какую открывать», но все панели
    /// видны сразу (а не спрятаны в дропдаун): тап переключает активную, выбор
    /// пишется в `mobile_panel` и переживает перезапуск (персист в RON).
    ///
    /// Перенос на новые строки — `horizontal_wrapped`. Это безопасно, потому что
    /// каждая вкладка — ОДИНОЧНЫЙ `selectable_label` (не вложенный фиксированный
    /// `ui.horizontal`), а удвоение ширины ловится именно на «wrapped + вложенный
    /// не-переносящий horizontal» (см. GOTCHA_egui_layout_width_doubling).
    #[cfg(target_os = "android")]
    fn draw_mobile_panel_tabs(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in WorkspaceTab::ALL {
                ui.selectable_value(&mut self.mobile_panel, tab, tab.label());
            }
        });
        ui.add_space(6.0);
    }

    #[cfg(target_os = "android")]
    pub(super) fn render(&mut self, ui: &mut Ui) {
        // Frame heartbeat: if these numbers keep climbing in logcat the eframe
        // loop is alive (not frozen). Logged every 30th frame to stay readable.
        {
            use std::sync::atomic::{
                AtomicU64,
                Ordering,
            };
            static FRAME: AtomicU64 = AtomicU64::new(0);
            let n = FRAME.fetch_add(1, Ordering::Relaxed);
            if n % 30 == 0 {
                crate::android_perm::alog(&format!("frame {n}"));
            }
        }

        // Mic permission is driven from here (the first real frame ⇒ the Activity
        // is resumed and can show the dialog). `request_record_audio` fires once;
        // on the rising edge of the grant we re-open capture, since the audio
        // engine's startup open happened before the user tapped "Allow".
        crate::android_perm::request_record_audio();
        if crate::android_perm::newly_granted() {
            self.audio.set_selected_input_id(None);
        }

        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(8)))
            .show(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));

                // «Конфиг какую панель открывать»: лента вкладок сверху (все панели
                // видны сразу), активная панель под ней. Одна панель за раз — на
                // узком экране это максимум места под контент.
                self.draw_mobile_panel_tabs(ui);

                match self.mobile_panel {
                    // У снейла свой мобильный вариант: тянется на всю оставшуюся
                    // высоту + встроенная полоса настроек, и НЕ в ScrollArea —
                    // иначе (unbounded height) он бы раздувался без предела.
                    WorkspaceTab::ResonatorSnail => self.draw_mobile_snail_card(ui),
                    // Остальные панели — конечной высоты; вертикальный скролл на
                    // случай, если контент не влезает в экран телефона.
                    tab => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.draw_workspace_tab(ui, tab));
                    }
                }
            });
    }

    #[cfg(not(target_os = "android"))]
    pub(super) fn render(&mut self, ui: &mut Ui) {
        // Верхняя полоса-меню: реестр всех панелей (открыть/закрыть/сфокусировать)
        // + сброс раскладки. Закрывать можно и «крестиком» на самой вкладке.
        egui::Panel::top("workspace_menu_bar")
            .frame(
                Frame::new()
                    .fill(color::TAB_BAR_BG)
                    .inner_margin(Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                let tree = self.workspace_tree.get_or_insert_with(default_workspace_tree);
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("Panels", |ui| {
                        for tab in WorkspaceTab::ALL {
                            // Галочка = панель сейчас в дереве. Клик переключает:
                            // снять → закрыть, поставить → открыть (или сфокусировать).
                            let mut open = tree.tiles.find_pane(&tab).is_some();
                            if ui.checkbox(&mut open, tab.label()).changed() {
                                if open {
                                    open_workspace_tab(tree, tab);
                                } else {
                                    close_workspace_tab(tree, tab);
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("Reset layout").clicked() {
                            *tree = default_workspace_tree();
                            ui.close();
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(8)))
            .show(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));

                let mut workspace_tree = self.workspace_tree.take().unwrap_or_else(default_workspace_tree);
                workspace_tree.ui(&mut WorkspaceBehavior { app: self }, ui);
                self.workspace_tree = Some(workspace_tree);
            });
    }
}

/// Открыть панель `tab`. Если она уже в дереве — просто делаем её активной
/// (фокус), не создавая дубликат. Иначе вставляем новую вкладку в корневой
/// контейнер; на пустом/вырожденном дереве заворачиваем в свежий tabs-контейнер.
fn open_workspace_tab(tree: &mut egui_tiles::Tree<WorkspaceTab>, tab: WorkspaceTab) {
    if let Some(existing) = tree.tiles.find_pane(&tab) {
        tree.make_active(|id, _| id == existing);
        return;
    }

    let new_id = tree.tiles.insert_pane(tab);
    match tree.root {
        // Корень — контейнер: дописываем вкладку в конец (индекс клампится).
        Some(root) if matches!(tree.tiles.get(root), Some(egui_tiles::Tile::Container(_))) => {
            tree.move_tile_to_container(new_id, root, usize::MAX, false);
            tree.make_active(|id, _| id == new_id);
        }
        // Пустое дерево или «голая» панель в корне: собираем tabs-контейнер.
        _ => {
            let children = match tree.root {
                Some(root) => vec![root, new_id],
                None => vec![new_id],
            };
            tree.root = Some(tree.tiles.insert_tab_tile(children));
        }
    }
}

/// Закрыть панель `tab`: убираем её tile из дерева ровно как встроенный
/// «крестик» egui_tiles (`tiles.remove`); висячую ссылку в контейнере чистит
/// gc на следующем `ui()`.
fn close_workspace_tab(tree: &mut egui_tiles::Tree<WorkspaceTab>, tab: WorkspaceTab) {
    if let Some(id) = tree.tiles.find_pane(&tab) {
        tree.tiles.remove(id);
    }
}

pub(super) fn default_workspace_tree() -> egui_tiles::Tree<WorkspaceTab> {
    // Дефолт = все панели в одной полосе вкладок. Реестр панелей — `WorkspaceTab::ALL`.
    egui_tiles::Tree::new_tabs("fretboard_workspace_tree", WorkspaceTab::ALL.to_vec())
}
