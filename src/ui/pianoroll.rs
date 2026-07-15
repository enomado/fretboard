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
//! **Time is the x axis, literally.** Each column carries its own age in seconds
//! ([`RollColumn::age_s`]) and is placed by it, so the ruler across the bottom means
//! what it says and a column the engine never published leaves a gap of exactly its
//! own width. Columns used to be laid out by *index* — evenly spaced, whatever they
//! were — which quietly made the axis a count of renderer ticks.
//!
//! **Which slice of time is on screen is a parameter** ([`TimeAxis`]), not this
//! module's opinion. It used to be one constant, `VISIBLE_SECONDS`, and two panels
//! now need two different answers: the live roll's ten seconds ending at the playhead,
//! and a recorded take read whole, by scrolling and zooming into it.
//!
//! Pure renderer: it owns no state and paints from borrowed slices over a `Rect`.
//! The history and both view windows live in the panels
//! ([`crate::app::pitch_roll_panel`], [`crate::app::take_roll`]).

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

/// One published bank frame, ready to draw: how long ago it sounded, what the melody
/// line was, and the spectral heat of that same instant.
///
/// The two layers ride one column because the heat is only useful as the *ground
/// truth the line is checked against* — a heat column from a neighbouring instant
/// would not disagree with the line honestly, it would just be misaligned. Keeping
/// them in one record is what makes that impossible rather than merely intended.
pub struct RollColumn<'a> {
    /// Seconds between this column and the playhead, ≥ 0 — the x position.
    pub age_s: f32,
    /// The melody line's pitch here, or `None` for silence *or* a rejected octave
    /// slip (the engine decides both; the renderer draws a gap for either).
    pub pitch: Option<PitchPoint>,
    /// The bank's energy per bin. **Empty** = draw nothing (a rest).
    pub heat:  &'a [f32],
}

/// Which slice of time the plot shows, how its ruler counts, and whether the past
/// fades.
///
/// All four of those used to be one constant (`VISIBLE_SECONDS = 10 s`) baked into the
/// x mapping, the ruler, the cell width and the fade. That is exactly right for the
/// live roll — a fixed span ending at the playhead — and it is why a frozen take could
/// not be drawn at all: a take runs up to 35 s and is read by scrolling and zooming
/// *into* it, so the span is a variable and "now" is usually not even on screen.
#[derive(Clone, Copy)]
pub struct TimeAxis {
    /// Age (seconds before the playhead) at the plot's **right** edge. `0.0` for the
    /// live roll, whose right edge *is* the playhead. May be negative on a frozen take
    /// still playing: the window covers time the line has not reached yet.
    pub near_s:     f32,
    /// Age (seconds) at the plot's **left** edge. `far_s - near_s` is the visible span,
    /// and therefore what one pixel is worth.
    pub far_s:      f32,
    /// The age the ruler calls zero. A column is labelled `zero_age_s - age_s`, which is
    /// one formula for both rolls because both are asking "how far is this column from
    /// the point I count from":
    ///
    /// - live: `0.0` — the zero is the playhead, so columns read `-2s`, i.e. two
    ///   seconds ago;
    /// - frozen: the age of the take's *start*, so columns read `3.5s`, i.e. three and
    ///   a half seconds **into the take** — which is how a player says where the slip
    ///   was, and is stable while the take plays and the playhead moves.
    pub zero_age_s: f32,
    /// See [`RollMode`].
    pub mode:       RollMode,
}

impl TimeAxis {
    /// The visible span in seconds, never zero — a pixel must always be worth
    /// something, however far the user zooms in.
    fn span_s(self) -> f32 {
        (self.far_s - self.near_s).max(1e-3)
    }
}

/// Whether the picture has a **now** in it.
///
/// Two things follow from that one question, and they look unrelated only until it is
/// asked: whether the heat fades into the past, and whether an empty plot invites you
/// to play a note. Both are about a live instrument being on the other end.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RollMode {
    /// The live waterfall: the right edge is the playhead, and the heat fades toward
    /// the far edge — the comet tail that makes the newest frame findable without
    /// looking for it. An empty plot is an invitation: play something.
    Live,
    /// A frozen take, read by scrolling and zooming. Every column is painted at full
    /// strength: a take has no playhead to be near, so a fade could only mean "further
    /// left", and the same audio would then change brightness as it was scrolled —
    /// the one thing evidence must not do. An empty stretch is not an invitation
    /// either; it is a rest, and that is an answer.
    Frozen,
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
/// The time ruler's lines and labels. File-local, like [`PLAYHEAD`], and for the same
/// reason: only this panel has a time axis, and `ui::tokens` is the vocabulary for
/// chrome that is spoken in more than one module. Deliberately dim — the ruler is for
/// reading the melody against, not for looking at.
const TIME_GRID_LINE: Color32 = Color32::from_rgb(46, 51, 59);
const TIME_GRID_LABEL: Color32 = Color32::from_rgb(84, 90, 100);

