//! Pitch-roll panel — a horizontal "waterfall table" of what you play/sing.
//!
//! Rows are notes (one per semitone), time flows right → left, and the live
//! detected pitch is traced as a continuous graph across the grid. Unlike the
//! staff panel it does not quantise into written noteheads — it just mirrors the
//! raw pitch line, so glide, vibrato and intonation drift are visible directly.
//!
//! State (the rolling pitch samples + the eased view window) lives in
//! [`PitchRoll`], held on `App`. The stateless grid/graph drawing lives in
//! [`crate::ui::pianoroll`]; this module owns only the capture + auto-framing.

use std::collections::VecDeque;

use eframe::egui::{
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Sense,
    Stroke,
    Ui,
    vec2,
};

use super::App;
use crate::ui::pianoroll::{
    self,
    PitchPoint,
};
use crate::ui::tokens::color;

/// Input level (RMS-ish, 0..1) below which the **heat** column is blanked.
///
/// Only the heat: the melody *line* is silence-gated in the engine now, where the
/// decision belongs (`audio::core::MELODY_LEVEL_GATE`). The heat cannot be, and this
/// is not the same rule wearing a disguise:
///
/// - the line's gate feeds a *decision* — it tells the segmenter the sound stopped;
/// - the heat's gate is *display* — each bank column is normalized to its own max, so
///   a silent column is room noise stretched to full scale, and painting it would fill
///   the rests with a wash.
///
/// The heat also must **not** be blanked on `melody_pitch == None`, tempting as it
/// looks: that also means "the engine rejected a slip", and the heat is precisely the
/// ground truth a slip is supposed to be visible against. Blanking it there would hide
/// the evidence the layer exists to show.
const HEAT_LEVEL_GATE: f32 = 0.02;
/// How many frames of pitch to keep — the graph fills the plot width with these,
/// so this is also the visible time span (~10 s at 60 fps, ~20 s at 30 fps).
const HISTORY_FRAMES: usize = 600;
/// Padding (semitones) kept above/below the played range so the line never rides
/// the very edge of the view.
const VIEW_PAD: f32 = 2.5;
/// Minimum visible pitch span (semitones) so a single sustained note still shows a
/// sensible band of rows around it instead of one giant row.
const MIN_SPAN: f32 = 14.0;
/// Per-frame easing for the view window toward its target range. Small = the rows
/// glide rather than jump when the played range changes.
const VIEW_EASE: f32 = 0.08;

/// Frames of *recent* pitch the framing looks at — a fraction of `HISTORY_FRAMES`
/// on purpose (~3 s at 60 fps against the buffer's ~10 s).
///
/// Framing on the whole waterfall means a phrase played fifteen seconds ago still
/// stretches the view, so the window only ever grows over a session: play low, then
/// play high, and you are stuck reading both forever. The view should follow what is
/// being played *now*; the scrolled-off past does not get a vote.
const FRAMING_FRAMES: usize = 180;
/// Quantiles of the recent pitch that set the view's bottom and top.
///
/// Quantiles, not min/max: min/max is decided by the single most extreme sample in
/// the window, so one octave slip or one grace note pins the view an octave wider
/// for the next several seconds. The 10th/90th percentile ignores brief excursions
/// and tracks the *body* of what is being played. The excursions are not lost —
/// [`Self::reframe`] always keeps the note sounding right now inside the view.
const VIEW_QUANTILE_LO: f32 = 0.10;
const VIEW_QUANTILE_HI: f32 = 0.90;
/// Slack (semitones) the view must be *too big* by before it shrinks — the
/// hysteresis.
///
/// Without it the view chases every wobble of the quantiles, so the rows creep
/// under the note continuously and the whole grid feels alive in a bad way. Growing
/// has no such dead zone: a note going off-screen is a real problem, a slightly
/// oversized view is not. So the view grows on demand and shrinks only when it is
/// clearly, persistently too wide.
const VIEW_SHRINK_SLACK: f32 = 3.0;

