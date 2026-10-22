//! The phone's back button (and the back gesture, which Android delivers as the same
//! key): what it undoes, in the order a user expects from any Android app.
//!
//! 1. The panel picker is unfolded → fold it. It is the one thing on the phone that
//!    opens over the panel, and back closing what was just opened is the platform's rule.
//! 2. The user came to this panel from another → go back to that one. Each pick in the
//!    picker leaves the panel it replaced in [`App::mobile_back_stack`].
//! 3. Nothing left to go back to → leave the app, as the system's own back does at an
//!    app's first screen. The shell cannot do that itself; it asks
//!    ([`ShellRequest::LeaveApp`]) and the Android `render` moves the task to the back.
//!    Except while a take is being recorded: then back stays (see
//!    [`App::handle_mobile_back`]).
//!
//! Why the app has to do any of this: winit reports every key but the volume keys as
//! handled to Android (`winit` `platform_impl/android`, `handle_input_event`), so the
//! system never gets to act on back. Before this the button and the gesture did nothing
//! at all — not even fold the picker.

use eframe::egui::{
    self,
    Event,
    Key,
};

use super::{
    App,
    WorkspaceTab,
};

/// What the phone shell needs done this frame that only the platform can do.
#[must_use]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ShellRequest {
    /// Nothing: the shell handled everything itself.
    Stay,
    /// Back was pressed with nothing left to go back to: leave the app.
    LeaveApp,
}

impl App {
    /// Show `tab` on the phone, remembering the panel it replaces for back.
    ///
    /// The stack holds each panel once and never the one on screen: picking a panel that
    /// is already in it moves it to the top rather than stacking a second copy. So back
    /// visits every panel the user has been to, most recent first, and stops — never
    /// cycling between two of them, and never longer than the sixteen panels.
    pub(super) fn open_mobile_panel(&mut self, tab: WorkspaceTab) {
        if tab == self.mobile_panel {
            return;
        }
        self.mobile_back_stack.retain(|&left| left != tab);
        self.mobile_back_stack.push(self.mobile_panel);
        self.mobile_panel = tab;
    }

