//! The frozen roll — a recorded take, whole, on a time axis you can scroll and zoom.
//!
//! ## Why the live roll cannot do this job
//!
//! [`crate::app::pitch_roll_panel`] *watches*: ten seconds wide, pinned to the playhead,
//! everything older thrown away. That is exactly right while you play, and useless for
//! reading a take — the corpus has a 35 s trill in it, so two thirds of it would be gone
//! before you could look. This panel *reads*: it keeps the take entire and moves the
//! window instead of the take.
//!
//! Same renderer, same engine history, same frames (`ui::pianoroll`,
//! `pitch_roll_panel::columns_of`). Only two policies differ, and both differ for the
//! same reason — **a take is evidence, and the live roll is a mirror to play into.**
//!
//! ## The framing is min/max, where the live roll's is quantiles — deliberately
//!
//! The live view frames on the 10th..90th percentile so that one octave slip cannot pin
//! the rows an octave wider for the next ten seconds. Here that rule is precisely
//! backwards: **the slip is what you came to look at.** A take gets opened *because* the
//! detector did something wrong in it, so the outlier the live roll is right to throw
//! out is the entire subject, and the framing is the full played range.
//!
//! ## What this panel is for, and what it is not for yet
//!
//! Triage, as the user put it: «я играю — потом размечаю поверх того что задетектилось.
//! и там будет видно пики куда что зашло не туда». Seeing the detector's line *is* that
//! job, so the line is on.
//!
//! Marking notes up is Ф2b and is not here yet — which matters, because the rules invert
//! when it lands. The line being read here is the line a marker would be *agreeing with*,
//! and a corpus that agrees with the detector proves nothing about it
//! (`memory/kickstart_recording_and_annotation.md`). Then: a mark's pitch comes from a
//! declaration made *before* playing, the line becomes a layer that is off by default,
//! and only the *time* axis is marked on. The time axis is legitimate — the user is the
//! authority on when a note started. The pitch axis is the mirror.

use std::ops::Range;
use std::path::PathBuf;

use eframe::egui::{
    Align,
    Context,
    Layout,
    Rect,
    Response,
    RichText,
    Sense,
    Ui,
    vec2,
};

use super::App;
use super::pitch_roll_panel::{
    MIN_SPAN,
    RollLayer,
    VIEW_PAD,
    columns_of,
};
use crate::audio::{
    MelodyFrame,
    MelodyHistory,
    ReplayStatus,
};
use crate::ui::pianoroll::{
    self,
    RollColumn,
    RollMode,
    TimeAxis,
};
use crate::ui::tokens::color;

/// How much of the take the history keeps: **all of it**.
///
/// The live roll keeps a window because what fell off the left is already off the
/// screen. Here the opposite holds — the take's beginning is wanted exactly as much as
/// its end, and it is wanted twenty minutes later. `INFINITY` is not "a lot" but "there
/// is nothing to trim by": what bounds this history is the take, which is finite and
/// ends, and not a span of time.
const TAKE_RETENTION_S: f64 = f64::INFINITY;

/// The narrowest the window may be zoomed, in seconds — about three bank frames.
///
/// Past this there is nothing left to resolve: the columns are wider than the plot and
/// you are reading one frame's rounding, not the take.
const MIN_WINDOW_S: f32 = 0.05;

/// Zoom per pixel of wheel scroll, as an exponent — so a notch multiplies the span
/// rather than adding to it, and zooming out undoes zooming in exactly.
const ZOOM_PER_SCROLL_PX: f32 = 0.0015;

/// Time kept either side of the window when culling columns to what is on screen.
///
/// Culling is what keeps a 35 s take (~2000 columns, each a full spectral slice)
/// affordable to paint at 60 fps when you are zoomed into half a second of it. The
/// margin is what stops the cull being visible: the line has to *enter* the left edge
/// and *leave* the right one, and it can only do that if the column just outside is
/// still there to draw a segment to.
const CULL_MARGIN_S: f32 = 0.1;

