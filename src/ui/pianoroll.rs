//! Pitch-roll (horizontal "waterfall table") rendering.
//!
//! A piano-roll style view: the vertical axis is **pitch**, one row per semitone
//! labelled with its note name (the "table"), and time flows **right → left** with
//! the newest sample pinned at the right edge. Two layers sit on that grid:
//!
//!   1. a **spectral heat** field — the resonator bank's per-column energy painted
//!      at each bin's own pitch. It makes *no* single-pitch decision, so there is
//!      no octave error to make: a strong overtone simply shows as a fainter cell
//!      an octave up (physically true), the fundamental as the bright low cell.
//!      Fast (per bank column) → trills and vibrato appear at full resolution.
//!   2. a **melody line** — the fused pYIN pitch, octave-stable and smoothed,
//!      coloured by intonation. It reads the melody as one clean curve on top.
//!
//! The heat is the ground truth; the line is a guide. Where the line's smoothing
//! or a rare octave slip disagrees with reality, the heat underneath shows it.
//!
//! The left gutter names every row; the **right** gutter repeats the scale but
//! colours each name by the energy at that pitch in the current heat column — a
//! live per-note meter of what is sounding right now at the playhead.
//!
//! Pure renderer: it owns no state and paints from borrowed slices over a `Rect`.
//! The rolling samples/heat and the eased view window live in
//! [`crate::app::pitch_roll_panel`].

use eframe::egui::epaint::Vertex;
use eframe::egui::{
    self,
    Align2,
    Color32,
    FontId,
    Mesh,
    Painter,
    Pos2,
    Rect,
    Shape,
    Stroke,
    pos2,
};

use crate::core_types::note::AccidentalStyle;
use crate::ui::theme::intonation_color;
use crate::ui::tokens::color;

/// One frame of continuous detected pitch: `midi_f` is the fractional MIDI number
/// (integer part = note, fraction = how sharp/flat), `level` the input level 0..1
/// that fades the graph so quiet playing reads faint.
#[derive(Clone, Copy)]
pub struct PitchPoint {
    pub midi_f: f32,
    pub level:  f32,
}

/// Width of the left gutter that carries the note-name labels, in pixels.
const LABEL_W: f32 = 46.0;
/// Width of the right gutter, a second note scale coloured by the *current* heat
/// energy at each pitch — a live "which notes are sounding now" readout at the
/// playhead edge.
const RIGHT_LABEL_W: f32 = 44.0;

/// The octave-anchor C label. Brighter than the [`color::TEXT_MUTED`] rows around
/// it because it is the one row you navigate by — at a zoomed-out view it is the
/// *only* row still labelled (see [`LABEL_MIN_ROW_H`]).
const LABEL_OCTAVE: Color32 = Color32::from_rgb(196, 202, 211);
/// The "now" edge. File-local rather than a token: only this panel has a
/// playhead, and `ui::tokens` is the vocabulary for chrome shared across modules.
const PLAYHEAD: Color32 = Color32::from_rgb(70, 76, 86);
/// Shading over the accidental rows, so the grid reads like a piano's black keys.
/// A translucent black rather than a colour: it must darken whatever heat is
/// already painted under it, not replace it.
const ACCIDENTAL_ROW_SHADE: Color32 = Color32::from_black_alpha(46);