/// Where the plot itself sits inside `rect` — between the two label gutters (names
/// left, live heat scale right).
///
/// Public because a panel that lets the user *scroll* the axis has to turn a pointer
/// position into a time, and that means knowing where the axis starts. Deriving it
/// there from `LABEL_W` would be a second opinion about the layout, and the day the
/// gutter changed width the two would disagree by exactly that much.
pub fn plot_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        pos2(rect.left() + LABEL_W, rect.top()),
        pos2(rect.right() - RIGHT_LABEL_W, rect.bottom()),
    )
}

/// Paint the pitch roll into `rect`.
///
/// `columns` are the published bank frames oldest → newest, each carrying its own
/// age, the line's pitch and the heat of that instant. `res_min_midi`/`res_max_midi`
/// are the bank's pitch range, which maps each heat bin to a pitch. `view_lo`/
/// `view_hi` are the fractional MIDI numbers at the bottom/top edges of the plot, so a
/// rising melody scrolls the rows smoothly. `time` is the slice of time on screen —
/// see [`TimeAxis`].
///
/// Columns outside `time` are the caller's to leave out: it knows the cadence, so it
/// can keep the one column either side that the line needs to enter and leave the
/// edges, which this function could only guess at.
#[allow(clippy::too_many_arguments)]
pub fn draw_pitch_roll(
    painter: &Painter,
    rect: Rect,
    columns: &[RollColumn<'_>],
    res_min_midi: i32,
    res_max_midi: i32,
    view_lo: f32,
    view_hi: f32,
    time: TimeAxis,
    style: AccidentalStyle,
) {
    let plot = plot_rect(rect);
    let span = (view_hi - view_lo).max(1.0);
    // Pitch → y: higher pitch is higher on screen (smaller y).
    let y_of = |midi_f: f32| plot.bottom() - (midi_f - view_lo) / span * plot.height();
    // Age → x: the right edge is the newest instant on screen (`near_s` — the playhead
    // itself on the live roll), the past runs left at a fixed number of pixels per
    // second. This is the whole axis fix — a column lands where its *time* says, not
    // where its position in the buffer says.
    let px_per_second = plot.width() / time.span_s();
    let x_of = |age_s: f32| plot.right() - (age_s - time.near_s) * px_per_second;

    painter.rect_filled(plot, 0.0, color::HEAT_BG);

    // Layer order: grid rows + time ruler (bottom) → spectral heat → melody line.
    draw_rows(painter, rect, plot, view_lo, view_hi, span, &y_of, style);
    draw_time_grid(painter, plot, time, &x_of);
    draw_heat(
        painter,
        plot,
        span,
        columns,
        res_min_midi,
        res_max_midi,
        time,
        px_per_second,
        &y_of,
        &x_of,
    );
    draw_graph(painter, plot, columns, time, &y_of, &x_of);
    draw_right_scale(
        painter,
        plot,
        view_lo,
        view_hi,
        span,
        &y_of,
        style,
        columns,
        res_min_midi,
        res_max_midi,
    );
}

/// Roughly how many gridlines the ruler aims for across the plot. The step is then
/// rounded *up* to a readable one, so this is a ceiling on the density, not a promise.
const TIME_GRID_TARGET_LINES: f32 = 5.0;

/// The ruler's step in seconds for a visible span: 1, 2 or 5 times a power of ten.
///
/// A fixed step cannot survive a zoom — 2 s draws nothing across a 0.4 s window and a
/// picket fence across 35 s. Rounding to 1-2-5 is what keeps the labels readable
/// (0.5, 1.0, 1.5 — never 0.43, 0.86): people read time in those steps, and a ruler is
/// only useful if a glance converts it to a number.
///
/// The live roll's 10 s span comes out at exactly 2 s — the constant this replaced —
/// so making the axis a variable did not move the ruler it already had.
fn tick_step_s(span_s: f32) -> f32 {
    let target = span_s / TIME_GRID_TARGET_LINES;
    // Split the target into a power of ten and a 1..10 mantissa, then snap the
    // mantissa to the nearest readable one. The bounds are the midpoints on a log
    // scale, so a span drifts to the nearer step rather than the smaller one.
    let magnitude = 10f32.powf(target.log10().floor());
    let mantissa = target / magnitude;
    let snapped = if mantissa < 1.5 {
        1.0
    } else if mantissa < 3.5 {
        2.0
    } else if mantissa < 7.5 {
        5.0
    } else {
        10.0
    };
    magnitude * snapped
}

/// The time ruler: a labelled vertical line every [`tick_step_s`] seconds.
///
/// It is what makes the axis checkable by eye rather than by argument — the thing this
/// panel spent its whole life not having. A trill's rate, a note's length and a bow
/// change all become measurable off the screen instead of inferred.
///
/// The lines are placed on round **ruler** values rather than round ages, which is
/// what makes them stand still while a frozen take is scrolled under them: you read
/// 2.0, 2.5, 3.0, not 2.13, 2.63, 3.13 sliding along with the drag.
fn draw_time_grid(painter: &Painter, plot: Rect, time: TimeAxis, x_of: &impl Fn(f32) -> f32) {
    let step = tick_step_s(time.span_s());
    // Ruler value ↔ age is `label = zero_age_s - age`, so walk the labels the window
    // spans and convert each back to the age that places it. The left edge is the older
    // one, hence the lower label.
    let label_lo = time.zero_age_s - time.far_s;
    let label_hi = time.zero_age_s - time.near_s;
    // Decimals from the step: a 0.5 s ruler reading "2s, 2s, 3s" is worse than none.
    let decimals = if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    };

    let mut tick = (label_lo / step).ceil();
    while tick * step <= label_hi {
        let label = tick * step;
        let x = x_of(time.zero_age_s - label);
        tick += 1.0;
        // A line on the right edge has no room for its label — it would be painted over
        // the note scale in the gutter. On the live roll that edge is also the playhead,
        // which is already marked and whose "-0s" would say nothing anyway.
        if x < plot.left() || x > plot.right() - 1.0 {
            continue;
        }
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, TIME_GRID_LINE),
        );
        painter.text(
            pos2(x + 3.0, plot.bottom() - 3.0),
            Align2::LEFT_BOTTOM,
            format!("{label:.decimals$}s"),
            FontId::proportional(9.0),
            TIME_GRID_LABEL,
        );
    }
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
    columns: &[RollColumn<'_>],
    res_min_midi: i32,
    res_max_midi: i32,
) {
    // "Now" = the newest *non-empty* column, so a one-frame dropout doesn't blink
    // the whole scale dark.
    let current = columns.iter().rev().map(|c| c.heat).find(|c| !c.is_empty());
    let bin_count = current.map_or(0, <[f32]>::len);
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

/// The mean gap between the given columns, in seconds — how long one frame lasts.
///
/// Two layers need it and need to agree: it is a heat cell's width, and it is the
/// distance beyond which the line breaks rather than guessing across a hole. Measured
/// rather than assumed, because the bank's cadence is a user setting (8..80 ms) and
/// the publish jitters around it anyway.
///
/// Measured across **the columns handed in**, not from `columns[0].age_s` alone —
/// that older form quietly assumed the newest column sits at the playhead, which is
/// true only while nobody culls to a window. A frozen take scrolled to its middle has
/// no column at age 0 at all, and the assumption would have inflated the gap by the
/// scroll distance: every cell a mile wide and the line joined across real holes.
fn mean_gap_s(columns: &[RollColumn<'_>]) -> f32 {
    let (Some(oldest), Some(newest)) = (columns.first(), columns.last()) else {
        return 0.0;
    };
    match columns.len() {
        0 | 1 => 0.0,
        n => (oldest.age_s - newest.age_s) / (n - 1) as f32,
    }
}

/// The spectral heat field: each frame's resonator column painted at every bin's
/// own pitch height, at the x its own age puts it. An *empty* column is a silent
/// frame (gated out by the panel) → nothing painted, a clean gap.
///
/// All cells go into one [`Mesh`] (a single draw call) instead of thousands of
/// `rect_filled` shapes — the whole grid is visible here (unlike the staff, which
/// clips to five lines), so the cell count is high.
///
/// Bin → pitch: the bank spans `res_min_midi..=res_max_midi`; bins-per-semitone is
/// derived from a column's length (it changes with the reassignment toggle, so we
/// read it rather than assume it), the same way the staff's waterfall does.
#[allow(clippy::too_many_arguments)]
fn draw_heat(
    painter: &Painter,
    plot: Rect,
    span: f32,
    columns: &[RollColumn<'_>],
    res_min_midi: i32,
    res_max_midi: i32,
    time: TimeAxis,
    px_per_second: f32,
    y_of: &impl Fn(f32) -> f32,
    x_of: &impl Fn(f32) -> f32,
) {
    let bin_count = columns
        .iter()
        .map(|c| c.heat)
        .find(|c| !c.is_empty())
        .map_or(0, <[f32]>::len);
    if bin_count < 2 || res_max_midi <= res_min_midi {
        return;
    }
    let bins_per_semitone = (bin_count - 1) as f32 / (res_max_midi - res_min_midi) as f32;
    // A cell is one *frame*, so it is as wide as the gap between frames — measured,
    // not assumed: the bank's cadence is a user setting (8..80 ms) and the publish
    // jitters around it anyway. The mean is robust enough here (a dropped frame moves
    // it by one 600th) and, crucially, keeps a cell an *instant* rather than an
    // interval: a frame the engine never published then leaves a hole exactly its own
    // width, instead of its neighbour smearing across the missing time. +0.6 closes
    // the hairline seams fractional sizes leave.
    let cell_w = mean_gap_s(columns) * px_per_second + 0.6;
    let px_per_semitone = span.recip() * plot.height();
    let cell_h = (px_per_semitone / bins_per_semitone + 0.6).max(1.5);

    let mut mesh = Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    for column in columns {
        if column.heat.is_empty() {
            continue; // silent frame → gap
        }
        let cx = x_of(column.age_s);
        // Older columns fade toward the far edge — but only where there is a "now" to
        // fade away from (see [`RollMode`]). Keyed off time, so the fade is a real
        // half-life rather than "how much of the buffer ago" — the same reading
        // regardless of cadence.
        let recency = match time.mode {
            RollMode::Live => 1.0 - ((column.age_s - time.near_s) / time.span_s()).clamp(0.0, 1.0),
            RollMode::Frozen => 1.0,
        };
        for (bin, &value) in column.heat.iter().enumerate() {
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

/// The played-pitch graph: consecutive non-silent frames joined into a line that
/// flows in from the left and ends at the playhead (right edge). Each segment is
/// coloured by that frame's intonation and faded by its level; a silent frame
/// breaks the line so rests show as gaps.
fn draw_graph(
    painter: &Painter,
    plot: Rect,
    columns: &[RollColumn<'_>],
    time: TimeAxis,
    y_of: &impl Fn(f32) -> f32,
    x_of: &impl Fn(f32) -> f32,
) {
    if !columns.iter().any(|c| c.pitch.is_some()) {
        // Nothing to draw. On the live roll that is an invitation; on a frozen take it
        // is a rest, and a rest is an answer — printing "play a note…" over a stretch
        // of a recording the user is reading would be telling them their evidence is
        // missing when it is the evidence itself.
        if time.mode == RollMode::Live {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                "play a note…",
                FontId::proportional(14.0),
                color::TEXT_MUTED,
            );
        }
        return;
    }

    // A stretch of missing frames is not a straight glide between the two frames that
    // bracket it — we simply do not know what happened in there. So the line breaks
    // over a gap much longer than the cadence, exactly as it does over silence: the
    // panel would rather show a hole than invent a segment. 3× is loose enough to
    // survive the publish jitter and tight enough to catch a real drop.
    let max_join_s = mean_gap_s(columns) * 3.0;

    let mut prev: Option<(Pos2, f32)> = None;
    for column in columns {
        let Some(point) = column.pitch else {
            prev = None; // silence (or a rejected slip) → break the line
            continue;
        };
        let x = x_of(column.age_s);
        // Clamp to the plot so a note briefly outside the eased window rides the
        // edge instead of drawing off into space; the window normally keeps it in.
        let y = y_of(point.midi_f).clamp(plot.top(), plot.bottom());
        let here = pos2(x, y);

        let cents = (point.midi_f - point.midi_f.round()) * 100.0;
        let base = intonation_color(cents);
        // Louder → more opaque, so dynamics read in the trail; sqrt lifts quiet notes.
        let alpha = (60.0 + point.level.clamp(0.0, 1.0).sqrt() * 195.0).clamp(0.0, 255.0) as u8;
        let color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);

        if let Some((from, from_age)) = prev
            && from_age - column.age_s <= max_join_s
        {
            painter.line_segment([from, here], Stroke::new(2.0, color));
        }
        painter.circle_filled(here, 1.8, color);
        prev = Some((here, column.age_s));
    }

    // Playhead: a faint vertical marker at the newest instant — age zero, wherever the
    // axis puts it. On the live roll that is always the right edge (`near_s == 0`); on
    // a take still playing it walks across the window as the line fills in, and parks
    // where the line ends. Scrolled off, it is simply not drawn: a marker clamped to
    // the edge would claim the line ends there.
    let head_x = x_of(0.0);
    if head_x >= plot.left() && head_x <= plot.right() {
        painter.line_segment(
            [pos2(head_x - 0.5, plot.top()), pos2(head_x - 0.5, plot.bottom())],
            Stroke::new(1.0, PLAYHEAD),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live roll's axis, as `app::pitch_roll_panel` builds it.
    const LIVE: TimeAxis = TimeAxis {
        near_s:     0.0,
        far_s:      10.0,
        zero_age_s: 0.0,
        mode:       RollMode::Live,
    };

    fn column(age_s: f32) -> RollColumn<'static> {
        RollColumn {
            age_s,
            pitch: None,
            heat: &[],
        }
    }

    /// REGRESSION: making the span a variable must not have moved the ruler the live
    /// roll already had.
    ///
    /// It read every 2 s across its 10 s window, from a constant. That constant is gone,
    /// and if the replacement disagrees with it the live panel's picture changed as a
    /// side effect of a feature built next to it — which is exactly what a refactor is
    /// not allowed to do, and nothing else here would notice.
    #[test]
    fn the_live_ruler_still_reads_every_two_seconds() {
        assert_eq!(tick_step_s(LIVE.span_s()), 2.0);
    }

    /// The ruler stays readable at any zoom, which is the whole reason the step is
    /// computed rather than fixed: 2 s draws *nothing* across a window zoomed to a
    /// note's attack, and a picket fence across a 35 s take.
    #[test]
    fn the_ruler_step_is_readable_at_any_zoom() {
        for span_s in [0.05_f32, 0.2, 1.0, 3.0, 10.0, 35.0, 120.0] {
            let step = tick_step_s(span_s);
            let lines = span_s / step;
            assert!(
                (1.0..=12.0).contains(&lines),
                "a {span_s} s window gets {lines:.1} gridlines at a {step} s step"
            );
            // 1-2-5×10ᵏ: what makes a glance convert to a number. Normalise the step to
            // its mantissa and check it is one of the three.
            let mantissa = step / 10f32.powf(step.log10().floor());
            assert!(
                [1.0, 2.0, 5.0].iter().any(|m| (m - mantissa).abs() < 1e-3),
                "step {step} is not a 1-2-5 step (mantissa {mantissa})"
            );
        }
    }

    /// REGRESSION: the frame gap is measured across the columns handed in, **not** from
    /// the oldest one's age alone.
    ///
    /// The old form divided `columns[0].age_s` by the count, which silently assumed the
    /// newest column sits at the playhead. True while nobody culls; false the moment a
    /// frozen take is scrolled to its middle, where the newest visible column can be
    /// twenty seconds old. The gap would then be inflated by the whole scroll distance —
    /// every heat cell drawn a mile wide, and the line cheerfully joined across real
    /// holes, because `max_join_s` is three of these.
    #[test]
    fn the_frame_gap_is_measured_across_the_columns_given() {
        // Six columns, 16 ms apart, sitting 20 s in the past — a scrolled take.
        let scrolled: Vec<RollColumn<'_>> = (0..6).map(|i| column(20.08 - i as f32 * 0.016)).collect();
        let gap = mean_gap_s(&scrolled);
        assert!(
            (gap - 0.016).abs() < 1e-4,
            "gap is {gap:.4} s between frames 16 ms apart; the scroll distance leaked in"
        );

        // The live case — newest at the playhead — must be unchanged by that fix.
        let live: Vec<RollColumn<'_>> = (0..6).map(|i| column(0.08 - i as f32 * 0.016)).collect();
        assert!((mean_gap_s(&live) - 0.016).abs() < 1e-4);

        // One column has no gap to measure, and must not divide by zero to find out.
        assert_eq!(mean_gap_s(&[column(1.0)]), 0.0);
        assert_eq!(mean_gap_s(&[]), 0.0);
    }
}