/// Live pitch-roll state for the panel.
pub struct PitchRoll {
    /// Per-frame melody-line pitch (fused pYIN), oldest → newest, *after* spike
    /// rejection. `None` = a silent frame or a rejected octave glitch (a gap).
    samples: VecDeque<Option<PitchPoint>>,
    /// Per-frame resonator column aligned 1:1 with `samples` (same index = same
    /// instant), oldest → newest. An *empty* `Vec` marks a silent frame. This is
    /// the spectral-heat ground truth; unlike the line it makes no octave decision.
    heat:    VecDeque<Vec<f32>>,
    /// Eased fractional-MIDI window shown on screen (`view_lo` bottom, `view_hi`
    /// top). Auto-frames the graph on the played range without per-frame jumps.
    view_lo: f32,
    view_hi: f32,
}

impl Default for PitchRoll {
    fn default() -> Self {
        // Default window ≈ violin open strings (G3=55 .. E5=76) with headroom, so
        // the panel looks sensible before the first note eases it to real data.
        Self {
            samples: VecDeque::with_capacity(HISTORY_FRAMES),
            heat:    VecDeque::with_capacity(HISTORY_FRAMES),
            view_lo: 53.0,
            view_hi: 79.0,
        }
    }
}

impl PitchRoll {
    /// Feed one UI frame. `pitch` is the melody-line pitch `Some(fractional_midi)`,
    /// `None` for silence *or* a rejected octave slip — both already decided upstream
    /// in `audio::dsp::melody`, which is the only place the octave is judged. `level`
    /// fades the line; `heat_col` is this frame's resonator column (empty = silent →
    /// a gap in the heat), passed through untouched as the spectral ground truth,
    /// since it makes no octave decision of its own.
    pub fn update(&mut self, pitch: Option<f32>, level: f32, heat_col: Vec<f32>) {
        let accepted = pitch.map(|midi_f| PitchPoint { midi_f, level });
        self.samples.push_back(accepted);
        self.heat.push_back(heat_col);
        while self.samples.len() > HISTORY_FRAMES {
            self.samples.pop_front();
        }
        while self.heat.len() > HISTORY_FRAMES {
            self.heat.pop_front();
        }
        self.reframe();
    }

    /// Ease the view window toward what is being played *now*.
    ///
    /// Three rules, each earning its keep (see the constants for why):
    /// 1. only the last [`FRAMING_FRAMES`] are considered — the scrolled-off past
    ///    gets no vote;
    /// 2. the bounds are *quantiles*, so a lone slip or grace note cannot pin the
    ///    view wide, with the currently sounding note always kept in view;
    /// 3. growing is immediate, shrinking needs [`VIEW_SHRINK_SLACK`] of slack — the
    ///    hysteresis that stops the rows creeping.
    ///
    /// Holds the last window while fully silent, so the rows don't drift when nothing
    /// is playing.
    fn reframe(&mut self) {
        // Rule 1: recent pitch only.
        let recent = self.samples.len().saturating_sub(FRAMING_FRAMES);
        let mut pitches: Vec<f32> = self
            .samples
            .iter()
            .skip(recent)
            .flatten()
            .map(|point| point.midi_f)
            .collect();
        if pitches.is_empty() {
            return; // nothing played recently → keep the current framing
        }

        // Rule 2: quantile bounds over the body of the recent pitch.
        pitches.sort_by(f32::total_cmp);
        let last = pitches.len() - 1;
        let quantile = |t: f32| pitches[(last as f32 * t).round() as usize];
        let mut data_lo = quantile(VIEW_QUANTILE_LO);
        let mut data_hi = quantile(VIEW_QUANTILE_HI);
        // …but never frame out the note sounding right now: it is the one sample the
        // player is actually looking at, and a quantile is free to exclude it.
        if let Some(now) = self.samples.iter().rev().flatten().next() {
            data_lo = data_lo.min(now.midi_f);
            data_hi = data_hi.max(now.midi_f);
        }

        let mut target_lo = data_lo - VIEW_PAD;
        let mut target_hi = data_hi + VIEW_PAD;
        // Enforce a minimum span, centred on the data, so one held note isn't a
        // single fat row.
        if target_hi - target_lo < MIN_SPAN {
            let center = 0.5 * (target_lo + target_hi);
            target_lo = center - MIN_SPAN * 0.5;
            target_hi = center + MIN_SPAN * 0.5;
        }

        // Rule 3: hysteresis. A target that would pull an edge *inward* by less than
        // the slack is ignored outright — that edge simply stays where it is.
        if target_lo > self.view_lo && target_lo - self.view_lo < VIEW_SHRINK_SLACK {
            target_lo = self.view_lo;
        }
        if target_hi < self.view_hi && self.view_hi - target_hi < VIEW_SHRINK_SLACK {
            target_hi = self.view_hi;
        }

        self.view_lo += (target_lo - self.view_lo) * VIEW_EASE;
        self.view_hi += (target_hi - self.view_hi) * VIEW_EASE;

        // The easing above is a glide, not a guarantee: at `VIEW_EASE` it takes ~0.6 s
        // to cover a leap, and for those frames the line the player is watching is
        // *off screen*. So growing is not eased at all — the view snaps open just far
        // enough to hold the current note. Shrinking keeps the glide, which is what
        // makes the motion read as calm: the view opens instantly and closes gently.
        if let Some(now) = self.samples.iter().rev().flatten().next() {
            self.view_lo = self.view_lo.min(now.midi_f - VIEW_PAD);
            self.view_hi = self.view_hi.max(now.midi_f + VIEW_PAD);
        }
    }
}