/// Paint the pitch roll into `rect`.
///
/// `samples` are per-frame melody-line pitches oldest → newest (`None` = a silent
/// frame → a gap in the line). `heat` is the aligned per-frame resonator column
/// (an *empty* `Vec` marks a silent frame); `res_min_midi`/`res_max_midi` are the
/// bank's pitch range, together mapping each heat bin to a pitch. `view_lo`/`view_hi`
/// are the fractional MIDI numbers at the bottom/top edges of the plot (the eased
/// auto-framing window), so a rising melody scrolls the rows smoothly.
#[allow(clippy::too_many_arguments)]
pub fn draw_pitch_roll(
    painter: &Painter,
    rect: Rect,
    samples: &[Option<PitchPoint>],
    heat: &[Vec<f32>],
    res_min_midi: i32,
    res_max_midi: i32,
    view_lo: f32,
    view_hi: f32,
    style: AccidentalStyle,
) {
    // Plot sits between the two label gutters (names left, live heat scale right).
    let plot = Rect::from_min_max(
        pos2(rect.left() + LABEL_W, rect.top()),
        pos2(rect.right() - RIGHT_LABEL_W, rect.bottom()),
    );
    let span = (view_hi - view_lo).max(1.0);
    // Pitch → y: higher pitch is higher on screen (smaller y).
    let y_of = |midi_f: f32| plot.bottom() - (midi_f - view_lo) / span * plot.height();

    painter.rect_filled(plot, 0.0, color::HEAT_BG);

    // Layer order: grid rows (bottom) → spectral heat → melody line (top).
    draw_rows(painter, rect, plot, view_lo, view_hi, span, &y_of, style);
    draw_heat(painter, plot, span, heat, res_min_midi, res_max_midi, &y_of);
    draw_graph(painter, plot, samples, &y_of);
    draw_right_scale(
        painter,
        plot,
        view_lo,
        view_hi,
        span,
        &y_of,
        style,
        heat,
        res_min_midi,
        res_max_midi,
    );
}

/// The right-hand note scale: the same rows as the left gutter, but each label is
/// coloured by the energy at that pitch in the **current** (newest) heat column —
/// dim when nothing sounds there, bright cyan when it does. Sitting at the playhead
/// edge, it reads as a live per-note level meter of what is playing right now.
#[allow(clippy::too_many_arguments)]
fn draw_right_scale(
    painter: &Painter,
    plot: Rect,
    view_lo: f32,
    view_hi: f32,
    span: f32,
    y_of: &impl Fn(f32) -> f32,
    style: AccidentalStyle,
    heat: &[Vec<f32>],
    res_min_midi: i32,
    res_max_midi: i32,
) {
    // "Now" = the newest *non-empty* column, so a one-frame dropout doesn't blink
    // the whole scale dark.
    let current = heat.iter().rev().find(|c| !c.is_empty());
    let bin_count = current.map_or(0, Vec::len);
    let mapped = bin_count >= 2 && res_max_midi > res_min_midi;
    let bins_per_semitone = if mapped {
        (bin_count - 1) as f32 / (res_max_midi - res_min_midi) as f32
    } else {
        1.0
    };

    let row_h = plot.height() / span;
    let lo = view_lo.floor() as i32;
    let hi = view_hi.ceil() as i32;
    let x = plot.right() + 6.0;

    for midi in lo..=hi {
        let pc = midi.rem_euclid(12);
        // Mirror the left gutter's density: every row when tall enough, else only C.
        if row_h < 9.0 && pc != 0 {
            continue;
        }
        // Peak energy within ±½ semitone of this note in the current column.
        let energy = match current {
            Some(col) if mapped => {
                let center = (midi as f32 - res_min_midi as f32) * bins_per_semitone;
                let last = (col.len() - 1) as f32;
                let b0 = (center - bins_per_semitone * 0.5).clamp(0.0, last) as usize;
                let b1 = (center + bins_per_semitone * 0.5).clamp(0.0, last) as usize;
                col[b0..=b1].iter().copied().fold(0.0, f32::max)
            }
            _ => 0.0,
        };
        let size = if pc == 0 {
            (row_h * 0.72).clamp(9.0, 13.0)
        } else {
            (row_h * 0.68).clamp(8.0, 12.0)
        };
        painter.text(
            pos2(x, y_of(midi as f32)),
            Align2::LEFT_CENTER,
            style.midi_name(midi),
            FontId::proportional(size),
            label_heat_color(energy),
        );
    }
}

/// Right-scale label colour: muted grey-blue with no energy → bright cyan when the
/// note is sounding. `powf(0.6)` lifts quiet energy so a soft note still shows.
fn label_heat_color(energy: f32) -> Color32 {
    let t = energy.clamp(0.0, 1.0).powf(0.6);
    let r = (100.0 + 22.0 * t).round() as u8;
    let g = (106.0 + 100.0 * t).round() as u8;
    let b = (116.0 + 124.0 * t).round() as u8;
    Color32::from_rgb(r, g, b)
}

