//! The phone's standard panel layout: the instrument's screen on top, its config under
//! it — and the config only while it is shown; hidden, the screen takes the whole pane.
//!
//! Stas, 2026-10-22 (decided, not to be revisited): «на телефоне экран инструмента
//! сверху, его конфиг снизу, и только если конфиг включён». One layout for every panel
//! that comes as a view and its settings, so the phone has one way of reading them
//! instead of a variant per panel. A panel joins by answering
//! [`WorkspaceTab::splits_on_phone`] and drawing its two parts in
//! [`App::draw_mobile_part`]; panels without such a pair keep the shell's plain scroll.
//!
//! Landscape puts the config beside the screen, on the right, as the snail always did
//! there: stacked, a 360 dp-tall phone leaves either part too short to use.

use std::fmt::Debug;
use std::hash::Hash;

use eframe::egui::{
    self,
    Frame,
    Id,
    Rect,
    Stroke,
    Ui,
    UiBuilder,
    Vec2,
    pos2,
};

use super::{
    App,
    WorkspaceTab,
};
use crate::ui::theme;
use crate::ui::tokens::color;

/// The most of the pane's height the config may take in portrait. The screen is what the
/// panel is for, so a config taller than this scrolls in its own area instead of
/// squeezing the screen further. At 0.4 a 360×740 portrait still gives the snail a
/// full-width spiral (≈400 of the ≈670 dp pane).
const CONFIG_SHARE: f32 = 0.4;
/// The most of the pane's width the config may take in landscape. Wider than the portrait
/// share: the snail's widest knob row (A4: −, slider, +, «440.0 Hz») is ≈294 dp with its
/// card, past 0.4 of a 740 dp landscape pane; the screen keeps the rest, and the content
/// narrower than the cap (the roll's two switches) takes only its own width anyway.
const CONFIG_SHARE_WIDE: f32 = 0.45;

/// Space between the screen and the config card.
const GAP: f32 = 8.0;

/// The two parts of a split panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    /// The view the panel is for: the spiral, the roll, the staff, the meter.
    Screen,
    /// What shapes that view: its knobs, layer switches, pickers.
    Config,
}

impl WorkspaceTab {
    /// Does this panel come as a screen and its config on the phone
    /// ([`App::draw_mobile_part`] draws both)?
    pub(super) fn splits_on_phone(self) -> bool {
        matches!(
            self,
            Self::ResonatorSnail | Self::PitchRoll | Self::LiveAnalysis | Self::Staff
        )
    }
}

impl App {
    /// A split panel in the phone's pane — see the module docs.
    pub(super) fn draw_mobile_split(&mut self, ui: &mut Ui, tab: WorkspaceTab) {
        debug_assert!(tab.splits_on_phone(), "{} has no screen/config pair", tab.label());
        split_screen(
            ui,
            ("mobile_split", tab.label()),
            self.mobile_config_shown,
            self,
            |app, ui| app.draw_mobile_part(ui, tab, Part::Screen),
            |app, ui| app.draw_mobile_part(ui, tab, Part::Config),
        );
    }

    /// One part of a split panel: the per-panel dispatch for [`split_screen`].
    fn draw_mobile_part(&mut self, ui: &mut Ui, tab: WorkspaceTab, part: Part) {
        match tab {
            WorkspaceTab::ResonatorSnail => self.draw_mobile_snail_part(ui, part),
            WorkspaceTab::PitchRoll => self.draw_pitch_roll_part(ui, part),
            WorkspaceTab::LiveAnalysis => self.draw_tuner_part(ui, part),
            WorkspaceTab::Staff => self.draw_staff_part(ui, part),
            tab => unreachable!("{} does not split on the phone", tab.label()),
        }
    }
}