impl App {
    pub(super) fn draw_pitch_roll_card(&mut self, ui: &mut Ui) {
        // Live panel: keep repainting to track the audio thread, and keep the
        // resonator bank alive so the fused pitch stays low-latency (it parks when
        // no consumer asks — same reason the staff panel requests it).
        ui.ctx().request_repaint();
        self.audio.request_resonator();

        let settings = self.audio.analysis_settings();
        let style = settings.accidental;
        let res_min_midi = settings.resonator.min_midi.as_u8() as i32;
        let res_max_midi = settings.resonator.max_midi.as_u8() as i32;
        let reading = self.audio.reading();
        let level = self.audio.input_level();

        // MELODY LINE source = `melody_pitch`: the resonator bank's fast fine pitch
        // with its octave pinned by pYIN (see `audio::dsp::melody`). Octave-stable
        // like pYIN, but it follows a note change in 8–29 ms instead of ~128 ms.
        //
        // NOT `reading.frequency_hz` (pYIN alone) — that is what this panel used to
        // read, and it put the line ~128 ms behind the heat drawn right beside it.
        //
        // Arrives finished: silence-gated and octave-decided in the engine. No range
        // clamp either — the melody line's range simply *is* the bank's, because the
        // bank is where the pitch comes from. The panel used to carry its own C1..G7
        // window "matching the pYIN tracker's grid", a second opinion about the range
        // that could only ever drift from the first.
        let pitch = reading
            .as_ref()
            .and_then(|r| r.melody_pitch)
            .map(|(midi_f, _)| midi_f);
        // HEAT source = the resonator bank's newest column (fast, per bank column),
        // painted as-is so trills/overtones show with no octave decision. Blanked
        // (empty column) when the input is silent, so rests are clean gaps rather
        // than normalized noise (each column is normalized to its own max).
        let heat_col = if level >= HEAT_LEVEL_GATE {
            reading
                .as_ref()
                .and_then(|r| r.resonator_waterfall.last().cloned())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.pitch_roll.update(pitch, level, heat_col);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Pitch Roll").size(20.0).color(color::TEXT_HEADING));
                    ui.label(
                        RichText::new("Rows are notes; your pitch scrolls in from the right")
                            .color(color::TEXT_HINT)
                            .size(12.0),
                    );
                });

