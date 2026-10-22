//! The neck model: musical coordinates → screen pixels.
//!
//! Given a [`Fret`] and a [`GString`] it returns the screen point inside the neck
//! rectangle. The neck runs along either screen axis ([`NeckAxis`]), so the model
//! speaks in *along* (nut → bridge) and *across* (string 1 → last string)
//! coordinates and maps them to `x`/`y` in one place, [`Fretboard::point`]. The
//! renderers that paint on top of this model (`crate::ui::fretboard::draw`,
//! `::positions`) borrow a [`Fretboard`] and own no state of their own.

use std::ops::Range;

use eframe::egui::{
    Pos2,
    Rect,
    pos2,
};

use crate::core_types::tuning::{
    Fret,
    GString,
    Tuning,
};

/// Which screen axis the neck runs along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NeckAxis {
    /// Nut on the left, frets running right, string 1 on top — the desktop card.
    Horizontal,
    /// Nut at the top, frets running down, string 1 on the right — a chord
    /// diagram facing the player, lowest string on the left. The phone in
    /// portrait: its long axis is the only one that holds 18 log-spaced frets.
    Vertical,
}

/// How fret spacing is laid out along the neck.
pub enum FretConfig {
    /// Physically accurate: each fret sits at its equal-tempered position, so
    /// spacing shrinks geometrically towards the bridge (real-neck look).
    Log,
    /// Every fret gets the same width — easier to read, geometrically wrong.
    Linear,
}

/// The neck as drawn on screen: the screen rectangle it occupies and the axis
/// it runs along, the tuning that fixes the open notes, and which slice of frets
/// is currently visible.
pub struct Fretboard {
    /// Screen span along the neck, nut → bridge, in pixels.
    along:               Range<f32>,
    /// Screen span across the neck, from string 1's side to the last string's.
    /// Runs backwards (`start > end`) on the vertical neck, which puts string 1
    /// on the right; `string_position` is plain interpolation, so it does not care.
    across:              Range<f32>,
    axis:                NeckAxis,
    pub config:          FretConfig,
    pub tuning:          Tuning,
    /// Half-open range of frets to show, e.g. `Fret(1)..Fret(19)` for frets 1–18.
    pub fret_range_show: Range<Fret>,
}

impl Fretboard {
    /// The neck filling `rect`, running along `axis`.
    pub fn new(
        rect: Rect,
        axis: NeckAxis,
        config: FretConfig,
        tuning: Tuning,
        fret_range_show: Range<Fret>,
    ) -> Self {
        let (along, across) = match axis {
            NeckAxis::Horizontal => (rect.left()..rect.right(), rect.top()..rect.bottom()),
            NeckAxis::Vertical => (rect.top()..rect.bottom(), rect.right()..rect.left()),
        };
        Self {
            along,
            across,
            axis,
            config,
            tuning,
            fret_range_show,
        }
    }

    pub fn axis(&self) -> NeckAxis {
        self.axis
    }

    /// The screen point at `along` (nut → bridge) and `across` (string 1 → last
    /// string) — the one place the neck's coordinates meet the screen's.
    pub fn point(&self, along: f32, across: f32) -> Pos2 {
        match self.axis {
            NeckAxis::Horizontal => pos2(along, across),
            NeckAxis::Vertical => pos2(across, along),
        }
    }

    /// Where the note at `fret` on `string` is drawn.
    pub fn note_center(&self, string: GString, fret: Fret) -> Pos2 {
        self.point(self.fret_pos(fret), self.string_pos(string))
    }

    /// Fret `n` as a line across the whole neck.
    pub fn fret_line(&self, n: Fret) -> [Pos2; 2] {
        let along = self.fret_pos(n);
        [
            self.point(along, self.across.start),
            self.point(along, self.across.end),
        ]
    }

    /// String `s` as a line along the whole neck.
    pub fn string_line(&self, s: GString) -> [Pos2; 2] {
        let across = self.string_pos(s);
        [
            self.point(self.along.start, across),
            self.point(self.along.end, across),
        ]
    }

    /// `offset` px out from the neck's edge on string 1's side (0 = the edge):
    /// above a horizontal neck, right of a vertical one. The position brackets
    /// stack out from there.
    pub fn outside(&self, offset: f32) -> f32 {
        let outward = if self.across.end >= self.across.start {
            -1.0
        } else {
            1.0
        };
        self.across.start + outward * offset
    }

    /// The middle of the neck across its width — where the inlay dots sit.
    pub fn across_center(&self) -> f32 {
        (self.across.start + self.across.end) * 0.5
    }

    /// Position along the neck of fret `n`, honouring the current [`FretConfig`].
    pub fn fret_pos(&self, n: Fret) -> f32 {
        match self.config {
            FretConfig::Log => fret_position_log(&self.along, &self.fret_range_show, n),
            FretConfig::Linear => fret_position_linear(&self.along, &self.fret_range_show, n),
        }
    }

    /// Position across the neck of string `s`. The `+ 2` widens the divisor by one
    /// row at each end, so the outer strings keep a margin instead of sitting flush
    /// against the edges (see `string_position`).
    pub fn string_pos(&self, s: GString) -> f32 {
        string_position(&self.across, s, (self.tuning.string_count() + 2) as u32)
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

/// Evenly spread strings across the neck's width. String index is 1-based, so
/// string `k` sits at `k / (string_count - 1)` of the way down the range; the
/// caller pads `string_count` (see [`Fretboard::string_pos`]) to leave margins.
fn string_position(range: &Range<f32>, string_index: GString, string_count: u32) -> f32 {
    let span = range.end - range.start;
    range.start + span * string_index.0 as f32 / (string_count.saturating_sub(1) as f32)
}
