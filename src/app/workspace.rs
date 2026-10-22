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
            WorkspaceTab::TakeRecorder => self.draw_take_recorder_card(ui),
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

    /// Мобильный выбор панели — тот самый «конфиг какую открывать»: выбор пишется
    /// в `mobile_panel` и переживает перезапуск (персист в RON).
    ///
    /// В покое это ОДНА строка «текущая панель ▼». Раньше все 16 панелей стояли
    /// лентой `horizontal_wrapped` постоянно и занимали 6–8 строк: ≈460 dp из 740 в
    /// портрете 360 dp и ≈230 из 360 в ландшафте — снейл, панель по умолчанию,
    /// уезжал под нижний край. Тап по строке раскрывает ВСЕ панели сразу (не
    /// дропдаун со скроллом: каждая видна и доступна одним тапом), выбор
    /// сворачивает список обратно. Раскрыт ли список — состояние жеста, а не
    /// настройка, поэтому живёт во временной памяти egui и не персистится.
    ///
    /// Плитки — сетка строками по `columns` штук (каждая строка — обычный
    /// `horizontal` внутри вертикального layout), а не `horizontal_wrapped`
    /// с вложенными рядами: см. GOTCHA_egui_layout_width_doubling.
    #[cfg(any(target_os = "android", test))]
    fn draw_mobile_panel_picker(&mut self, ui: &mut Ui) {
        use crate::ui::segmented::SegmentedButton;
        use crate::ui::tokens::space;

        // Один выбор панели на приложение — глобальный id, без привязки к месту в дереве.
        let open_id = egui::Id::unique("mobile_panel_picker_open");
        let open = ui.data_mut(|data| *data.get_temp_mut_or_default::<bool>(open_id));
        let arrow = if open { "▲" } else { "▼" };
        let width = ui.available_width();
        let header = format!("{}  {arrow}", self.mobile_panel.label());
        let mut toggle = ui
            .add(
                SegmentedButton::new(&header, open)
                    .min_width(width)
                    .height(space::TOUCH_H),
            )
            .clicked();

        if open {
            // Плитки в колонку шириной не меньше MIN_TILE_W (на 360-dp экране — две),
            // а ряд переносится: самая длинная подпись («Config Resonator FFT»,
            // ≈172 dp пилюлей) шире половины узкого экрана и уходит на свою строку,
            // а не за правый край. Каждая плитка — одиночный виджет, так что
            // wrapped-ряд не раздувает ширину (GOTCHA_egui_layout_width_doubling).
            const MIN_TILE_W: f32 = 150.0;
            let gap = ui.spacing().item_spacing.x;
            let columns = (((width + gap) / (MIN_TILE_W + gap)).floor()).clamp(2.0, 4.0);
            let tile_w = (width - gap * (columns - 1.0)) / columns;
            ui.horizontal_wrapped(|ui| {
                for tab in WorkspaceTab::ALL {
                    let tile = SegmentedButton::new(tab.label(), tab == self.mobile_panel)
                        .min_width(tile_w)
                        .height(space::TOUCH_H);
                    if ui.add(tile).clicked() {
                        self.mobile_panel = tab;
                        toggle = true;
                    }
                }
            });
        }
        if toggle {
            ui.data_mut(|data| data.insert_temp(open_id, !open));
        }
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

        self.render_mobile_shell(ui);
    }

    /// Мобильная оболочка целиком — всё, что Android рисует после разрешения на
    /// микрофон. Отдельно от `render`, чтобы хостовые kittest-тесты гоняли ровно эту
    /// вёрстку на ширине телефона (JNI-часть `render` на хосте не собирается).
    #[cfg(any(target_os = "android", test))]
    pub(super) fn render_mobile_shell(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::same(8)))
            .show(ui, |ui| {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(33));

                // «Конфиг какую панель открывать»: одна строка выбора сверху,
                // активная панель под ней. Одна панель за раз — на узком экране
                // это максимум места под контент.
                self.draw_mobile_panel_picker(ui);

                // Панель-пара «экран + его конфиг» — стандартная раскладка телефона
                // (`mobile_split`): экран сверху, конфиг снизу; скроллит каждая часть сама.
                let tab = self.mobile_panel;
                if tab.splits_on_phone() {
                    self.draw_mobile_split(ui, tab);
                    return;
                }

                // Скролл в обе стороны, как у пейна на десктопе: панели рассчитаны
                // на десктопную ширину, и их ряды без переноса (поле имени дубля +
                // Record, клавиатура дрона, тональность нотного стана) шире 360 dp.
                // С одним вертикальным скроллом правый край молча обрезался —
                // кнопка за ним была недостижима. Внутренний max_rect ScrollArea
                // равен видимой области, поэтому «тянуться на оставшуюся высоту»
                // по-прежнему значит «до нижнего края экрана».
                egui::ScrollArea::both()
                    .id_salt(("mobile_panel_scroll", tab.label()))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.draw_workspace_tab(ui, tab));
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

                // The tree is taken only while rendering and restored before returning.
                // App::new and persistence both supply it; absence is a broken invariant.
                let mut workspace_tree = self
                    .workspace_tree
                    .take()
                    .expect("workspace tree exists before render");
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
    egui_tiles::Tree::new_tabs(
        egui::Id::unique("fretboard_workspace_tree"),
        WorkspaceTab::ALL.to_vec(),
    )
}