/// Plot height in pixels. Taller than it is wide is wrong for a waterfall; this is the
/// same order as the live roll's, with room for the controls above it.
const PLOT_HEIGHT: f32 = 320.0;

/// The frozen roll's state.
#[derive(Default)]
pub struct TakeRoll {
    /// The take on screen. `None` = nothing has been replayed this session, which is a
    /// real state and not a missing value: the panel has nothing to draw and says so.
    ///
    /// Everything that only means something *with* a take lives inside, so that none of
    /// it can be read as a zero when there is no take — a take is never 0.0 s long, and
    /// a field claiming so would be a lie the type system was helping to tell.
    loaded: Option<LoadedTake>,
    /// Which layer is painted under the line. Outside [`LoadedTake`] because it is a
    /// property of the *looking*, not of the take: having chosen to read the salience,
    /// you mean to keep reading it as you step through take after take.
    layer:  RollLayer,
}

/// A take, and the window onto it.
struct LoadedTake {
    /// Which take this is — the identity the harvest compares against to notice that a
    /// different one started playing.
    path:    PathBuf,
    /// The take's length in seconds, from the corpus listing (the WAV's own header).
    /// This is what "the whole take" means for fitting and for clamping the window.
    seconds: f32,
    /// Every frame the engine published for this take, oldest → newest — the take's
    /// line, entire. See [`TAKE_RETENTION_S`].
    frames:  MelodyHistory,
    /// How far this panel has read the engine's history. Ours alone, so the live roll
    /// reads the same frames with its own (see `AudioEngine::melody_since`).
    cursor:  Option<u64>,
    /// The visible slice, **in take seconds** — 0.0 is the take's first sample.
    ///
    /// Take seconds rather than ages, because that is the frame the user reasons in
    /// («слип на третьей секунде») and it is the only one that holds still: an age is
    /// measured from the playhead, and the playhead moves while the take plays. The
    /// conversion to the renderer's ages happens once, in [`Self::time_axis`].
    window:  Range<f32>,
    /// Fractional-MIDI window at the plot's bottom/top edges, framed on the take's full
    /// played range (see the module docs on why full, not quantiles).
    view_lo: f32,
    view_hi: f32,
}

impl LoadedTake {
    fn new(path: PathBuf, seconds: f32) -> Self {
        Self {
            path,
            seconds,
            frames: MelodyHistory::with_retention(TAKE_RETENTION_S),
            cursor: None,
            // Open on the whole take: its length is known before its first frame is, so
            // the line draws *into* a window that already means something, and nothing
            // has to jump or auto-follow as it fills.
            window: 0.0..seconds.max(MIN_WINDOW_S),
            // Violin open strings (G3=55 .. E5=76) with headroom — the same start the
            // live roll makes, and just as short-lived: the first frame reframes it.
            view_lo: 53.0,
            view_hi: 79.0,
        }
    }

    /// Take everything the engine has published since the last read.
    fn update(&mut self, fresh: Vec<MelodyFrame>) {
        let Some(last_seq) = fresh.last().map(|frame| frame.seq) else {
            return; // nothing new — hold the picture rather than let it drift
        };
        self.cursor = Some(last_seq);
        for frame in fresh {
            self.frames.push(frame);
        }
        self.reframe();
    }

    /// Frame the pitch window on the take's **whole** played range.
    ///
    /// Min/max, not quantiles, and not eased — see the module docs. Not eased because a
    /// frozen take does not move: easing is how the live view keeps up with a player,
    /// and the only motion here is the range growing as the take streams in, which is
    /// over once it has played.
    ///
    /// Framed on the whole take rather than the visible slice so that the rows stay put
    /// while you scroll: two moments of the take are only comparable by eye if they are
    /// measured against the same grid.
    fn reframe(&mut self) {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for frame in self.frames.iter() {
            if let Some(pitch) = frame.pitch {
                lo = lo.min(pitch);
                hi = hi.max(pitch);
            }
        }
        if !lo.is_finite() {
            return; // not a single note yet — keep the current framing
        }

        let (mut view_lo, mut view_hi) = (lo - VIEW_PAD, hi + VIEW_PAD);
        // A take of one held note spans a semitone; without this it would be one
        // enormous row, and the rest of the plot empty.
        if view_hi - view_lo < MIN_SPAN {
            let center = 0.5 * (view_lo + view_hi);
            view_lo = center - MIN_SPAN * 0.5;
            view_hi = center + MIN_SPAN * 0.5;
        }
        self.view_lo = view_lo;
        self.view_hi = view_hi;
    }

