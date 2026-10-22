//! Items of a wrapping row (`ui.horizontal_wrapped`) that a narrow line must not split.
//!
//! A wrapping row breaks between any two allocations, and on a 360–412 dp phone two
//! things came apart that way:
//!
//! - a group: the fretboard's "Root" caption ended one line and its seven note pills
//!   opened the next (on 412 dp "Root C" ended it, D…B went below);
//! - a separator: `ui.separator()` is an item of its own, so it stayed at the end of a
//!   line whose next item wrapped — "System |", "16.4k |", "C8 |" — a divider with
//!   nothing after it.
//!
//! [`keep_together`] lays a group out as ONE allocation, so the row moves it to a fresh
//! line whole; [`separated`] puts a divider before such a group only when the group
//! stays on the divider's line. A single fixed-width widget (`SliderRow`, a captioned
//! `PillCombo`) already wraps whole; passing it through [`separated`] is what drops its
//! divider at a line end.

use eframe::egui::{
    Align,
    InnerResponse,
    Layout,
    Ui,
    Vec2,
    vec2,
};

/// A divider's own width between its neighbours' item spacing — egui's `Separator`
/// default, so a divided row keeps the spacing it had with `ui.separator()`.
const SEPARATOR_W: f32 = 6.0;

/// Lay `add`'s widgets out left to right as one item of the wrapping row `ui`: if the
/// group does not fit what is left of the line, all of it goes to the next line.
///
/// A wrapping row decides where an item goes by the size it asks for *before* the item
/// is drawn, and a group's size is known only after. So the group asks for the size it
/// had last frame (egui temp memory under `id_salt`, as `mobile_split` keeps its
/// config's size); the first frame, with nothing remembered, is discarded and redrawn
/// (`request_discard`) so a wrong guess is never shown. The height is remembered too, not
/// taken from the row band: the row centres items on its line, and a group of labels
/// asking for a 44 dp band would sit lower than a plain label before it.
///
/// A group wider than a whole line wraps inside itself instead of running past the
/// screen edge. The width remembered is then still the group's one-line width, not the
/// wrapped one: a wrapped group measures narrower, and remembering that would let it
/// back after something on a line where only part of it fits, wrapped there for good.
pub fn keep_together<R>(ui: &mut Ui, id_salt: &str, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let id = ui.make_persistent_id(("wrap_row::keep_together", id_salt));
    let remembered = ui.data(|data| data.get_temp::<Vec2>(id));
    if remembered.is_none() {
        ui.ctx()
            .request_discard("wrap_row: a kept-together group's size is not known yet");
    }
    let line_w = ui.max_rect().width();
    let size = remembered.unwrap_or(Vec2::ZERO);
    let wraps = size.x > line_w;
    let inner = ui.allocate_ui_with_layout(
        vec2(size.x.min(line_w), size.y),
        Layout::top_down(Align::Min),
        |ui| {
            // The group's widgets see the whole line, as they would in the row itself, not
            // the width remembered from last frame: a widget sized off `max_rect` (a
            // `SliderRow`'s slider, the device combo) would otherwise shrink to whatever it
            // was measured at — 0 on the first frame — and stay there.
            ui.set_max_width(line_w);
            // A row of its own, not a layout set on this child: a wrapping layout spaces its
            // lines by the height of its `max_rect`, which is the remembered height, so a
            // group that wrapped inside measured taller every frame without end.
            let inner = if wraps {
                ui.horizontal_wrapped(add).inner
            } else {
                ui.horizontal(add).inner
            };
            let used = ui.min_rect().size();
            let width = if wraps { used.x.max(size.x) } else { used.x };
            ui.data_mut(|data| data.insert_temp(id, vec2(width, used.y)));
            inner
        },
    );
    InnerResponse::new(inner.inner, inner.response)
}