    /// Act on this frame's back presses — see the module docs — and consume them, so
    /// no widget sees the key.
    ///
    /// Only fresh presses count: holding the button down repeats the key, and a held
    /// back must not walk the whole stack and drop the user out of the app.
    pub(super) fn handle_mobile_back(&mut self, ctx: &egui::Context) -> ShellRequest {
        let presses = ctx.input_mut(|input| {
            let mut presses = 0;
            input.events.retain(|event| {
                let Event::Key {
                    key: Key::BrowserBack,
                    pressed,
                    repeat,
                    ..
                } = event
                else {
                    return true;
                };
                presses += usize::from(*pressed && !*repeat);
                false
            });
            presses
        });

        for _ in 0..presses {
            let picker_open = super::workspace::picker_open_id();
            if ctx.data(|data| data.get_temp::<bool>(picker_open)) == Some(true) {
                ctx.data_mut(|data| data.insert_temp(picker_open, false));
            } else if let Some(previous) = self.mobile_back_stack.pop() {
                self.mobile_panel = previous;
            } else if self.audio.take_wanted() {
                // A take is being recorded: stay. In the background Android hands an
                // app's capture silence (Android 9+, no foreground service here), so
                // the take would go on being written — as silence — and the
                // performance would be lost. On gesture navigation an edge swipe is
                // back, easy to make by accident; Stop is the way to end a take.
            } else {
                return ShellRequest::LeaveApp;
            }
        }
        ShellRequest::Stay
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::os::OperatingSystem;
    use eframe::egui::{
        Event,
        Key,
        Modifiers,
        Role,
        Vec2,
        vec2,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::ShellRequest;
    use crate::app::touch_fingers::{
        Finger,
        gesture,
    };
    use crate::app::{
        App,
        WorkspaceTab,
    };

    /// The phone shell and every request it made, in order.
    struct Phone {
        app:      App,
        requests: Vec<ShellRequest>,
    }

    const SIZE: Vec2 = vec2(360.0, 740.0);

    fn phone(tab: WorkspaceTab) -> Harness<'static, Phone> {
        let mut app = App::for_ui_test();
        app.mobile_panel = tab;
        let mut harness = Harness::builder()
            .with_size(SIZE)
            .with_os(OperatingSystem::Android)
            .with_context_setup(|ctx| {
                crate::ui::theme::install_fonts(ctx);
                crate::ui::theme::apply_theme(ctx);
            })
            .build_ui_state(
                |ui, phone: &mut Phone| {
                    let request = phone.app.render_mobile_shell(ui);
                    if request != ShellRequest::Stay {
                        phone.requests.push(request);
                    }
                },
                Phone {
                    app,
                    requests: Vec::new(),
                },
            );
        harness.run_steps(4);
        harness
    }

    fn tap(harness: &mut Harness<'_, Phone>, label: &str) {
        let at = harness.get_by_role_and_label(Role::Button, label).rect().center();
        gesture(harness, &[(Finger::FIRST, at, at)], 0);
        harness.run_steps(2);
    }

    /// Pick `tab` the way a finger does: unfold the picker, tap the tile.
    fn pick(harness: &mut Harness<'_, Phone>, tab: WorkspaceTab) {
        let header = format!("{}  ▼", harness.state().app.mobile_panel.label());
        tap(harness, &header);
        tap(harness, tab.label());
        assert!(harness.state().app.mobile_panel == tab);
    }

    /// The back button as `egui-winit` delivers it: a press (`repeat` while held) and a
    /// release.
    fn back(harness: &mut Harness<'_, Phone>, repeats: usize) {
        let key = |pressed, repeat| {
            Event::Key {
                key: Key::BrowserBack,
                physical_key: None,
                pressed,
                repeat,
                modifiers: Modifiers::NONE,
            }
        };
        harness.event(key(true, false));
        harness.step();
        for _ in 0..repeats {
            harness.event(key(true, true));
            harness.step();
        }
        harness.event(key(false, false));
        harness.run_steps(2);
    }

    fn shown(harness: &Harness<'_, Phone>) -> WorkspaceTab {
        let tab = harness.state().app.mobile_panel;
        // What is on screen, not just the field: the picker's row names the panel.
        harness.get_by_role_and_label(Role::Button, &format!("{}  ▼", tab.label()));
        tab
    }

    /// Back folds an unfolded picker and does nothing else: the panel stays, the app stays.
    #[test]
    fn back_folds_the_open_picker_first() {
        let mut harness = phone(WorkspaceTab::Drone);
        tap(&mut harness, "Drone  ▼");
        assert!(harness.query_by_label("Pitch Roll").is_some(), "picker unfolded");

        back(&mut harness, 0);
        assert!(
            harness.query_by_label("Pitch Roll").is_none(),
            "back left the picker open"
        );
        assert!(shown(&harness) == WorkspaceTab::Drone);
        assert!(harness.state().requests.is_empty());
    }

    /// Back retraces the panels the user picked, most recent first, each once — a panel
    /// picked twice is found where it was last left — and then leaves the app.
    #[test]
    fn back_walks_the_picked_panels_back_and_then_leaves() {
        use WorkspaceTab::{
            Drone,
            PitchRoll,
            ResonatorSnail,
            TakeRecorder,
        };
        let mut harness = phone(ResonatorSnail);
        for tab in [Drone, PitchRoll, Drone, TakeRecorder] {
            pick(&mut harness, tab);
        }

        let mut retraced = Vec::new();
        while harness.state().requests.is_empty() {
            back(&mut harness, 0);
            retraced.push(shown(&harness));
            assert!(retraced.len() <= WorkspaceTab::ALL.len(), "back never leaves");
        }
        assert!(
            retraced == [Drone, PitchRoll, ResonatorSnail, ResonatorSnail],
            "back went {:?}",
            retraced.iter().map(|tab| tab.label()).collect::<Vec<_>>()
        );
        assert_eq!(harness.state().requests, [ShellRequest::LeaveApp]);
    }

    /// Back does not take the app to the background while a take is being recorded —
    /// there it would record silence — and does once the take is stopped.
    #[test]
    fn back_does_not_leave_the_app_in_the_middle_of_a_take() {
        let mut harness = phone(WorkspaceTab::TakeRecorder);
        // The engine has no device: the take is only wanted, nothing is written.
        harness
            .state()
            .app
            .audio
            .start_take("/testdata/never_written.wav".into());

        back(&mut harness, 0);
        assert!(harness.state().requests.is_empty(), "left during a take");
        assert!(shown(&harness) == WorkspaceTab::TakeRecorder);

        harness.state().app.audio.stop_take();
        back(&mut harness, 0);
        assert_eq!(harness.state().requests, [ShellRequest::LeaveApp]);
    }

    /// Holding back is one back: the key's repeats do not walk the stack.
    #[test]
    fn a_held_back_steps_once() {
        let mut harness = phone(WorkspaceTab::ResonatorSnail);
        pick(&mut harness, WorkspaceTab::Drone);
        pick(&mut harness, WorkspaceTab::PitchRoll);

        back(&mut harness, 10);
        assert!(shown(&harness) == WorkspaceTab::Drone);
        assert!(harness.state().requests.is_empty());
    }
}