/// The pitch grid: one horizontal band per semitone. Accidental ("black-key")
/// rows are shaded darker so the octave shape reads at a glance (like a piano
/// roll), C-boundaries are drawn a touch brighter, and every row wide enough to
/// fit a label (plus every C regardless) is named in the left gutter.
fn draw_rows(
    painter: &Painter,
    rect: Rect,
    plot: Rect,
    view_lo: f32,
    view_hi: f32,
    span: f32,
    y_of: &impl Fn(f32) -> f32,
    style: AccidentalStyle,
) {
    // Semitones per pixel is `span / height`; invert for pixels per semitone, which
    // decides whether a per-row label fits.
    let row_h = plot.height() / span;
    let lo = view_lo.floor() as i32;
    let hi = view_hi.ceil() as i32;

    for midi in lo..=hi {
        let center_y = y_of(midi as f32);
        let top_y = y_of(midi as f32 + 0.5);
        let bottom_y = y_of(midi as f32 - 0.5);
        let pc = midi.rem_euclid(12);
        // Black keys on a piano: C#, D#, F#, G#, A#.
        let is_accidental = matches!(pc, 1 | 3 | 6 | 8 | 10);

        if is_accidental {
            painter.rect_filled(
                Rect::from_min_max(pos2(plot.left(), top_y), pos2(plot.right(), bottom_y)),
                0.0,
                ACCIDENTAL_ROW_SHADE,
            );
        }

        // Row boundary line; the C boundary (octave) is brighter as a landmark.
        let line_col = if pc == 0 {
            color::GRID_LINE_STRONG
        } else {
            color::GRID_LINE
        };
        painter.line_segment(
            [pos2(plot.left(), bottom_y), pos2(plot.right(), bottom_y)],
            Stroke::new(1.0, line_col),
        );

        // Label the row when it is tall enough to read; always label C so there is
        // an octave anchor even in a zoomed-out (many-row) view.
        if row_h >= LABEL_MIN_ROW_H || pc == 0 {
            let (color, size) = if pc == 0 {
                (LABEL_OCTAVE, (row_h * 0.72).clamp(9.0, 13.0))
            } else {
                (color::TEXT_MUTED, (row_h * 0.68).clamp(8.0, 12.0))
            };
            painter.text(
                pos2(rect.left() + LABEL_W - 6.0, center_y),
                Align2::RIGHT_CENTER,
                style.midi_name(midi),
                FontId::proportional(size),
                color,
            );
        }
    }
}

/// Row height (px) below which a note name no longer fits, so only the C rows stay
/// labelled.
///
/// This is what couples the *framing* to what the panel actually reads like: rows are
/// `plot_height / span` tall, so an over-wide view silently degrades the grid to
/// octave landmarks only. Framing that keeps the span under `plot_height / this` is
/// what keeps every note named — see `app::pitch_roll_panel::reframe`.
pub(crate) const LABEL_MIN_ROW_H: f32 = 9.0;

/// Below this normalized magnitude a heat bin is treated as noise and skipped —
/// keeps the field to a note + its partials (not a wash) and the mesh small.
const HEAT_GATE: f32 = 0.16;

