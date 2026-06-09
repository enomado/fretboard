//! Fretboard — the geometric model of the instrument neck and its renderers.
//!
//! The *root logic* (this module) maps musical coordinates onto the screen:
//! given a [`Fret`] and a [`GString`] it returns the `x` / `y` pixel position
//! inside the neck rectangle. The renderers that paint on top of that model
//! live in submodules and reach back in through explicit paths
//! (e.g. `crate::ui::fretboard::draw::draw_fret_lines`) — no re-exports, so it
//! stays obvious where each piece lives:
//!   - [`draw`]      — note marks, string lines, fret lines and inlays,
//!   - [`positions`] — the cello-position bracket overlay.

use std::ops::Range;

use crate::core_types::tuning::{
    Fret,
    GString,
    Tuning,
};

pub mod draw;
pub mod positions;

/// How fret spacing is laid out along the neck.
pub enum FretConfig {
    /// Physically accurate: each fret sits at its equal-tempered position, so
    /// spacing shrinks geometrically towards the bridge (real-neck look).
    Log,
    /// Every fret gets the same width — easier to read, geometrically wrong.
    Linear,
}

/// The neck as drawn on screen: the screen rectangle it occupies, the tuning
/// that fixes the open notes, and which slice of frets is currently visible.
pub struct Fretboard {
    /// Horizontal screen span of the neck, nut → bridge, in pixels.
    pub screen_size_x:   Range<f32>,
    /// Vertical screen span of the neck, in pixels.
    pub screen_size_y:   Range<f32>,
    pub config:          FretConfig,
    pub tuning:          Tuning,
    /// Half-open range of frets to show, e.g. `Fret(1)..Fret(19)` for frets 1–18.
    pub fret_range_show: Range<Fret>,
}

impl Fretboard {
    /// Screen `x` of fret `n`, honouring the current [`FretConfig`].
    pub fn fret_pos(&self, n: Fret) -> f32 {
        match self.config {
            FretConfig::Log => fret_position_log(&self.screen_size_x, &self.fret_range_show, n),
            FretConfig::Linear => fret_position_linear(&self.screen_size_x, &self.fret_range_show, n),
        }
    }

    /// Screen `y` of string `s`. The `+ 2` widens the divisor by one row at each
    /// end, so the outer strings keep a margin instead of sitting flush against
    /// the top and bottom edges (see [`string_position`]).
    pub fn string_pos(&self, s: GString) -> f32 {
        string_position(&self.screen_size_y, s, (self.tuning.string_count() + 2) as u32)
    }

    /// Visible frets, low → high (`fret_range_show` is half-open).
    pub fn iter_frets(&self) -> impl Iterator<Item = Fret> {
        (self.fret_range_show.start.0..self.fret_range_show.end.0).map(Fret)
    }

    /// Every string of the tuning, counted from 1 (see `Tuning::note`).
    pub fn iter_strings(&self) -> impl Iterator<Item = GString> {
        (1..self.tuning.string_count() + 1).map(GString)
    }
}

/// Equal-tempered (logarithmic) fret position.
///
/// On a real neck the distance from the nut to fret `k` is
/// `scale_length * (1 - 2^(-k/12))` — each semitone divides the remaining
/// string length by `2^(1/12)`. We invert that: `scale_length` is solved so the
/// last visible fret lands exactly on `ui_range.end`, then every fret is placed
/// by the same formula. Frets are re-based to the visible window (`+ 1` so the
/// first shown fret starts a fret-width in from the nut, not on top of it).
fn fret_position_log(
    ui_range: &Range<f32>,    // screen span of the neck
    fret_range: &Range<Fret>, // visible fret window
    n: Fret,                  // fret to place
) -> f32 {
    if n.0 < fret_range.start.0 {
        return ui_range.start;
    }
    if n.0 >= fret_range.end.0 {
        return ui_range.end;
    }

    let effective_end = fret_range.end.0 - fret_range.start.0 + 1;
    let scale_length = (ui_range.end - ui_range.start) / (1.0 - 2f32.powf(-(effective_end as f32 / 12.0)));
    let effective_fret = n.0 - fret_range.start.0 + 1;

    ui_range.start + scale_length - scale_length / 2f32.powf(effective_fret as f32 / 12.0)
}

/// Even fret spacing: the visible window is split into equal slices regardless
/// of pitch. Reads more clearly than [`fret_position_log`] but is not the shape
/// of a real neck.
fn fret_position_linear(ui_range: &Range<f32>, fret_range: &Range<Fret>, n: Fret) -> f32 {
    if n.0 < fret_range.start.0 {
        return ui_range.start;
    }
    if n.0 >= fret_range.end.0 {
        return ui_range.end;
    }

    let visible = (fret_range.end.0 - fret_range.start.0 + 1) as f32;
    let n_eff = (n.0 - fret_range.start.0 + 1) as f32;

    ui_range.start + (ui_range.end - ui_range.start) * (n_eff / visible)
}

/// Evenly spread strings across the vertical span. String index is 1-based, so
/// string `k` sits at `k / (string_count - 1)` of the way down the range; the
/// caller pads `string_count` (see [`Fretboard::string_pos`]) to leave margins.
fn string_position(range: &Range<f32>, string_index: GString, string_count: u32) -> f32 {
    let span = range.end - range.start;
    range.start + span * string_index.0 as f32 / (string_count.saturating_sub(1) as f32)
}
