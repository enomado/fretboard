//! Throwaway diagnostic behind `ui::segmented`'s baseline rule: prints the EXACT
//! font metrics (`Glyph::pos` = baseline, `Glyph::font_ascent`, `font_height`)
//! rather than atlas-quantised `mesh_bounds`, so the centring rule is derived
//! from measurements instead of assumed ascent/descent.
//! Run: `cargo run --bin font_probe`.

use eframe::egui::{
    self,
    Color32,
    FontId,
};

/// Exact, un-quantised vertical metrics of the proportional font at `size`, as
/// egui itself lays text out. `baseline`/`ascent`/`height` come straight from the
/// glyph record; `ink_top`/`ink_bottom` come from the rasterised quad
/// (`uv_rect`), which is the only way to see cap-height / descender depth.
struct Metrics {
    baseline:   f32,
    ascent:     f32,
    height:     f32,
    ink_top:    f32,
    ink_bottom: f32,
}

fn metrics(ctx: &egui::Context, text: &str, size: f32) -> Metrics {
    ctx.fonts_mut(|f| {
        let g = f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE);
        let row = &g.rows[0];
        let first = &row.glyphs[0];

        // uv_rect.offset is the ink's top-left relative to the glyph's baseline
        // pos; add size to reach the ink bottom. Fold over every glyph so the
        // extremes cover ascenders and descenders in the whole string.
        let mut ink_top = f32::INFINITY;
        let mut ink_bottom = f32::NEG_INFINITY;
        for glyph in &row.glyphs {
            if glyph.uv_rect.size == egui::Vec2::ZERO {
                continue; // whitespace has no ink
            }
            let top = glyph.pos.y + glyph.uv_rect.offset.y;
            ink_top = ink_top.min(top);
            ink_bottom = ink_bottom.max(top + glyph.uv_rect.size.y);
        }

        Metrics {
            baseline: first.pos.y,
            ascent: first.font_ascent,
            height: first.font_height,
            ink_top,
            ink_bottom,
        }
    })
}

fn main() {
    let ctx = egui::Context::default();
    fretboard::ui::theme::install_fonts(&ctx);
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});

    let m = metrics(&ctx, "X", 14.0);
    println!("=== EXACT metrics, proportional 14.0 ===");
    println!("font_ascent = {:.4}", m.ascent);
    println!("font_height = {:.4}", m.height);
    println!("baseline(pos.y) = {:.4}", m.baseline);
    println!("=> descent = height - ascent = {:.4}", m.height - m.ascent);

    println!("\n=== ink extents (uv_rect, per string) ===");
    println!(
        "{:<14} {:>10} {:>10} {:>10}",
        "text", "ink_top", "ink_bot", "ink_mid"
    );
    for s in [
        "X",
        "x",
        "Microphone",
        "System",
        "Power",
        "Monitor on",
        "C",
        "Magnitude",
        "Flats (Db)",
        "Xhgjpqy",
    ] {
        let m = metrics(&ctx, s, 14.0);
        println!(
            "{s:<14} {:>10.4} {:>10.4} {:>10.4}",
            m.ink_top,
            m.ink_bottom,
            (m.ink_top + m.ink_bottom) * 0.5
        );
    }

    // The three candidate rules, expressed as "where does the galley origin go
    // inside a 28 px pill", using EXACT metrics.
    const H: f32 = 28.0;
    let x = metrics(&ctx, "X", 14.0);
    let ink = metrics(&ctx, "Xhgjpqy", 14.0);
    let caps_mid = (x.ink_top + x.ink_bottom) * 0.5;
    let ink_mid = (ink.ink_top + ink.ink_bottom) * 0.5;
    let box_mid = x.height * 0.5;
    println!("\n=== rule midpoints (exact) ===");
    println!("box_mid  = {box_mid:.4}  (what egui::Button centres)");
    println!("caps_mid = {caps_mid:.4}");
    println!("ink_mid  = {ink_mid:.4}");
    println!("split    = {:.4}", (caps_mid + ink_mid) * 0.5);
    println!("\norigin_y in a {H} px pill:");
    for (name, mid) in [
        ("box/egui", box_mid),
        ("caps", caps_mid),
        ("ink", ink_mid),
        ("split", (caps_mid + ink_mid) * 0.5),
    ] {
        println!("  {name:<9} origin_y = {:.4}", H * 0.5 - mid);
    }

    // THE verification that matters: epaint rounds every galley origin to a whole
    // physical pixel, so a rule change only shows up on screen if it survives that
    // rounding. Anything printing "same" below would be an invisible no-op.
    println!("\n=== rendered origin AFTER epaint's pixel rounding ===");
    println!(
        "{:>5} {:>12} {:>12} {:>12} {:>10}",
        "ppp", "box(before)", "split", "ink(now)", "moved?"
    );
    for ppp in [1.0_f32, 1.25, 1.5, 1.75, 2.0] {
        let snap = |v: f32| (v * ppp).round() / ppp;
        let before = snap(H * 0.5 - box_mid);
        let split = snap(H * 0.5 - (caps_mid + ink_mid) * 0.5);
        let now = snap(H * 0.5 - ink_mid);
        let moved = (before - now) * ppp; // in physical pixels
        println!(
            "{ppp:>5.2} {before:>12.3} {split:>12.3} {now:>12.3} {:>10}",
            if moved.abs() < 0.01 {
                "SAME (!)".to_owned()
            } else {
                format!("{moved:+.0} px up")
            }
        );
    }
}