/// The spectral heat field: each frame's resonator column painted at every bin's
/// own pitch height. Columns march left from the playhead exactly like the melody
/// line (same per-frame cadence → the two layers align in time). An *empty* column
/// is a silent frame (gated out by the panel) → nothing painted, a clean gap.
///
/// All cells go into one [`Mesh`] (a single draw call) instead of thousands of
/// `rect_filled` shapes — the whole grid is visible here (unlike the staff, which
/// clips to five lines), so the cell count is high.
///
/// Bin → pitch: the bank spans `res_min_midi..=res_max_midi`; bins-per-semitone is
/// derived from a column's length (it changes with the reassignment toggle, so we
/// read it rather than assume it), the same way the staff's waterfall does.
fn draw_heat(
    painter: &Painter,
    plot: Rect,
    span: f32,
    heat: &[Vec<f32>],
    res_min_midi: i32,
    res_max_midi: i32,
    y_of: &impl Fn(f32) -> f32,
) {
    let bin_count = heat.iter().find(|c| !c.is_empty()).map_or(0, Vec::len);
    if bin_count < 2 || res_max_midi <= res_min_midi {
        return;
    }
    let bins_per_semitone = (bin_count - 1) as f32 / (res_max_midi - res_min_midi) as f32;
    let n = heat.len();
    let dx = plot.width() / n.max(1) as f32;
    // A cell spans one column in x and one bin in y, with slight overdraw to close
    // hairline seams at fractional sizes.
    let cell_w = dx + 0.6;
    let px_per_semitone = span.recip() * plot.height();
    let cell_h = (px_per_semitone / bins_per_semitone + 0.6).max(1.5);

    let mut mesh = Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    for (i, col) in heat.iter().enumerate() {
        if col.is_empty() {
            continue; // silent frame → gap
        }
        let age = (n - 1 - i) as f32; // 0 = newest, at the playhead
        let cx = plot.right() - age * dx;
        let recency = 1.0 - age / n as f32; // older columns fade
        for (bin, &value) in col.iter().enumerate() {
            if value < HEAT_GATE {
                continue;
            }
            let midi = res_min_midi as f32 + bin as f32 / bins_per_semitone;
            let cy = y_of(midi);
            if cy < plot.top() || cy > plot.bottom() {
                continue; // off the visible pitch window
            }
            let color = heat_color(value, recency);
            let (x0, x1) = (cx - cell_w * 0.5, cx + cell_w * 0.5);
            let (y0, y1) = (cy - cell_h * 0.5, cy + cell_h * 0.5);
            let base = mesh.vertices.len() as u32;
            for pos in [pos2(x0, y0), pos2(x1, y0), pos2(x1, y1), pos2(x0, y1)] {
                mesh.vertices.push(Vertex { pos, uv, color });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Heat cell colour: a cool blue ramp that brightens with magnitude, dimmed with
/// age. Cool + magnitude-scaled so the warm intonation *line* reads clearly on top.
fn heat_color(value: f32, recency: f32) -> Color32 {
    let v = value.clamp(0.0, 1.0);
    let f = (0.4 + recency * 0.6) * v;
    let r = (30.0 + 70.0 * f).round() as u8;
    let g = (54.0 + 130.0 * f).round() as u8;
    let b = (72.0 + 150.0 * f).round() as u8;
    let a = (40.0 + 200.0 * f).clamp(0.0, 245.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// The played-pitch graph: consecutive non-silent samples joined into a line that
/// flows in from the left and ends at the playhead (right edge). Each segment is
/// coloured by that sample's intonation and faded by its level; a silent frame
/// breaks the line so rests show as gaps.
fn draw_graph(painter: &Painter, plot: Rect, samples: &[Option<PitchPoint>], y_of: &impl Fn(f32) -> f32) {
    if !samples.iter().any(Option::is_some) {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "play a note…",
            FontId::proportional(14.0),
            color::TEXT_MUTED,
        );
        return;
    }

    let n = samples.len();
    // Newest sample sits at the right edge; older ones step left by `dx` per frame,
    // so the whole buffer fills the plot width exactly.
    let dx = plot.width() / n.max(1) as f32;

    let mut prev: Option<Pos2> = None;
    for (i, sample) in samples.iter().enumerate() {
        let Some(point) = sample else {
            prev = None; // silence → break the line
            continue;
        };
        let age = (n - 1 - i) as f32; // 0 = newest
        let x = plot.right() - age * dx;
        // Clamp to the plot so a note briefly outside the eased window rides the
        // edge instead of drawing off into space; the window normally keeps it in.
        let y = y_of(point.midi_f).clamp(plot.top(), plot.bottom());
        let here = pos2(x, y);

        let cents = (point.midi_f - point.midi_f.round()) * 100.0;
        let base = intonation_color(cents);
        // Louder → more opaque, so dynamics read in the trail; sqrt lifts quiet notes.
        let alpha = (60.0 + point.level.clamp(0.0, 1.0).sqrt() * 195.0).clamp(0.0, 255.0) as u8;
        let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);

        if let Some(from) = prev {
            painter.line_segment([from, here], Stroke::new(2.0, color));
        }
        painter.circle_filled(here, 1.8, color);
        prev = Some(here);
    }

    // Playhead: a faint vertical marker at the "now" edge.
    painter.line_segment(
        [
            pos2(plot.right() - 0.5, plot.top()),
            pos2(plot.right() - 0.5, plot.bottom()),
        ],
        Stroke::new(1.0, PLAYHEAD),
    );
}