/// Lay `screen` and `config` out in the rest of `ui` — see the module docs.
///
/// The config is sized to its own content, capped at [`CONFIG_SHARE`] of the pane. Its
/// content size is only known once it has been drawn, so it is the previous frame's,
/// kept in egui's temp memory under `id_salt` (the way egui's own side panels remember
/// theirs); the first frame gives the config its whole share. The screen gets everything
/// else — inside it `available_height()` runs to the screen's bottom, so a screen that
/// fills it ends exactly there. Each part scrolls in its own area should its content not
/// fit, rather than being cut.
///
/// The orientation is the device screen's, not the pane's: with the panel picker unfolded
/// a portrait pane is short, and a config column beside the screen would be too narrow.
pub(super) fn split_screen<T>(
    ui: &mut Ui,
    id_salt: impl Hash + Debug,
    show_config: bool,
    state: &mut T,
    screen: impl FnOnce(&mut T, &mut Ui),
    config: impl FnOnce(&mut T, &mut Ui),
) {
    // One split per panel on the screen at a time — a global id, as the picker's.
    let id = Id::unique(id_salt);
    let pane = ui.available_rect_before_wrap();
    let device = ui.ctx().content_rect();
    let side_by_side = device.width() > device.height();
    let measured: Option<Vec2> = ui.data(|data| data.get_temp(id));
    let wanted = show_config.then(|| measured.unwrap_or(Vec2::INFINITY));
    let (screen_rect, config_rect) = split_rects(pane, side_by_side, wanted);

    ui.scope_builder(UiBuilder::new().max_rect(screen_rect), |ui| {
        egui::ScrollArea::both()
            .id_salt(id.with("screen"))
            .auto_shrink([false, false])
            .show(ui, |ui| screen(state, ui));
    });

    let Some(config_rect) = config_rect else {
        return;
    };
    let content = ui
        .scope_builder(UiBuilder::new().max_rect(config_rect), |ui| {
            // A solid bar while the config overflows its share (the snail's knobs do).
            // egui's floating bar shows only under a hovering pointer, which a finger never
            // is, and a config cut mid-screen — unlike a page cut at the screen's edge —
            // does not look like it goes on: its last row was simply missing.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            egui::ScrollArea::both()
                .id_salt(id.with("config"))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (pad, radius) = theme::card_pad(ui.ctx(), 14, 22);
                    Frame::new()
                        .fill(color::PANEL_FILL)
                        .corner_radius(radius)
                        .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
                        .inner_margin(pad)
                        .show(ui, |ui| {
                            // The card spans the split's cross direction, as the screen's
                            // card does — pane-wide under the screen, pane-tall beside it.
                            // Along the split it is its content's size: that is the size
                            // measured for the next frame.
                            if side_by_side {
                                ui.set_min_height(ui.available_height());
                            } else {
                                ui.set_min_width(ui.available_width());
                            }
                            config(state, ui);
                        });
                })
                .content_size
        })
        .inner;
    if measured != Some(content) {
        ui.data_mut(|data| data.insert_temp(id, content));
        ui.ctx().request_repaint();
    }
}