/// A divider, then `add`'s group kept together ([`keep_together`]) — the divider drawn
/// only when the group ends up on its line, after something.
///
/// The divider's place is allocated first, as `ui.separator()` would; if the group then
/// wraps, that place is left blank at the end of the line above. A line with no room
/// left even for the divider, or none used yet, gets no place at all — it would wrap
/// and indent the group on its new line. Returns whether the divider was drawn.
pub fn separated<R>(ui: &mut Ui, id_salt: &str, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let rest = ui.available_size_before_wrap().x;
    let at_line_start = ui.cursor().left() <= ui.max_rect().left() + 0.5;
    // Zero height: the place must not raise the line (its items are centred on it).
    let gap = (!at_line_start && rest >= SEPARATOR_W + ui.spacing().item_spacing.x)
        .then(|| ui.allocate_space(vec2(SEPARATOR_W, 0.0)).1);
    let group = keep_together(ui, id_salt, add);
    let rect = group.response.rect;
    // Left to right, an item starts left of an earlier one's end only on a new line.
    let drawn = gap.filter(|gap| rect.left() >= gap.right() - 0.5);
    if let Some(gap) = drawn {
        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
        ui.painter()
            .vline(gap.center().x, rect.top()..=rect.bottom(), stroke);
    }
    (group.inner, drawn.is_some())
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        Rect,
        Vec2,
        vec2,
    };
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::{
        keep_together,
        separated,
    };
    use crate::ui::segmented::{
        RowCaption,
        SegmentedButton,
    };

    /// What one frame of the row below left: the divider drawn or not.
    #[derive(Default)]
    struct Row {
        divided: bool,
    }

    /// A wrapping row as wide as `width`: a 100 dp pill, then a divider and a group of
    /// four 60 dp pills — "Root" before them as its caption.
    fn row(width: f32) -> Harness<'static, Row> {
        let mut harness = Harness::builder()
            .with_size(Vec2::new(width, 200.0))
            .build_ui_state(
                |ui, row: &mut Row| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(SegmentedButton::new("Before", false).min_width(100.0));
                        let ((), divided) = separated(ui, "notes", |ui| {
                            ui.add(RowCaption::new("Root"));
                            for note in ["C", "D", "E", "F"] {
                                ui.add(SegmentedButton::new(note, false).min_width(60.0));
                            }
                        });
                        row.divided = divided;
                    });
                },
                Row::default(),
            );
        harness.run_steps(3);
        harness
    }

    fn rect(harness: &Harness<'_, Row>, label: &str) -> Rect {
        harness.get_by_label(label).rect()
    }

    /// Room on the line: the group follows the pill there, divided from it.
    #[test]
    fn a_group_that_fits_stays_on_the_line_after_a_divider() {
        let harness = row(600.0);
        let before = rect(&harness, "Before");
        let root = rect(&harness, "Root");
        assert!(
            (root.center().y - before.center().y).abs() < 1.0,
            "{before:?} {root:?}"
        );
        assert!(harness.state().divided);
    }

    /// No room: the whole group — caption and every pill — opens the next line, and the
    /// divider is not left hanging at the end of the first.
    #[test]
    fn a_group_that_does_not_fit_wraps_whole_without_its_divider() {
        let harness = row(330.0);
        let before = rect(&harness, "Before");
        let root = rect(&harness, "Root");
        assert!(root.top() >= before.bottom(), "{before:?} {root:?}");
        for note in ["C", "D", "E", "F"] {
            let pill = rect(&harness, note);
            assert!(
                (pill.center().y - root.center().y).abs() < 1.0,
                "{note} {pill:?} left its caption {root:?}"
            );
        }
        assert!(!harness.state().divided, "divider left at the end of the line");
    }

    /// A group wider than a whole line wraps inside itself rather than running past it.
    #[test]
    fn a_group_wider_than_the_line_wraps_inside_it() {
        let mut harness = Harness::builder().with_size(vec2(200.0, 300.0)).build_ui(|ui| {
            ui.horizontal_wrapped(|ui| {
                keep_together(ui, "wide", |ui| {
                    for note in ["C", "D", "E", "F"] {
                        ui.add(SegmentedButton::new(note, false).min_width(60.0));
                    }
                });
            });
        });
        harness.run_steps(3);
        for note in ["C", "D", "E", "F"] {
            let pill = harness.get_by_label(note).rect();
            assert!(pill.right() <= 200.0, "{note} {pill:?} past the edge");
        }
    }
}