/// The phone shell, rendered by `egui_kittest` at phone sizes (logical px = dp).
///
/// Runs on the host: the JNI half of the Android `render` does not build here, but
/// [`App::render_mobile_shell`] — everything the phone draws — does. The engine has
/// no audio device ([`App::for_ui_test`]), so panels show their empty states, which
/// is also what a phone shows before the first frame of audio.
///
/// The harness also fails on any glyph no installed font can draw (kittest's
/// missing-glyph policy) and on an interactive widget with no accessible name, so
/// walking every panel doubles as a tofu and TalkBack check.
#[cfg(test)]
mod tests {
    use eframe::egui::os::OperatingSystem;
    use eframe::egui::{
        Rect,
        Role,
        Vec2,
        pos2,
        vec2,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::{
        NodeT,
        Queryable,
    };

    use super::super::{
        App,
        WorkspaceTab,
    };
    use crate::audio::types::MelodyFrame;
    use crate::core_types::pitch::Midi;
    use crate::ui::tokens::space;

    /// The narrowest common Android portrait and a large one, each also turned landscape.
    const PHONES: [Vec2; 4] = [
        vec2(360.0, 740.0),
        vec2(412.0, 915.0),
        vec2(740.0, 360.0),
        vec2(915.0, 412.0),
    ];

    fn phone(size: Vec2, tab: WorkspaceTab) -> Harness<'static, App> {
        let mut app = App::for_ui_test();
        app.mobile_panel = tab;
        phone_with(size, app)
    }

