//! Pixel-level regression guard for the pill row, rendered through the real
//! egui/epaint/wgpu stack.
//!
//! Why it exists: `ui.horizontal_wrapped` cannot vertically centre its items (a
//! wrapping layout does not know a row's height until the row is closed, so it
//! top-aligns). A plain `ui.label("Source")` is ~16 px tall beside a 28 px pill, so
//! it rendered ~4 px above the pill's own label — plainly visible, and invisible to
//! every font-metric argument we made about the pill's internals. `RowCaption`
//! fixes it by construction; this test is what keeps it fixed.
//!
//! Run: `cargo test --test pill_layout`

use eframe::egui::{
    self,
    Color32,
};
use egui_kittest::Harness;
use fretboard::ui::segmented::{
    RowCaption,
    SegmentedButton,
};

const PPP: f32 = 2.0;
/// Both texts are the SAME string, so their ink bands are identical by
/// construction and any row delta is pure misalignment — no font maths involved.
const WORD: &str = "Source";

/// `caption` picks the fixed `RowCaption` over the old plain-label rendering.
fn render_row(caption: bool) -> image::RgbaImage {
    let mut harness = Harness::builder()
        .with_pixels_per_point(PPP)
        .with_size(egui::Vec2::new(230.0, 40.0))
        .build_ui(move |ui| {
            fretboard::ui::theme::install_fonts(ui.ctx());
            fretboard::ui::theme::apply_theme(ui.ctx());
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(24, 27, 31));
            ui.add_space(5.0);
            ui.horizontal_wrapped(|ui| {
                if caption {
                    ui.add(RowCaption::new(WORD));
                } else {
                    ui.label(
                        egui::RichText::new(WORD)
                            .color(Color32::from_rgb(205, 194, 176))
                            .strong(),
                    );
                }
                ui.add(SegmentedButton::new(WORD, false).min_width(104.0));
            });
        });
    harness.run();
    harness.render().expect("wgpu render failed")
}

/// Rows containing glyph ink inside an x-window. The label (~214 luminance) is far
/// brighter than the idle pill fill (~46) or its outline (~90), so one threshold
/// isolates text from chrome.
fn ink_rows(img: &image::RgbaImage, x0: u32, x1: u32) -> (u32, u32) {
    let (mut top, mut bottom) = (u32::MAX, 0);
    for y in 0..img.height() {
        for x in x0..x1.min(img.width()) {
            let p = img.get_pixel(x, y).0;
            if (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 > 170 {
                top = top.min(y);
                bottom = bottom.max(y);
                break;
            }
        }
    }
    (top, bottom)
}

/// The caption lives in the first ~45 logical px; the pill's label starts past 75.
const CAPTION_X: (u32, u32) = (0, 45 * PPP as u32);
const PILL_TEXT_X: (u32, u32) = (75 * PPP as u32, 200 * PPP as u32);

#[test]
fn row_caption_shares_baseline_with_pill_label() {
    let img = render_row(true);
    let (cap_top, cap_bottom) = ink_rows(&img, CAPTION_X.0, CAPTION_X.1);
    let (pill_top, pill_bottom) = ink_rows(&img, PILL_TEXT_X.0, PILL_TEXT_X.1);

    assert_ne!(cap_top, u32::MAX, "caption text not found in render");
    assert_ne!(pill_top, u32::MAX, "pill label not found in render");
    assert_eq!(
        (cap_top, cap_bottom),
        (pill_top, pill_bottom),
        "caption and pill label must occupy identical ink rows for identical text \
         (caption {cap_top}..{cap_bottom} vs pill {pill_top}..{pill_bottom})"
    );
}

/// Guards the bug this whole exercise was actually about: a plain `ui.label`
/// caption drifts upward beside a pill. If a future egui teaches `horizontal_wrapped`
/// to centre its items this test will fail — at which point `RowCaption` may be
/// redundant, and that is worth being told about rather than silently carrying it.
#[test]
fn plain_label_caption_is_the_thing_that_misaligns() {
    let img = render_row(false);
    let (_, cap_bottom) = ink_rows(&img, CAPTION_X.0, CAPTION_X.1);
    let (_, pill_bottom) = ink_rows(&img, PILL_TEXT_X.0, PILL_TEXT_X.1);
    assert!(
        cap_bottom < pill_bottom,
        "expected the plain-label caption to sit high (that is the bug RowCaption \
         exists for); got caption bottom {cap_bottom}, pill bottom {pill_bottom}"
    );
}