                ui.add_space(10.0);

                let width = ui.available_width();
                let (rect, _resp) = ui.allocate_exact_size(vec2(width, 360.0), Sense::hover());
                let painter = ui.painter_at(rect);
                // `make_contiguous` needs `&mut`; the view fields and the two
                // buffers are disjoint, so borrowck permits reading them together.
                let (view_lo, view_hi) = (self.pitch_roll.view_lo, self.pitch_roll.view_hi);
                let heat = self.pitch_roll.heat.make_contiguous();
                let samples = self.pitch_roll.samples.make_contiguous();
                pianoroll::draw_pitch_roll(
                    &painter,
                    rect,
                    samples,
                    heat,
                    res_min_midi,
                    res_max_midi,
                    view_lo,
                    view_hi,
                    style,
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed the line only; the heat layer is exercised by the renderer, not here.
    fn line(roll: &mut PitchRoll, pitch: Option<f32>, level: f32) {
        roll.update(pitch, level, Vec::new());
    }

    /// Silence keeps the framing put (no drift while nothing plays).
    #[test]
    fn silence_holds_the_view() {
        let mut roll = PitchRoll::default();
        let (lo, hi) = (roll.view_lo, roll.view_hi);
        for _ in 0..30 {
            line(&mut roll, None, 0.0);
        }
        assert_eq!(roll.view_lo, lo);
        assert_eq!(roll.view_hi, hi);
    }

    /// A sustained note eases the window to frame it, keeping the min span.
    #[test]
    fn note_reframes_within_min_span() {
        let mut roll = PitchRoll::default();
        for _ in 0..400 {
            line(&mut roll, Some(69.0), 0.5); // A4
        }
        assert!(
            roll.view_lo < 69.0 && roll.view_hi > 69.0,
            "note is inside the view"
        );
        assert!(
            (roll.view_hi - roll.view_lo) >= MIN_SPAN - 0.5,
            "span stays at least the minimum"
        );
    }

    /// REGRESSION: a phrase played a while ago must not keep the view stretched.
    ///
    /// Reported live as "их слишком много - три [октавы]": framing on min/max over
    /// the whole waterfall meant playing low and then high left the view spanning
    /// both for the next ten seconds, and at three octaves the rows are too short for
    /// `pianoroll` to label anything but the Cs — "больше не рисуются ноты, только
    /// октавы".
    #[test]
    fn old_phrase_does_not_keep_the_view_stretched() {
        let mut roll = PitchRoll::default();
        for _ in 0..200 {
            line(&mut roll, Some(48.0), 0.5); // C3, low phrase
        }
        for _ in 0..600 {
            line(&mut roll, Some(84.0), 0.5); // C6, settle up high
        }
        let span = roll.view_hi - roll.view_lo;
        assert!(
            span < 24.0,
            "view still spans {span:.1} semitones after moving on; the old phrase \
             should have stopped counting"
        );
        assert!(
            roll.view_lo < 84.0 && roll.view_hi > 84.0,
            "the note being played must be in view"
        );
    }

    /// REGRESSION, stated the way it was reported: while playing, the grid must name
    /// **notes**, not just octaves.
    ///
    /// `pianoroll` drops a row's label once the row is under
    /// [`crate::ui::pianoroll::LABEL_MIN_ROW_H`], keeping only the Cs — so "больше не
    /// рисуются ноты, только октавы" is not a labelling bug, it is the framing having
    /// gone too wide. Ties the two together so neither can drift from the other.
    #[test]
    fn framing_keeps_the_rows_labellable() {
        use crate::ui::pianoroll::LABEL_MIN_ROW_H;

        // A modest panel: the plot is the tight case, so if it labels, taller ones do.
        const PLOT_H: f32 = 250.0;
        let mut roll = PitchRoll::default();
        // A realistic phrase: a violin first position wandering over a fifth or so.
        for (i, midi) in [67.0f32, 69.0, 71.0, 72.0, 74.0, 71.0, 69.0]
            .iter()
            .cycle()
            .take(600)
            .enumerate()
        {
            let _ = i;
            line(&mut roll, Some(*midi), 0.5);
        }
        let span = roll.view_hi - roll.view_lo;
        let row_h = PLOT_H / span;
        assert!(
            row_h >= LABEL_MIN_ROW_H,
            "span {span:.1} semitones gives {row_h:.1} px rows in a {PLOT_H:.0} px plot; \
             under {LABEL_MIN_ROW_H} only the C rows get labelled"
        );
    }

    /// A lone octave slip must not pin the view an octave wider — that is what the
    /// quantile bounds are for.
    #[test]
    fn a_lone_slip_does_not_stretch_the_view() {
        let mut roll = PitchRoll::default();
        for _ in 0..400 {
            line(&mut roll, Some(69.0), 0.5); // steady A4
        }
        let span_before = roll.view_hi - roll.view_lo;
        line(&mut roll, Some(81.0), 0.5); // one slipped frame, an octave up
        for _ in 0..200 {
            line(&mut roll, Some(69.0), 0.5);
        }
        let span_after = roll.view_hi - roll.view_lo;
        assert!(
            span_after < span_before + 2.0,
            "one slip stretched the view from {span_before:.1} to {span_after:.1}"
        );
    }

    /// The currently sounding note is always framed, even when it is the outlier the
    /// quantiles would exclude.
    #[test]
    fn current_note_is_never_framed_out() {
        let mut roll = PitchRoll::default();
        for _ in 0..400 {
            line(&mut roll, Some(60.0), 0.5); // settled on C4
        }
        // Leap up and hold briefly — far too few frames to move the quantiles.
        for _ in 0..3 {
            line(&mut roll, Some(79.0), 0.5); // G5
        }
        assert!(
            roll.view_hi > 79.0 - VIEW_PAD,
            "current note at 79 is outside a view topping out at {:.1}",
            roll.view_hi
        );
    }

    /// Hysteresis: a settled note must not make the rows creep frame after frame.
    #[test]
    fn settled_note_does_not_creep() {
        let mut roll = PitchRoll::default();
        for _ in 0..800 {
            line(&mut roll, Some(69.0), 0.5);
        }
        let (lo, hi) = (roll.view_lo, roll.view_hi);
        for _ in 0..60 {
            line(&mut roll, Some(69.0), 0.5);
        }
        assert!(
            (roll.view_lo - lo).abs() < 0.5 && (roll.view_hi - hi).abs() < 0.5,
            "view crept from ({lo:.2}, {hi:.2}) to ({:.2}, {:.2}) on a held note",
            roll.view_lo,
            roll.view_hi
        );
    }

    /// Old samples fall off once the buffer is full.
    #[test]
    fn buffer_is_capped() {
        let mut roll = PitchRoll::default();
        for _ in 0..(HISTORY_FRAMES + 50) {
            line(&mut roll, Some(60.0), 0.4);
        }
        assert_eq!(roll.samples.len(), HISTORY_FRAMES);
        assert_eq!(roll.heat.len(), HISTORY_FRAMES);
    }

    /// A rejected frame reads as a gap, exactly like silence.
    ///
    /// The octave *judgement* itself is no longer this panel's business — it happens
    /// once, upstream, in `audio::dsp::melody`, and is tested there against the bank's
    /// cadence rather than the frame rate. All this panel owes is not to invent a
    /// point where the engine gave it none.
    #[test]
    fn a_rejected_frame_is_a_gap() {
        let mut roll = PitchRoll::default();
        for _ in 0..5 {
            line(&mut roll, Some(69.0), 0.5);
        }
        line(&mut roll, None, 0.5); // engine rejected this frame (slip or silence)
        assert!(roll.samples.back().unwrap().is_none(), "rejected frame is a gap");
        line(&mut roll, Some(71.0), 0.5);
        assert!(roll.samples.back().unwrap().is_some(), "next real frame is drawn");
    }
}
