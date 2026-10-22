//! Fingers for `egui_kittest`: touch input exactly as a phone delivers it to egui.
//!
//! kittest can click and hover, but a phone does not send clicks — it sends touches,
//! and `egui-winit` *also* emulates the mouse with the first finger down (`on_touch`):
//! that finger's touch arrives together with a pointer press/move/release, the others
//! as bare touches. Both halves matter to the panels, and to their bugs: a pinch is read
//! off the touches (`InputState::multi_touch`) while every widget still sees the first
//! finger as a dragged pointer. A test that sent only one half would not be a phone.

use eframe::egui::{
    Event,
    PointerButton,
    Pos2,
    TouchDeviceId,
    TouchId,
    TouchPhase,
};
use egui_kittest::Harness;

/// One finger on the screen. `id` 0 is the one `egui-winit` turns into the pointer —
/// the first finger down; put it down first.
#[derive(Clone, Copy)]
pub(super) struct Finger {
    pub(super) id: u64,
}

impl Finger {
    pub(super) const FIRST: Self = Self { id: 0 };
    pub(super) const SECOND: Self = Self { id: 1 };

    /// Queue this finger's event at `pos` for the next frame — the touch, and for the
    /// first finger the mouse emulation `egui-winit` sends alongside it.
    pub(super) fn send<S>(self, harness: &Harness<'_, S>, phase: TouchPhase, pos: Pos2) {
        harness.event(Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(self.id),
            phase,
            pos,
            force: None,
        });
        if self.id != Self::FIRST.id {
            return;
        }
        let button = |pressed| {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }
        };
        match phase {
            TouchPhase::Start => {
                harness.event(Event::PointerMoved(pos));
                harness.event(button(true));
            }
            TouchPhase::Move => harness.event(Event::PointerMoved(pos)),
            TouchPhase::End | TouchPhase::Cancel => {
                harness.event(button(false));
                harness.event(Event::PointerGone);
            }
        }
    }
}

/// Put `fingers` down at their `from`, move them together in `steps` frames to their
/// `to`, and lift them — first finger first down, last up, like a hand.
pub(super) fn gesture<S>(harness: &mut Harness<'_, S>, fingers: &[(Finger, Pos2, Pos2)], steps: usize) {
    for &(finger, from, _) in fingers {
        finger.send(harness, TouchPhase::Start, from);
        harness.step();
    }
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        for &(finger, from, to) in fingers {
            finger.send(harness, TouchPhase::Move, from.lerp(to, t));
        }
        harness.step();
    }
    for &(finger, _, to) in fingers.iter().rev() {
        finger.send(harness, TouchPhase::End, to);
        harness.step();
    }
}