    /// The newest frame's timestamp — where the line has got to, in take seconds.
    ///
    /// `None` until the first frame lands, which is the take's first ~16 ms and the one
    /// moment there is genuinely no line to place anything against.
    fn playhead_t(&self) -> Option<f64> {
        self.frames.newest().map(|frame| frame.t)
    }

    /// Visible span in seconds — what one pixel is worth.
    fn span_s(&self) -> f32 {
        self.window.end - self.window.start
    }

    /// The window as the renderer's time axis: ages before the newest frame.
    ///
    /// `near_s` goes **negative** while the take is still playing and the window reaches
    /// past the line — that is honest and the renderer expects it: the ruler runs on to
    /// the take's end, and the line simply has not arrived there yet.
    fn time_axis(&self, playhead_t: f64) -> TimeAxis {
        TimeAxis {
            near_s:     (playhead_t - self.window.end as f64) as f32,
            far_s:      (playhead_t - self.window.start as f64) as f32,
            // The ruler's zero is the take's start, and the take's start is exactly
            // `playhead_t` seconds old — so labels come out as take seconds and hold
            // still while the playhead moves.
            zero_age_s: playhead_t as f32,
            mode:       RollMode::Frozen,
        }
    }

    /// The visible columns, aged against the playhead. See [`CULL_MARGIN_S`].
    fn columns(&self, playhead_t: f64, layer: RollLayer) -> Vec<RollColumn<'_>> {
        let lo = (self.window.start - CULL_MARGIN_S) as f64;
        let hi = (self.window.end + CULL_MARGIN_S) as f64;
        columns_of(
            self.frames.iter().filter(|frame| frame.t >= lo && frame.t <= hi),
            playhead_t,
            layer,
        )
    }

    /// Show the take whole.
    fn fit(&mut self) {
        self.window = 0.0..self.seconds.max(MIN_WINDOW_S);
    }

    /// Slide the window by `delta_s` take-seconds.
    fn pan(&mut self, delta_s: f32) {
        self.window = (self.window.start + delta_s)..(self.window.end + delta_s);
        self.clamp();
    }

    /// Multiply the span by `factor`, keeping `anchor_s` where it is on screen.
    ///
    /// Anchored zoom rather than centred: the point of zooming is to look *at*
    /// something, and centred zoom walks it off the screen just as it gets interesting.
    fn zoom_about(&mut self, factor: f32, anchor_s: f32) {
        // Where the anchor sits across the window (0 = left edge, 1 = right) — the
        // fraction that must survive the zoom for it to stay under the cursor.
        let fraction = (anchor_s - self.window.start) / self.span_s();
        let span = self.span_s() * factor;
        let start = anchor_s - span * fraction;
        self.window = start..(start + span);
        self.clamp();
    }

    /// Keep the window inside the take, and wider than [`MIN_WINDOW_S`].
    ///
    /// Clamped rather than free because past the ends there is nothing — not silence, no
    /// data at all — and a plot showing four seconds of nothing beside the take reads as
    /// a take with four seconds of nothing in it.
    fn clamp(&mut self) {
        let whole = self.seconds.max(MIN_WINDOW_S);
        let span = self.span_s().clamp(MIN_WINDOW_S, whole);
        // Slide, don't squash: hitting an end must not silently rescale the zoom the
        // user chose. `max(0.0)` covers the span == whole case, where there is nowhere
        // to slide to at all.
        let start = self.window.start.clamp(0.0, (whole - span).max(0.0));
        self.window = start..(start + span);
    }

    /// Scroll and zoom.
    fn interact(&mut self, ui: &Ui, resp: &Response, plot: Rect) {
        // Double-click = "show me the whole thing" — the way back from any window,
        // without hunting for a button.
        if resp.double_clicked() {
            self.fit();
            return;
        }

        let px_per_second = plot.width() / self.span_s();
        if resp.dragged() {
            // Drag moves the *take*, not the window: pull right and the take goes right,
            // which means the window walks left, into the past.
            self.pan(-resp.drag_delta().x / px_per_second);
        }

        let Some(pos) = resp.hover_pos() else {
            return;
        };
        // The wheel zooms. Take the delta rather than read it: this panel lives inside a
        // `ScrollArea`, which reads the same delta *after* us and would otherwise scroll
        // the whole pane out from under the roll while the roll zoomed — both, at once,
        // for one gesture. Zeroing y leaves the horizontal wheel to the pane.
        let scroll_y = ui.input_mut(|input| {
            let delta_y = input.smooth_scroll_delta.y;
            input.smooth_scroll_delta.y = 0.0;
            delta_y
        });
        if scroll_y == 0.0 {
            return;
        }
        let anchor_s = self.window.start + (pos.x - plot.left()) / px_per_second;
        // Wheel up (positive) → zoom in → span shrinks, hence the sign.
        self.zoom_about((-scroll_y * ZOOM_PER_SCROLL_PX).exp(), anchor_s);
    }
}

