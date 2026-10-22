//! Text painted inside a plot frame: its title row and the hint shown in place of an
//! empty plot. Plots are painted, not laid out, so nothing wraps or makes room on its
//! own — on a 360 dp phone a one-line hint is wider than the plot and a caption runs
//! into the title. These two keep them inside the frame at any width.

use eframe::egui::{
    self,
    Color32,
    FontId,
    Painter,
    Rect,
    pos2,
};

use crate::ui::tokens::color;

/// Inset of the title row from the plot frame's top-left / top-right corner.
const TITLE_INSET: egui::Vec2 = egui::vec2(14.0, 12.0);
/// Least gap between the title's end and the caption's start.
const TITLE_CAPTION_GAP: f32 = 12.0;

/// The plot's title at its top-left and, at its top-right, a caption explaining it.
///
/// The caption is painted only where it fits beside the title. It is an aside (the
/// title names the plot), so on a narrow plot — a phone, a small pane — dropping it
/// beats painting the two over each other, which is what a fixed right-aligned caption
/// did on every 360–412 dp phone.
pub fn draw_title(painter: &Painter, rect: Rect, title: &str, caption: &str) {
    let title = painter.layout_no_wrap(title.to_owned(), FontId::proportional(15.0), color::TEXT_BADGE);
    let caption = painter.layout_no_wrap(caption.to_owned(), FontId::proportional(12.0), color::TEXT_HINT);
    let room = rect.width() - 2.0 * TITLE_INSET.x;
    painter.galley(rect.left_top() + TITLE_INSET, title.clone(), color::TEXT_BADGE);
    if title.size().x + TITLE_CAPTION_GAP + caption.size().x <= room {
        let pos = pos2(
            rect.right() - TITLE_INSET.x - caption.size().x,
            rect.top() + TITLE_INSET.y,
        );
        painter.galley(pos, caption, color::TEXT_HINT);
    }
}

/// A hint in place of the plot, centred in `rect` and wrapped to its width. On one line
/// such hints are 250–400 dp wide — wider than a phone's plot, which cut both their ends
/// off.
pub fn draw_hint(painter: &Painter, rect: Rect, text: &str, size: f32, ink: Color32) {
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), FontId::proportional(size), ink, rect.width());
    job.halign = egui::Align::Center;
    let galley = painter.layout_job(job);
    // Centred rows lie around the galley's origin, so place it by its own centre.
    let pos = rect.center() - galley.rect.center().to_vec2();
    painter.galley(pos, galley, ink);
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        self,
        Rect,
        pos2,
    };

    use super::*;

    /// Every text shape the closure paints, as its rect.
    fn painted(width: f32, paint: impl Fn(&Painter, Rect)) -> Vec<Rect> {
        let ctx = egui::Context::default();
        crate::ui::theme::install_fonts(&ctx);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(width, 300.0));
        // A bare background painter: the frame paints nothing else to tell apart.
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint(&ui.ctx().layer_painter(egui::LayerId::background()), rect)
        });
        output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Text(_)))
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .collect()
    }

    /// REGRESSION: on a phone-width plot the caption is dropped, not painted over the
    /// title; on a wide one both are there, apart.
    #[test]
    fn the_caption_never_runs_into_the_title() {
        let paint = |painter: &Painter, rect: Rect| {
            draw_title(
                painter,
                rect,
                "Raw waveform preview",
                "recent mono samples after input gain",
            )
        };
        let narrow = painted(298.0, paint);
        assert_eq!(narrow.len(), 1, "{narrow:?}");
        let wide = painted(600.0, paint);
        assert_eq!(wide.len(), 2, "{wide:?}");
        assert!(wide[0].right() + TITLE_CAPTION_GAP <= wide[1].left(), "{wide:?}");
        assert!(wide[1].right() <= 600.0 - TITLE_INSET.x + 0.5, "{wide:?}");
    }

    /// REGRESSION: a hint longer than the plot is wide wraps inside it.
    #[test]
    fn a_long_hint_wraps_inside_the_plot() {
        for width in [200.0, 298.0, 600.0] {
            let rects = painted(width, |painter, rect| {
                draw_hint(
                    painter,
                    rect,
                    "The resonator bank starts filling as soon as audio reaches the input",
                    13.0,
                    color::TEXT_HINT,
                )
            });
            assert_eq!(rects.len(), 1);
            assert!(
                rects[0].left() >= -0.5 && rects[0].right() <= width + 0.5,
                "at {width}: {rects:?}"
            );
        }
    }
}