    /// The phone shell around an `app` the test has already set up.
    fn phone_with(size: Vec2, app: App) -> Harness<'static, App> {
        let mut harness = Harness::builder()
            .with_size(size)
            // A phone: the panels shared with the desktop size their targets and name
            // their gestures for a finger (`theme::is_touch_screen`).
            .with_os(OperatingSystem::Android)
            .with_context_setup(|ctx| {
                crate::ui::theme::install_fonts(ctx);
                crate::ui::theme::apply_theme(ctx);
            })
            .build_ui_state(|ui, app: &mut App| app.render_mobile_shell(ui), app);
        // Live panels keep asking for repaints, so `run()` would never settle; a few
        // frames let every lazily-sized layout reach its steady state.
        harness.run_steps(4);
        harness
    }

    /// Every laid-out node with its rect (nodes without bounds have none to check).
    fn rects(harness: &Harness<'_, App>) -> Vec<(Role, String, Rect)> {
        harness
            .root()
            .children_recursive()
            .filter(|node| node.accesskit_node().bounding_box().is_some())
            .map(|node| {
                let ak = node.accesskit_node();
                // A plain label's text is its value, not its name.
                let text = ak.label().or_else(|| ak.value());
                (ak.role(), text.unwrap_or_default(), node.rect())
            })
            .collect()
    }

    /// REGRESSION: nothing is laid out past the right edge of a phone screen.
    ///
    /// The phone's panels are the desktop's, built for a wide pane; rows that never
    /// wrap (take name + Record, the drone keyboard's twelve keys, clef pills + key
    /// combo, the monitor row, the 340 dp device combo) ran past 360 dp, and the
    /// part past the edge was simply cut — Record and five keys per octave could
    /// not be reached. Vertical overflow is fine (the pane scrolls); a node right of
    /// the screen is a control the user has to find by panning sideways, if at all.
    #[test]
    fn every_panel_stays_inside_a_phone_screen() {
        for size in PHONES {
            for tab in WorkspaceTab::ALL {
                let harness = phone(size, tab);
                // Left too: a right-to-left row that does not fit spills leftwards,
                // over its caption and out of the card (the Take Roll switches did).
                let past: Vec<_> = rects(&harness)
                    .into_iter()
                    .filter(|(_, _, rect)| rect.right() > size.x + 0.5 || rect.left() < -0.5)
                    .collect();
                assert!(
                    past.is_empty(),
                    "{} at {size:?}: laid out past a screen edge: {past:?}",
                    tab.label()
                );
            }
        }
    }

    /// REGRESSION: no text is laid out over another text or a control on a phone.
    ///
    /// Card headers put the title column first and the status badge right-to-left in what
    /// was left; on 360–412 dp the column took the whole width and the badge ("waiting
    /// for input", "0 samples", the staff's "—") was painted over the subtitle — Input
    /// Scope, Resonator Bank/Waterfall, Scale Finder. Painted plot text has no node; that
    /// is `ui::plot_text`'s to test.
    #[test]
    fn no_text_is_laid_over_another_on_a_phone() {
        let mut failures = Vec::new();
        for size in PHONES {
            for tab in WorkspaceTab::ALL {
                let harness = phone_with(size, app_with_rolls(tab));
                let nodes = rects(&harness);
                let texts = nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, (role, ..))| *role == Role::Label);
                let mut overlaps = Vec::new();
                for (i, (_, text, a)) in texts {
                    // Against every later label, and against every non-label node that is
                    // not a container of the label (a button's caption is its own label).
                    for (j, (role, other, b)) in nodes.iter().enumerate() {
                        let later_label = *role == Role::Label && j > i;
                        let control = matches!(
                            role,
                            Role::Button | Role::TextInput | Role::Slider | Role::CheckBox | Role::ComboBox
                        );
                        let overlap = a.intersect(*b);
                        if (later_label || control) && overlap.width() > 1.0 && overlap.height() > 1.0 {
                            overlaps.push(format!("'{text}' {a:?} × {role:?} '{other}' {b:?}"));
                        }
                    }
                }
                if !overlaps.is_empty() {
                    failures.push(format!("{} at {size:?}: {overlaps:#?}", tab.label()));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// REGRESSION: the panel picker is one touch-sized row until tapped, then shows
    /// every panel at once, each a touch-sized tile on screen, and picking one folds
    /// it back. It used to be all sixteen panels as a permanent wrapped strip of
    /// 31 dp text buttons — 6–8 rows, about half a portrait screen — above the panel.
    #[test]
    fn picker_is_one_row_until_tapped_and_then_one_tap_from_every_panel() {
        let size = PHONES[0];
        let mut harness = phone(size, WorkspaceTab::ResonatorSnail);

        let header = harness.get_by_role_and_label(Role::Button, "Resonator Snail  ▼");
        assert!(header.rect().height() >= space::TOUCH_H, "{:?}", header.rect());
        assert!(
            harness.query_by_label("Drone").is_none(),
            "folded picker shows only the current panel"
        );

        header.click();
        harness.run_steps(2);
        for tab in WorkspaceTab::ALL {
            let tile = harness.get_by_role_and_label(Role::Button, tab.label());
            let rect = tile.rect();
            assert!(
                rect.height() >= space::TOUCH_H && rect.right() <= size.x && rect.bottom() <= size.y,
                "{}: tile {rect:?} is not a touch target on a {size:?} screen",
                tab.label()
            );
        }

        // The panel under the unfolded picker still fits the width (the snail's knob
        // column used to go landscape here, judged by the height the picker left).
        let past: Vec<_> = rects(&harness)
            .into_iter()
            .filter(|(_, _, rect)| rect.right() > size.x + 0.5)
            .collect();
        assert!(
            past.is_empty(),
            "past the right edge with the picker open: {past:?}"
        );

        harness.get_by_role_and_label(Role::Button, "Drone").click();
        harness.run_steps(2);
        assert!(harness.state().mobile_panel == WorkspaceTab::Drone);
        assert!(
            harness.query_by_label("Pitch Roll").is_none(),
            "picking a panel folds the picker"
        );
        harness.get_by_role_and_label(Role::Button, "Drone  ▼");
    }

    /// The fretboard card stands its neck up on a portrait phone (Stas, 2026-10-22:
    /// "гриф на телефоне рисовать вертикально") and keeps it lying in landscape.
    /// Whether its notes then fit is `fretboard_panel`'s geometry test; this pins
    /// that the phone actually gets that layout.
    #[test]
    fn fretboard_stands_up_on_a_portrait_phone() {
        for size in PHONES {
            let harness = phone(size, WorkspaceTab::Fretboard);
            let neck = harness.get_by_role_and_label(Role::Image, "Fretboard").rect();
            // Not just "taller than wide": the lying neck is 340 dp tall, which on a
            // 360 dp phone is taller than its ~314 dp width too. Standing, it is
            // over twice as tall as wide.
            let standing = neck.height() > 2.0 * neck.width();
            let lying = neck.width() > neck.height();
            let portrait = size.y > size.x;
            assert!(
                if portrait { standing } else { lying },
                "at {size:?} the neck {neck:?} lies the wrong way"
            );
        }
    }

    /// A sung phrase for the rolls: a slow G4–D5 scale with rests, one bank frame every
    /// 16 ms. Pitch only — the layout tests below are about where the rolls are, and the
    /// heat's rendering is `ui::pianoroll`'s to test.
    fn phrase(seconds: f64) -> Vec<MelodyFrame> {
        const NOTES: [f32; 8] = [67.0, 69.0, 71.0, 72.0, 74.0, 72.0, 71.0, 69.0];
        (1..=(seconds / 0.016) as u64)
            .map(|seq| {
                let t = seq as f64 * 0.016;
                let rest = t % 0.9 > 0.8;
                MelodyFrame {
                    seq,
                    t,
                    pitch: (!rest).then(|| Midi(NOTES[(t / 0.9) as usize % NOTES.len()])),
                    frontend: Default::default(),
                    raw_pitch: None,
                    reference_pitch: None,
                    level: if rest { 0.0 } else { 0.5 },
                    heat: Vec::new(),
                    salience: None,
                }
            })
            .collect()
    }

    /// The app with both rolls showing something: the live roll fed frame by frame, as
    /// the engine feeds it (its framing eases per update), and a 20 s take open in the
    /// take roll.
    fn app_with_rolls(tab: WorkspaceTab) -> App {
        let mut app = App::for_ui_test();
        app.mobile_panel = tab;
        for frame in phrase(9.0) {
            app.pitch_roll.update(vec![frame]);
        }
        app.take_roll
            .load_for_test("/testdata/phone_take.wav".into(), 20.0, phrase(20.0));
        app
    }

    /// The part of the screen the panel scrolls in: under the picker's row.
    fn pane(harness: &Harness<'_, App>, size: Vec2) -> Rect {
        let header = format!("{}  ▼", harness.state().mobile_panel.label());
        let top = harness
            .get_by_role_and_label(Role::Button, &header)
            .rect()
            .bottom();
        Rect::from_min_max(pos2(0.0, top), pos2(size.x, size.y))
    }

    /// REGRESSION: the live roll is whole on the first screen, portrait and landscape.
    /// It was a fixed 360 px under its header — taller than a landscape phone's whole
    /// pane, so its bottom rows and its time ruler were never on screen with its top.
    #[test]
    fn the_live_roll_is_whole_on_the_first_screen() {
        for size in PHONES {
            let harness = phone_with(size, app_with_rolls(WorkspaceTab::PitchRoll));
            let roll = harness.get_by_role_and_label(Role::Image, "Pitch roll").rect();
            assert!(
                pane(&harness, size).contains_rect(roll),
                "at {size:?} the live roll {roll:?} runs off the first screen"
            );
        }
    }

    /// REGRESSION: a finger can scroll the take recorder until the take roll is whole on
    /// screen, and the pane keeps a finger's strip beside it to scroll on by.
    ///
    /// The roll takes the drag itself (it pans the take), and it was a fixed 320 px — on a
    /// landscape phone taller than the whole ~270 px pane. Scrolled to, it covered every
    /// pixel a finger could land on, and the page could neither reach its bottom nor be
    /// scrolled back off it. Driven with real touches, from wherever the roll is not —
    /// which is exactly what a user has to find.
    #[test]
    fn the_take_roll_scrolls_into_view_whole_and_leaves_room_to_scroll_on() {
        use crate::app::touch_fingers::{
            Finger,
            gesture,
        };

        for size in PHONES {
            let mut harness = phone_with(size, app_with_rolls(WorkspaceTab::TakeRecorder));
            let pane = pane(&harness, size);
            let roll_rect =
                |harness: &Harness<'_, App>| harness.get_by_role_and_label(Role::Image, "Take roll").rect();
            for _ in 0..30 {
                let roll = roll_rect(&harness);
                if pane.contains_rect(roll) {
                    break;
                }
                // The lowest point of the pane the roll does not cover, in the roll's
                // column — where a thumb scrolling down lands. A fingertip's width clear
                // of the roll: egui hit-tests a touch with a radius
                // (`interaction.interact_radius`), so a press just beside the roll is the
                // roll's. Clear of the screen's bottom edge too, past the shell's margin.
                const CLEAR: f32 = 12.0;
                let free = ((pane.top() + CLEAR) as i32..(pane.bottom() - 2.0 * CLEAR) as i32)
                    .rev()
                    .map(|y| y as f32)
                    .find(|&y| y < roll.top() - CLEAR || y > roll.bottom() + CLEAR);
                let Some(y) = free else {
                    panic!("at {size:?} the take roll {roll:?} covers the whole pane {pane:?}");
                };
                let from = pos2(roll.center().x, y);
                gesture(&mut harness, &[(Finger::FIRST, from, from - vec2(0.0, 80.0))], 8);
                harness.run_steps(8);
                assert!(
                    roll_rect(&harness) != roll,
                    "at {size:?} dragging from {from:?} did not scroll the pane"
                );
            }
            let roll = roll_rect(&harness);
            assert!(
                pane.contains_rect(roll),
                "at {size:?} the take roll {roll:?} never came whole into the pane {pane:?}"
            );
            assert!(
                pane.height() - roll.height() >= space::TOUCH_H,
                "at {size:?} the take roll leaves {:.0} dp of a {:.0} dp pane to scroll on",
                pane.height() - roll.height(),
                pane.height()
            );
        }
    }

    /// Every control of the two roll panels is a finger's height on a phone, and the take
    /// roll's hint names the phone's gestures — the pinch, not ctrl/shift+wheel, which a
    /// phone does not have.
    #[test]
    fn the_roll_panels_are_made_for_a_finger() {
        let size = PHONES[0];
        for tab in [WorkspaceTab::PitchRoll, WorkspaceTab::TakeRecorder] {
            let harness = phone_with(size, app_with_rolls(tab));
            let small: Vec<_> = rects(&harness)
                .into_iter()
                .filter(|(role, _, rect)| {
                    matches!(role, Role::Button | Role::TextInput) && rect.height() < space::TOUCH_H
                })
                .collect();
            assert!(
                small.is_empty(),
                "{}: under a finger's height: {small:?}",
                tab.label()
            );
        }
        let harness = phone_with(size, app_with_rolls(WorkspaceTab::TakeRecorder));
        assert!(
            harness
                .query_by_label_contains("pinch sideways zooms time")
                .is_some()
        );
        assert!(harness.query_by_label_contains("wheel").is_none());
    }

    /// REGRESSION: the snail — the phone's default panel — shows its spiral on the
    /// first screen, portrait and landscape. Nine knob rows sat above it and the card
    /// was not in a scroll area, so on 360×740 and on every landscape phone the
    /// spiral was entirely below the bottom edge and could not be scrolled to.
    #[test]
    fn snail_spiral_is_on_the_first_screen() {
        for size in PHONES {
            let harness = phone(size, WorkspaceTab::ResonatorSnail);
            let spiral = harness
                .get_by_role_and_label(Role::Image, "Resonator spiral")
                .rect();
            let screen = Rect::from_min_size(eframe::egui::Pos2::ZERO, size);
            let visible = spiral.intersect(screen);
            assert!(
                visible.width() >= 0.9 * spiral.width() && visible.height() >= 0.6 * spiral.height(),
                "at {size:?} the spiral {spiral:?} is mostly off screen"
            );
            assert!(
                spiral.height() >= 200.0,
                "at {size:?} the spiral {spiral:?} is a sliver"
            );
        }
    }

    /// Each split panel (`mobile_split`) with the node of its screen and one of its
    /// config's controls.
    const SPLITS: [(WorkspaceTab, &str, &str); 4] = [
        (WorkspaceTab::ResonatorSnail, "Resonator spiral", "Spread"),
        (WorkspaceTab::PitchRoll, "Pitch roll", "SWIPE′"),
        (WorkspaceTab::LiveAnalysis, "Spiral spectrogram", "FFT"),
        (WorkspaceTab::Staff, "Violin staff", "Treble"),
    ];

    /// All nodes labelled exactly `label`, as one rect (a slider and its caption share
    /// one name), or `None` if there are none.
    fn rect_of(harness: &Harness<'_, App>, label: &str) -> Option<Rect> {
        rects(harness)
            .into_iter()
            .filter(|(_, l, _)| l == label)
            .map(|(_, _, rect)| rect)
            .reduce(|a, b| a.union(b))
    }

    /// The phone's split panels (Stas, 2026-10-22: «экран инструмента сверху, его конфиг
    /// снизу, и только если конфиг включён»): the screen is whole on the first screen with
    /// the config under it in portrait and beside it, on the right, in landscape; with the
    /// config hidden its controls are gone and the screen keeps (the roll: grows into)
    /// the room.
    #[test]
    fn split_panels_put_the_screen_over_its_config_and_take_the_room_without_it() {
        for size in PHONES {
            let portrait = size.y > size.x;
            for (tab, screen_label, config_label) in SPLITS {
                let at = format!("{} at {size:?}", tab.label());
                let shown = phone_with(size, app_with_rolls(tab));
                let pane = pane(&shown, size);
                let screen = rect_of(&shown, screen_label).unwrap();
                let config =
                    rect_of(&shown, config_label).unwrap_or_else(|| panic!("{at}: no {config_label}"));
                assert!(
                    pane.contains_rect(screen),
                    "{at}: screen {screen:?} is not whole in {pane:?}"
                );
                assert!(
                    pane.contains_rect(config),
                    "{at}: config {config:?} is not in {pane:?}"
                );
                // Nothing past the split's right edge (the picker row's: both span the
                // shell's content width), where each part's scroll area clips. The
                // snail's A4 row («440.0 Hz») did run past it in a landscape config column.
                let header = format!("{}  ▼", tab.label());
                let edge = shown.get_by_role_and_label(Role::Button, &header).rect().right() + 0.5;
                let past: Vec<_> = rects(&shown)
                    .into_iter()
                    .filter(|(_, _, rect)| rect.right() > edge)
                    .collect();
                assert!(past.is_empty(), "{at}: past the split's right edge: {past:?}");
                if portrait {
                    assert!(
                        screen.bottom() < config.top(),
                        "{at}: screen {screen:?} over config {config:?}"
                    );
                } else {
                    assert!(
                        screen.right() < config.left(),
                        "{at}: screen {screen:?} left of config {config:?}"
                    );
                }

                let mut app = app_with_rolls(tab);
                app.mobile_config_shown = false;
                let hidden = phone_with(size, app);
                assert!(
                    rect_of(&hidden, config_label).is_none(),
                    "{at}: hidden config still shows"
                );
                let alone = rect_of(&hidden, screen_label).unwrap();
                assert!(
                    pane.contains_rect(alone),
                    "{at}: screen {alone:?} alone is not whole in {pane:?}"
                );
                assert!(
                    alone.height() >= screen.height(),
                    "{at}: {alone:?} shrank from {screen:?}"
                );
                if tab == WorkspaceTab::PitchRoll && portrait {
                    assert!(
                        alone.height() > screen.height() + config.height(),
                        "{at}: the roll {alone:?} did not take the config's room (was {screen:?})"
                    );
                }
            }
        }
    }
}