impl App {
    /// Collect the replayed take's frames.
    ///
    /// Called every frame from `eframe::App::ui`, **not** from the panel, and for a
    /// stronger reason than the take list's: egui_tiles does not draw an unselected tab,
    /// so a take replayed while the user watches the live roll would land nowhere. The
    /// take is played back in real time, once — there is no second chance to collect it.
    ///
    /// That is also why this asks for the bank and for repaints itself. The engine keeps
    /// only [`crate::audio::MELODY_HISTORY_SECONDS`] of history, so an idle UI is not a
    /// slow UI here: it is a hole in the take's line, of exactly the length of the idle.
    pub(super) fn harvest_take_roll(&mut self, ctx: &Context) {
        // Drain only while a take holds the input. On `Idle` the microphone is back and
        // its frames are a different stream entirely; on `Failed` there is nothing. In
        // both cases the take already on screen stays — it is what the user is reading,
        // and handing the input back is not a reason to clear it.
        let (path, playing) = match self.audio.replay_status() {
            ReplayStatus::Playing { path, .. } => (path, true),
            ReplayStatus::Finished { path } => (path, false),
            ReplayStatus::Idle | ReplayStatus::Failed(_) | ReplayStatus::Unsupported => return,
        };

        if self.take_roll.loaded.as_ref().map(|take| &take.path) != Some(&path) {
            // A different take: new evidence, so none of the last one's frames may
            // survive into it. The length comes from the corpus listing, which is where
            // the ▶ that started this replay came from — so the entry is there, and if
            // it somehow is not, the window has no honest bounds and this must not
            // quietly pick some.
            let seconds = self
                .corpus
                .iter()
                .find(|take| take.path == path)
                .map(|take| take.seconds())
                .unwrap();
            self.take_roll.loaded = Some(LoadedTake::new(path, seconds));
        }

        if playing {
            // The bank parks when nobody asks, and a parked bank publishes no frames —
            // so while a take plays this panel is a consumer of it, open or not.
            self.audio.request_resonator();
        }

        let loaded = self.take_roll.loaded.as_mut().unwrap();
        let fresh = self.audio.melody_since(loaded.cursor);
        // The take's tail can still be arriving after the status says `Finished`: the
        // replay thread parks at the last sample, but the bank publishes what it has
        // already been handed. Keep waking until a drain comes back empty — the replay
        // pushes no more samples, so the bank publishes no more frames, and this ends.
        let still_arriving = !fresh.is_empty();
        loaded.update(fresh);
        if playing || still_arriving {
            ctx.request_repaint();
        }
    }