/// The screen's and the config's rects in `pane`. `config` is the size the config wants
/// (`None` = hidden: the screen is the whole pane); it gets that, up to [`CONFIG_SHARE`]
/// ([`CONFIG_SHARE_WIDE`] side by side) of the pane along the split, and the whole pane
/// across it.
fn split_rects(pane: Rect, side_by_side: bool, config: Option<Vec2>) -> (Rect, Option<Rect>) {
    let Some(config) = config else {
        return (pane, None);
    };
    if side_by_side {
        let width = config.x.min(pane.width() * CONFIG_SHARE_WIDE);
        let left = pane.right() - width;
        let screen = Rect::from_min_max(pane.min, pos2(left - GAP, pane.bottom()));
        (screen, Some(Rect::from_min_max(pos2(left, pane.top()), pane.max)))
    } else {
        let height = config.y.min(pane.height() * CONFIG_SHARE);
        let top = pane.bottom() - height;
        let screen = Rect::from_min_max(pane.min, pos2(pane.right(), top - GAP));
        (screen, Some(Rect::from_min_max(pos2(pane.left(), top), pane.max)))
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::os::OperatingSystem;
    use eframe::egui::{
        Sense,
        vec2,
    };
    use egui_kittest::Harness;

    use super::*;

    /// What a split laid out, as its two parts saw it.
    struct Probe {
        show_config: bool,
        /// What the split was given.
        pane:        Rect,
        /// The screen's whole room: its `max_rect` (the viewport of its scroll area).
        screen:      Rect,
        /// The config's content — one fixed-size block.
        config:      Option<Rect>,
    }

    const CONFIG_CONTENT: Vec2 = vec2(120.0, 100.0);

    /// A real split at phone `size`, its config a `CONFIG_CONTENT` block.
    fn probe(size: Vec2, show_config: bool) -> Probe {
        let mut harness = Harness::builder()
            .with_size(size)
            .with_os(OperatingSystem::Android)
            .build_ui_state(
                |ui, probe: &mut Probe| {
                    probe.config = None;
                    probe.pane = ui.available_rect_before_wrap();
                    split_screen(
                        ui,
                        "split_probe",
                        probe.show_config,
                        probe,
                        |probe, ui| probe.screen = ui.max_rect(),
                        |probe, ui| {
                            probe.config = Some(ui.allocate_exact_size(CONFIG_CONTENT, Sense::hover()).0);
                        },
                    );
                },
                Probe {
                    show_config,
                    pane: Rect::NOTHING,
                    screen: Rect::NOTHING,
                    config: None,
                },
            );
        // The first frame lays the config out at its full share; the second at its size.
        harness.run_steps(3);
        harness.into_state()
    }

    /// The config switched on and off changes the screen's height (width in landscape)
    /// by exactly what the config takes, and the config sits under (beside) the screen at
    /// its own size, not its cap.
    #[test]
    fn the_config_takes_its_size_from_the_screen_and_gives_it_back_when_hidden() {
        let pad = f32::from(crate::ui::tokens::space::TOUCH_CARD_PAD) + 1.0;
        for (size, side_by_side) in [(vec2(360.0, 740.0), false), (vec2(740.0, 360.0), true)] {
            let hidden = probe(size, false);
            let pane = hidden.pane;
            assert_eq!(
                hidden.screen, pane,
                "{size:?}: hidden config, the screen is the pane"
            );
            assert!(hidden.config.is_none());

            let shown = probe(size, true);
            assert_eq!(shown.pane, pane);
            let config = shown.config.unwrap();
            let card = config.expand(pad);
            if side_by_side {
                assert_eq!(
                    card.right(),
                    pane.right(),
                    "{size:?}: config {config:?} not at the right"
                );
                assert_eq!(shown.screen.height(), pane.height());
                assert_eq!(
                    shown.screen.right() + GAP,
                    card.left(),
                    "{size:?}: {:?}",
                    shown.screen
                );
            } else {
                assert_eq!(
                    card.bottom(),
                    pane.bottom(),
                    "{size:?}: config {config:?} not at the bottom"
                );
                assert_eq!(shown.screen.width(), pane.width());
                assert_eq!(
                    shown.screen.bottom() + GAP,
                    card.top(),
                    "{size:?}: {:?}",
                    shown.screen
                );
            }
        }
    }

    const PANE: Rect = Rect {
        min: pos2(8.0, 60.0),
        max: pos2(352.0, 732.0),
    };

    #[test]
    fn a_hidden_config_leaves_the_screen_the_whole_pane() {
        for side_by_side in [false, true] {
            assert_eq!(split_rects(PANE, side_by_side, None), (PANE, None));
        }
    }

    /// A short config takes its own height; the screen ends a gap above it.
    #[test]
    fn a_short_config_takes_its_own_size_under_the_screen() {
        let (screen, config) = split_rects(PANE, false, Some(vec2(200.0, 60.0)));
        let config = config.unwrap();
        assert_eq!(config.height(), 60.0);
        assert_eq!(
            config.width(),
            PANE.width(),
            "across the split the config is pane-wide"
        );
        assert_eq!(config.bottom(), PANE.bottom());
        assert_eq!(screen.top(), PANE.top());
        assert_eq!(screen.bottom(), config.top() - GAP);
    }

    /// A config bigger than its share is cut to it (and scrolls); so is the unmeasured
    /// first frame's.
    #[test]
    fn a_tall_or_unmeasured_config_gets_its_share_and_no_more() {
        for wanted in [vec2(100.0, 900.0), Vec2::INFINITY] {
            let (screen, config) = split_rects(PANE, false, Some(wanted));
            let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
            assert!(near(config.unwrap().height(), PANE.height() * CONFIG_SHARE));
            assert!(near(screen.height(), PANE.height() * (1.0 - CONFIG_SHARE) - GAP));
        }
    }

    #[test]
    fn landscape_puts_the_config_on_the_right() {
        let pane = Rect::from_min_max(pos2(8.0, 60.0), pos2(732.0, 352.0));
        let (screen, config) = split_rects(pane, true, Some(vec2(120.0, 900.0)));
        let config = config.unwrap();
        assert_eq!(config.width(), 120.0);
        assert_eq!(config.height(), pane.height());
        assert_eq!(config.right(), pane.right());
        assert_eq!(screen.left(), pane.left());
        assert_eq!(screen.right(), config.left() - GAP);
        assert_eq!(screen.height(), pane.height());
    }
}