    /// The frozen roll: the take, with the window the user put on it.
    pub(super) fn draw_take_roll(&mut self, ui: &mut Ui) {
        let settings = self.audio.analysis_settings();
        let style = settings.accidental;
        let res_min_midi = settings.resonator.min_midi.as_u8() as i32;
        let res_max_midi = settings.resonator.max_midi.as_u8() as i32;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Take Roll").color(color::TEXT_CAPTION).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.selectable_value(&mut self.take_roll.layer, RollLayer::Salience, "SWIPE′");
                ui.selectable_value(&mut self.take_roll.layer, RollLayer::Spectrum, "spectrum");
            });
        });
        ui.add_space(6.0);

        let layer = self.take_roll.layer;
        let Some(loaded) = self.take_roll.loaded.as_mut() else {
            ui.label(
                RichText::new("Play a take above — the whole line lands here, to scroll and zoom")
                    .color(color::TEXT_MUTED)
                    .size(12.0),
            );
            return;
        };
        let Some(playhead_t) = loaded.playhead_t() else {
            ui.label(
                RichText::new("Waiting for the first frame…")
                    .color(color::TEXT_MUTED)
                    .size(12.0),
            );
            return;
        };

        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, PLOT_HEIGHT), Sense::click_and_drag());
        // The plot is inset from `rect` by the label gutters, and the pointer has to be
        // read against the plot, not the widget — see `pianoroll::plot_rect`.
        let plot = pianoroll::plot_rect(rect);
        loaded.interact(ui, &resp, plot);

        let painter = ui.painter_at(rect);
        pianoroll::draw_pitch_roll(
            &painter,
            rect,
            &loaded.columns(playhead_t, layer),
            res_min_midi,
            res_max_midi,
            loaded.view_lo,
            loaded.view_hi,
            loaded.time_axis(playhead_t),
            style,
        );

        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} · {:.2}–{:.2} s of {:.1} · drag to pan, wheel to zoom, double-click to fit",
                loaded.path.file_name().unwrap().to_string_lossy(),
                loaded.window.start,
                loaded.window.end,
                loaded.seconds,
            ))
            .color(color::TEXT_HINT)
            .monospace()
            .size(11.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use web_time::Instant;

    use super::*;
    use crate::audio::AudioEngine;

    /// A take: `seconds` long, replayed at the bank's cadence.
    const CADENCE_S: f64 = 0.016;

    fn take(seconds: f32) -> LoadedTake {
        LoadedTake::new(PathBuf::from("/testdata/g_string_trill.wav"), seconds)
    }

    /// Feed the take `count` frames of `pitch`, continuing from where it left off.
    fn play(loaded: &mut LoadedTake, count: usize, pitch: Option<f32>) {
        let first = loaded.frames.newest().map_or(0, |frame| frame.seq) + 1;
        let start_t = loaded.frames.newest().map_or(0.0_f64, |frame| frame.t);
        let frames = (0..count)
            .map(|i| {
                MelodyFrame {
                    seq: first + i as u64,
                    t: start_t + (i + 1) as f64 * CADENCE_S,
                    pitch,
                    level: 0.5,
                    heat: Vec::new(),
                    salience: None,
                }
            })
            .collect();
        loaded.update(frames);
    }

    /// REGRESSION, and the reason this panel exists: the take is kept **whole**.
    ///
    /// The live roll trims to ten seconds because what fell off is off the screen. Run
    /// that rule over `a_string_trill.wav` — 35 s, the take most in need of reading —
    /// and the first two thirds are gone before the user can look at them. The retention
    /// is the take, not a span (see [`TAKE_RETENTION_S`]).
    #[test]
    fn the_whole_take_is_kept_not_a_ten_second_window() {
        let mut loaded = take(35.0);
        play(&mut loaded, (35.0 / CADENCE_S) as usize, Some(69.0));

        let held = loaded.playhead_t().unwrap() - loaded.frames.oldest().unwrap().t;
        assert!(
            held > 34.9,
            "history spans {held:.2} s; the take is 35 s and all of it is evidence"
        );
    }

    /// REGRESSION: the frozen roll frames the **slip**, where the live roll is right to
    /// ignore it.
    ///
    /// This is the one place the two rolls' policies point in opposite directions, so it
    /// is the one most likely to be "fixed" into agreement by someone reading
    /// `a_lone_slip_does_not_stretch_the_view` next door. Both are correct: the live view
    /// must not be pinned an octave wide by one bad frame, and a take is opened
    /// *precisely because* of that frame. Framing it out would hide the subject.
    #[test]
    fn the_frozen_roll_frames_the_slip_the_live_roll_would_ignore() {
        let mut loaded = take(10.0);
        play(&mut loaded, 300, Some(69.0)); // steady A4
        play(&mut loaded, 1, Some(81.0)); // one frame, an octave up — the slip
        play(&mut loaded, 300, Some(69.0));

        assert!(
            loaded.view_hi > 81.0,
            "the octave slip at 81 is outside a view topping out at {:.1} — it is the \
             one thing the user opened this take to see",
            loaded.view_hi
        );
        assert!(loaded.view_lo < 69.0, "the note itself is framed too");
    }

    /// The window cannot leave the take, however hard it is thrown.
    ///
    /// Past either end there is no data at all — not silence — so a window that hangs
    /// off the edge draws empty plot that reads as an empty take.
    #[test]
    fn the_window_stays_inside_the_take() {
        let mut loaded = take(8.0);
        loaded.pan(-100.0);
        assert_eq!(loaded.window.start, 0.0, "cannot pan before the first sample");
        assert_eq!(loaded.window.end, 8.0, "and the span is kept while it slides");

        loaded.zoom_about(0.25, 4.0); // zoom into the middle
        loaded.pan(100.0);
        assert_eq!(loaded.window.end, 8.0, "cannot pan past the last sample");
        assert!(
            (loaded.span_s() - 2.0).abs() < 1e-3,
            "hitting the end must slide the window, not squash the zoom: span {:.3}",
            loaded.span_s()
        );
    }

    /// Zoom keeps what is under the cursor under the cursor, and stays within bounds.
    #[test]
    fn zoom_holds_the_anchor_and_respects_the_limits() {
        let mut loaded = take(10.0);
        // Anchor at 7 s, a quarter of the way in from the window's right edge.
        loaded.zoom_about(0.5, 7.0);
        let fraction = (7.0 - loaded.window.start) / loaded.span_s();
        assert!(
            (fraction - 0.7).abs() < 1e-3,
            "the anchor drifted to {fraction:.3} of the window; it was at 0.7"
        );

        // Zoom in past all reason: the span must stop, not collapse or invert.
        for _ in 0..200 {
            loaded.zoom_about(0.5, 7.0);
        }
        assert!(
            (loaded.span_s() - MIN_WINDOW_S).abs() < 1e-4,
            "span {:.4} s; it must stop at MIN_WINDOW_S",
            loaded.span_s()
        );

        // And back out past all reason: the whole take, never more.
        for _ in 0..200 {
            loaded.zoom_about(2.0, 7.0);
        }
        assert_eq!(loaded.window, 0.0..10.0, "zoomed out is exactly the take");
    }

    /// A take opens on itself, whole — no jump, no auto-follow as the line arrives.
    ///
    /// The length is known before the first frame is (it comes from the WAV's header via
    /// the corpus), which is what lets the window be right from the start rather than
    /// chase the playhead and settle.
    #[test]
    fn a_take_opens_on_the_whole_of_itself() {
        let mut loaded = take(8.234);
        assert_eq!(loaded.window, 0.0..8.234);

        // The line arriving must not move the window the user is watching.
        play(&mut loaded, 100, Some(69.0));
        assert_eq!(loaded.window, 0.0..8.234);
    }

    /// The ruler reads take seconds, and holds still while the playhead moves.
    ///
    /// The renderer's axis is ages, which shift under every new frame; the user's frame
    /// of reference is «на третьей секунде дубля», which must not. `zero_age_s` is what
    /// converts one into the other — a column's label is `zero_age_s - age_s`, and here
    /// that has to come out as the column's own `t`.
    #[test]
    fn the_ruler_reads_take_seconds_whatever_the_playhead_does() {
        let mut loaded = take(10.0);
        play(&mut loaded, 200, Some(69.0)); // playhead at 3.2 s

        let axis = loaded.time_axis(loaded.playhead_t().unwrap());
        let label_of = |age_s: f32| axis.zero_age_s - age_s;
        // A column at t = 1.0 s is (playhead - 1.0) old, and must read "1.0s".
        let age_of_one_second = (loaded.playhead_t().unwrap() - 1.0) as f32;
        assert!(
            (label_of(age_of_one_second) - 1.0).abs() < 1e-3,
            "a column one second into the take reads {:.3}s",
            label_of(age_of_one_second)
        );

        // Play on: the same column is older now, and must still read "1.0s".
        play(&mut loaded, 200, Some(69.0));
        let axis = loaded.time_axis(loaded.playhead_t().unwrap());
        let age_of_one_second = (loaded.playhead_t().unwrap() - 1.0) as f32;
        assert!(
            (axis.zero_age_s - age_of_one_second - 1.0).abs() < 1e-3,
            "the ruler moved under the take as it played"
        );
    }

    /// VERIFICATION on a real take: a real bowed G, replayed through the real engine in
    /// real time, lands in this panel's state — whole, framed, and on the take's own
    /// clock.
    ///
    /// Everything above this is the panel arguing with itself: hand-made frames, at a
    /// cadence chosen by the test, proving the arithmetic does what the arithmetic was
    /// written to do. None of it would notice if the engine's real frames arrived on a
    /// clock this panel misreads — and the panel's entire premise is a claim *about* those
    /// frames: that they carry take time in `t`, from zero, restarting per replay. That
    /// claim is inherited from `replay` building a fresh pipeline per capture. If it is
    /// ever false, every window, ruler and label here is silently wrong, the unit tests
    /// stay green, and the picture still looks plausible — which is the failure this whole
    /// kickstart exists to refuse (`memory/violin_recordings_are_the_ground_truth.md`).
    ///
    /// Real time, because the engine's cadence is gated on the wall clock: shovelling the
    /// WAV in faster analyses it once. That is the same reason `replay` sleeps, and it is
    /// what makes ~10 s here a cost worth paying rather than a slow test.
    #[test]
    #[ignore = "needs testdata/*.wav + ~10 s of real time — run with --ignored --nocapture"]
    fn a_real_take_replayed_through_the_engine_fills_the_roll() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let path = dir.join("g_open_slow_strokes.wav");
        let engine = AudioEngine::new();

        // The length comes from the corpus listing, exactly as `harvest_take_roll` takes
        // it — so this checks that source too, against the take the engine then plays.
        let seconds = engine
            .list_takes(&dir)
            .iter()
            .find(|take| take.path == path)
            .expect("g_open_slow_strokes.wav must be in the corpus")
            .seconds();
        let mut loaded = LoadedTake::new(path.clone(), seconds);

        engine.start_replay(path.clone());
        // The loop the panel runs: ask for the bank (it parks otherwise, and publishes
        // nothing), drain by cursor, once a UI frame.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            engine.request_resonator();
            loaded.update(engine.melody_since(loaded.cursor));
            match engine.replay_status() {
                ReplayStatus::Finished { .. } => break,
                ReplayStatus::Failed(message) => panic!("replay refused the take: {message}"),
                _ => assert!(Instant::now() < deadline, "replay never finished"),
            }
            thread::sleep(Duration::from_millis(16));
        }
        loaded.update(engine.melody_since(loaded.cursor));

        let oldest = loaded.frames.oldest().unwrap().t;
        let held = loaded.playhead_t().unwrap() - oldest;
        let voiced = loaded.frames.iter().filter(|f| f.pitch.is_some()).count();
        println!("\n=== g_open_slow_strokes in the take roll ===");
        println!("  take       : {seconds:.2} s (from the WAV header)");
        println!("  frames     : {}", loaded.frames.len());
        println!("  ...voiced  : {voiced}");
        println!("  held       : {held:.2} s of line, from t={oldest:.3}");
        println!(
            "  window     : {:.2}..{:.2}",
            loaded.window.start, loaded.window.end
        );
        println!("  framed     : {:.1}..{:.1} midi", loaded.view_lo, loaded.view_hi);

        assert!(
            voiced > 0,
            "the engine replayed a bowed G and decided no pitch, ever"
        );
        // `t` starts at the take's start: the first frame is one bank cadence in, not
        // wherever a long-running live stream's clock had got to. This is the claim the
        // window and the whole ruler rest on.
        assert!(
            oldest < 0.2,
            "the take's first frame is at t={oldest:.3}; take time must start at the take"
        );
        // The take is held *whole* — the live roll's ten-second window would have thrown
        // away everything before t=0, and this take is nearly ten seconds long.
        assert!(
            held > seconds as f64 - 0.5,
            "held {held:.2} s of a {seconds:.2} s take; the rest is gone"
        );
        // The window the take opened on covers what actually arrived, with nothing
        // hanging off either end.
        assert!(
            loaded.window.start == 0.0 && (loaded.window.end - seconds).abs() < 0.01,
            "the window {:.2}..{:.2} does not fit the {seconds:.2} s take",
            loaded.window.start,
            loaded.window.end
        );
        // Framed on a real open G (G3 = 55): the framing ran on real pitches, not on the
        // 53..79 default it starts at.
        assert!(
            loaded.view_lo < 55.0 && loaded.view_hi > 55.0,
            "framed {:.1}..{:.1}; the open G at 55 must be in view",
            loaded.view_lo,
            loaded.view_hi
        );
        // Every frame is inside the window, so `fit` really does show all of it.
        let visible = loaded.columns(loaded.playhead_t().unwrap(), RollLayer::Spectrum);
        assert_eq!(
            visible.len(),
            loaded.frames.len(),
            "fit must show the whole take: {} of {} columns are in the window",
            visible.len(),
            loaded.frames.len()
        );
    }

    /// Culling is invisible: the columns either side of the window survive, so the line
    /// enters and leaves the edges instead of starting at them.
    #[test]
    fn the_cull_keeps_the_columns_the_line_enters_from() {
        let mut loaded = take(10.0);
        play(&mut loaded, 600, Some(69.0)); // 9.6 s of line
        loaded.window = 4.0..5.0;

        let columns = loaded.columns(loaded.playhead_t().unwrap(), RollLayer::Spectrum);
        let playhead = loaded.playhead_t().unwrap();
        let times: Vec<f64> = columns
            .iter()
            .map(|column| playhead - column.age_s as f64)
            .collect();
        let oldest = times.iter().copied().fold(f64::INFINITY, f64::min);
        let newest = times.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        assert!(
            oldest < 4.0 && newest > 5.0,
            "culled to {oldest:.3}..{newest:.3}; the line must be able to enter the \
             window from outside it"
        );
        assert!(
            oldest > 4.0 - 2.0 * CULL_MARGIN_S as f64 && newest < 5.0 + 2.0 * CULL_MARGIN_S as f64,
            "culled to {oldest:.3}..{newest:.3}; that is most of the take, so the cull \
             is not culling"
        );
    }
}
