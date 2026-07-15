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
//! ## Both axes move, one at a time, and the wheel is not ours
//!
//! Drag pans time and pitch together; zoom takes them one axis at a time, about the
//! cursor — **ctrl+wheel = time, shift+wheel = pitch** (a pinch does both at once,
//! because two fingers say so). Double-click fits the take back into view. The plain
//! wheel belongs to the pane's `ScrollArea` — this panel is taller than most panes, so it
//! has to be scrollable, and the roll is most of what the wheel would land on. See
//! [`LoadedTake::interact`].
//!
//! ## The framing is min/max, where the live roll's is quantiles — deliberately
//!
//! The live view frames on the 10th..90th percentile so that one octave slip cannot pin
//! the rows an octave wider for the next ten seconds. Here that rule is precisely
//! backwards: **the slip is what you came to look at.** A take gets opened *because* the
//! detector did something wrong in it, so the outlier the live roll is right to throw
//! out is the entire subject, and the framing is the full played range.
//!
//! ## Two jobs here, and they must not be confused
//!
//! **Triage**, as the user put it: «я играю — потом размечаю поверх того что
//! задетектилось. и там будет видно пики куда что зашло не туда». Seeing the detector's
//! line *is* that job, so the line is on by default when you are only reading.
//!
//! **Marking**, which is the point of the whole exercise: recording what was actually
//! played, so the detector can be scored against it. Here the line is a liability — a
//! corpus that agrees with the detector proves nothing about it. The resolution is not
//! to hide the roll but to split the truth: **the pitch is declared before playing**
//! (the field above the roll, which is also the take's name), and only the **time** is
//! marked on screen. The time axis is legitimate — the player is the authority on when a
//! note started. The pitch axis is the mirror.
//!
//! So arming a declaration turns the line off, and the toggle is still there if the user
//! wants it (they asked for it): what it costs is *recorded on the mark*
//! ([`MarkedAgainst`]) rather than forbidden. See [`crate::app::take_marks`] and
//! `memory/kickstart_recording_and_annotation.md`.

use std::ops::Range;
use std::path::PathBuf;

use eframe::egui::{
    self,
    Align,
    Color32,
    Context,
    Event,
    Layout,
    Rect,
    Response,
    RichText,
    Sense,
    Stroke,
    Ui,
    Vec2,
    vec2,
};

use super::App;
use super::pitch_roll_panel::{
    MIN_SPAN,
    RollLayer,
    VIEW_PAD,
    columns_of,
};
use super::take_marks::{
    Declaration,
    MarkedAgainst,
    NoteMark,
    load_marks,
    save_marks,
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
use crate::ui::segmented::RowCaption;
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

/// The narrowest the pitch window may be zoomed, in semitones.
///
/// Two rows. Past that the grid the pitch is read *against* is gone — a row is the
/// unit of the answer here ("did it play G3 or G2"), so a window narrower than a
/// couple of them is zoomed past the question.
///
/// Deliberately **not** [`MIN_SPAN`], which is the floor for *auto* framing and is a
/// whole 14 semitones: that one keeps a take of one held note from filling the plot
/// with a single fat row, which is a statement about a picture nobody asked for. This
/// one bounds a window the user is aiming by hand, and they are allowed to aim it at
/// two rows.
const MIN_PITCH_SPAN: f32 = 2.0;

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

/// A note mark's bar: half-height either side of its declared row, and the ticks at its
/// two ends.
///
/// **Violet, and that is not decoration.** The roll already speaks two colour languages
/// and a mark belongs to neither: the heat is a cool blue ramp (`pianoroll::heat_color`)
/// and the line runs green → yellow → orange → red by intonation
/// (`ui::theme::intonation_color`). Amber was the first choice and was wrong — the
/// line's "a bit flat" is `(236, 150, 72)`, within a few units of it, so a mark and a
/// slightly sour note would have looked alike **exactly** when the user turns the line
/// on to compare them against each other, which is the one moment the picture has to be
/// unambiguous. Violet appears nowhere else in the roll, so anything violet is the
/// player speaking, not the detector.
const MARK_BAR_H: f32 = 9.0;
const MARK_TICK_H: f32 = 7.0;
const MARK_FILL: Color32 = Color32::from_rgba_premultiplied(92, 48, 122, 175);
const MARK_EDGE: Color32 = Color32::from_rgb(198, 128, 255);
/// The boundary already clicked, waiting for its pair.
const MARK_DRAFT: Color32 = Color32::from_rgb(198, 128, 255);

/// The frozen roll's state.
pub struct TakeRoll {
    /// The take on screen. `None` = nothing has been replayed this session, which is a
    /// real state and not a missing value: the panel has nothing to draw and says so.
    ///
    /// Everything that only means something *with* a take lives inside, so that none of
    /// it can be read as a zero when there is no take — a take is never 0.0 s long, and
    /// a field claiming so would be a lie the type system was helping to tell.
    loaded:    Option<LoadedTake>,
    /// Which layer is painted under the line. Outside [`LoadedTake`] because it is a
    /// property of the *looking*, not of the take: having chosen to read the salience,
    /// you mean to keep reading it as you step through take after take.
    layer:     RollLayer,
    /// The declaration, as typed — "F5", "G3". Parsed every frame into a
    /// [`Declaration`]; the text is what the user is editing, and holding the parsed
    /// form here instead would mean deciding what a half-typed "F" means.
    ///
    /// Outside [`LoadedTake`], like the layer: you declare what you are about to play
    /// and then play it, so the declaration outlives any one take on screen.
    declared:  String,
    /// Whether the detector's **line** is painted. See [`MarkedAgainst`] — this is the
    /// mirror, and marking is what it must not be in the way of.
    ///
    /// `true` by default because reading a take without marking it is triage, the panel's
    /// first job and the one that found the open-G sub-octave. Arming a declaration
    /// turns it off (see [`TakeRoll::armed`]).
    show_line: bool,
    /// Was a declaration armed on the previous frame?
    ///
    /// The line is hidden on the *transition* into marking, not on every frame that
    /// marking is armed — otherwise turning the line back on would be impossible, and
    /// the user's own stated workflow («размечаю поверх того что задетектилось») would
    /// be forbidden rather than merely discouraged. What it costs them is recorded, not
    /// prevented: the mark carries [`MarkedAgainst::TheLineToo`].
    was_armed: bool,
}

impl Default for TakeRoll {
    fn default() -> Self {
        Self {
            loaded:    None,
            layer:     RollLayer::default(),
            declared:  String::new(),
            show_line: true,
            was_armed: false,
        }
    }
}

impl TakeRoll {
    /// The declared note, if the field holds one — and therefore whether clicking the
    /// roll marks anything.
    ///
    /// Marking is armed by the declaration alone, which is the whole design in one line:
    /// **there is no way to make a mark without having said what you played first.** Not
    /// a validation — a construction. You cannot click a note's boundaries into being
    /// and then look up at the line to decide what note it was.
    fn armed(&self) -> Option<Declaration> {
        Declaration::parse(&self.declared)
    }
}

/// The resonator bank's pitch range, in fractional MIDI — the bounds of where evidence
/// can exist at all.
///
/// Its own type rather than a `Range<f32>` so it cannot be mixed up with the *window*
/// (also a pair of MIDI numbers, and the thing it bounds), and because it is `Copy`:
/// it is read once per frame from the settings and threaded through the interaction as
/// a fact about the detector, not as a value anything here may edit.
#[derive(Clone, Copy)]
struct BankRange {
    lo: f32,
    hi: f32,
}

impl BankRange {
    fn span(self) -> f32 {
        self.hi - self.lo
    }
}

/// Who decides the pitch window.
///
/// The auto framing is **load-bearing, not a convenience**: it is min/max over the
/// whole take (see the module docs), and that is the only reason the open-G sub-octave
/// was ever seen — the live roll's quantile framing puts the slip *below the window*
/// (`memory/open_g_sub_octave_bursts.md`). So a hand-aimed pitch window must not become
/// the default by accident: you get it by asking, and a double-click gives it back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PitchFraming {
    /// Framed on the take's whole played range, re-framed as the line arrives.
    Auto,
    /// The user framed it by hand. [`LoadedTake::reframe`] must not steal it back —
    /// the take streams in over its whole length, so an auto-frame that ignored this
    /// would yank the window out from under a user who aimed it while the take played.
    Manual,
}

/// Everything a click needs to become a mark: what was declared, and what was on screen
/// while the user was deciding.
///
/// The two travel together because they are recorded together and neither means much
/// alone — the declaration is the mark's pitch, and [`MarkedAgainst`] is the honest note
/// about how the timing was arrived at. Passing them as one value is also what stops a
/// caller from arming the marking while forgetting to say what the user could see.
#[derive(Clone, Copy)]
struct Marking {
    declared: Declaration,
    against:  MarkedAgainst,
}

/// Which axis a wheel zoom moves — the roll zooms one at a time, see
/// [`LoadedTake::interact`].
///
/// Latched rather than read per frame, and that is not a style choice: egui smooths one
/// notch of the wheel over several frames, and the frames after the first carry no event
/// and no modifier — by then the user may well have let go of shift. Reading the live
/// modifier every frame puts the head of one flick on one axis and its tail on the other.
/// egui hit this first and latches the wheel's modifiers at the notch for exactly this
/// reason (`WheelState::modifiers`: *«If the user lets go of a modifier - ignore it»*),
/// but keeps them private, so the latch is ours to keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum ZoomAxis {
    /// Plain ctrl+wheel. The default because time is what a take is read along.
    #[default]
    Time,
    /// shift+wheel — ctrl may be held too, but does not have to be, and that is the
    /// whole point: see the taking in [`LoadedTake::interact`].
    Pitch,
}

/// A take, and the window onto it.
struct LoadedTake {
    /// Which take this is — the identity the harvest compares against to notice that a
    /// different one started playing.
    path:       PathBuf,
    /// The take's length in seconds, from the corpus listing (the WAV's own header).
    /// This is what "the whole take" means for fitting and for clamping the window.
    seconds:    f32,
    /// Every frame the engine published for this take, oldest → newest — the take's
    /// line, entire. See [`TAKE_RETENTION_S`].
    frames:     MelodyHistory,
    /// How far this panel has read the engine's history. Ours alone, so the live roll
    /// reads the same frames with its own (see `AudioEngine::melody_since`).
    cursor:     Option<u64>,
    /// The visible slice, **in take seconds** — 0.0 is the take's first sample.
    ///
    /// Take seconds rather than ages, because that is the frame the user reasons in
    /// («слип на третьей секунде») and it is the only one that holds still: an age is
    /// measured from the playhead, and the playhead moves while the take plays. The
    /// conversion to the renderer's ages happens once, in [`Self::time_axis`].
    window:     Range<f32>,
    /// Fractional-MIDI window at the plot's bottom/top edges, framed on the take's full
    /// played range (see the module docs on why full, not quantiles).
    ///
    /// Always the *effective* window, whoever framed it — the renderer asks one
    /// question ("what is at the edges") and must get one answer. Who chose it is
    /// [`Self::framing`], and that is a separate question.
    view_lo:    f32,
    view_hi:    f32,
    /// See [`PitchFraming`].
    framing:    PitchFraming,
    /// The player's own truth about this take, loaded from `<take>.marks.jsonl` when it
    /// opened and written back on every change. See [`crate::app::take_marks`].
    marks:      Vec<NoteMark>,
    /// The click-click gesture in progress: the take-second the first click fixed,
    /// waiting for the second.
    ///
    /// `Option` earns its place here — "no gesture in progress" is the resting state of
    /// a gesture, not a value we are missing. Click-click rather than drag, copied from
    /// `main_app`'s annotator (`ui/notes/annotate.rs`): between the two clicks the user
    /// is free to scroll and zoom, which matters because a note's two ends are often not
    /// on screen at the same magnification.
    draft:      Option<f64>,
    /// Which axis the wheel's zoom is aimed at, latched at the notch — see
    /// [`Self::interact`], which is also the only thing that writes it.
    zoom_axis:  ZoomAxis,
    /// Why the last write of the marks file failed, if it did. `None` = the file on disk
    /// matches what is on screen.
    ///
    /// Kept and shown rather than logged: marking into a read-only corpus looks exactly
    /// like marking into a working one, and the user would find out an evening later.
    save_error: Option<String>,
}

impl LoadedTake {
    fn new(path: PathBuf, seconds: f32) -> Self {
        Self {
            // The take's marks come with the take, from beside its WAV: they are part of
            // the corpus, not of this session, and a take opened on a different machine
            // a year from now must arrive with its truth attached.
            marks: load_marks(&path),
            draft: None,
            zoom_axis: ZoomAxis::default(),
            save_error: None,
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
            framing: PitchFraming::Auto,
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
    ///
    /// A hand-framed window is left alone — see [`PitchFraming::Manual`].
    fn reframe(&mut self) {
        if self.framing == PitchFraming::Manual {
            return;
        }

        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for frame in self.frames.iter() {
            if let Some(pitch) = frame.pitch {
                lo = lo.min(pitch);
                hi = hi.max(pitch);
            }
        }
        // 🔑 The player's marks are framed too, and this is not a nicety.
        //
        // Framing on the detector's line alone puts the user's own answer off screen in
        // precisely the case this panel exists for: declare F5, watch the detector
        // insist on F4, and the framing — computed from *its* pitches — never includes
        // row 77 at all. The marks would be invisible exactly when they disagree, which
        // is when they matter. A take's evidence is the line; a take's truth is the
        // marks; the window has to hold both.
        for mark in &self.marks {
            lo = lo.min(mark.midi as f32);
            hi = hi.max(mark.midi as f32);
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

    /// One click of the click-click marking gesture: the first fixes a boundary, the
    /// second closes the mark.
    ///
    /// **Only the time is taken from the click. The pitch is the declaration's**, and
    /// that is the whole design: the user says "F5" before playing, then says *when* —
    /// so the answer never passes through an eye looking at the detector's output. See
    /// [`crate::app::take_marks`].
    ///
    /// A zero-length mark is dropped rather than stored. Two clicks in the same place is
    /// how a mis-click looks, not how a note does; and an empty interval would match
    /// nothing in Ф3's scoring while still sitting in the file looking like evidence.
    fn mark_click(&mut self, t: f64, marking: Marking) {
        let t = t.clamp(0.0, self.seconds as f64);
        let Some(start) = self.draft.take() else {
            self.draft = Some(t);
            return;
        };
        let mark = NoteMark::new(start, t, marking.declared, marking.against);
        if mark.seconds() <= 0.0 {
            return; // a mis-click, not a note
        }
        self.marks.push(mark);
        // Re-frame now, not on the next batch of frames: a finished take publishes
        // nothing ever again, so `update` would never run and a mark declared outside
        // the line's range would stay invisible for good. See [`Self::reframe`].
        self.reframe();
        self.persist();
    }

    /// Drop the newest mark. The undo for a boundary put in the wrong place — which is
    /// most of what goes wrong while marking, and is otherwise unfixable without leaving
    /// the app to edit the file by hand.
    fn undo_mark(&mut self) {
        self.marks.pop();
        // Symmetric with `mark_click`: the framing counts the marks, so dropping one
        // that had stretched the window must let it close again.
        self.reframe();
        self.persist();
    }

    /// Write the marks back beside the WAV, now.
    ///
    /// On every change rather than on some "save" — these are hand-made and cannot be
    /// regenerated by re-running anything, so the cost of losing them to a crash is an
    /// evening of playing, while the cost of writing tens of lines of JSON is nothing.
    ///
    /// A failure is surfaced, not swallowed: [`Self::save_error`] puts it on screen. If
    /// the corpus directory is read-only the user is marking into a void, and the one
    /// thing worse than that is not being told.
    fn persist(&mut self) {
        self.save_error = save_marks(&self.path, &self.marks).err();
    }

    /// Paint the marks, and the half-made one.
    ///
    /// On top of the roll rather than inside `ui::pianoroll`, because a mark is not a
    /// layer of the picture — it is the *answer* about the picture, and it belongs to
    /// this panel alone. Placed through [`pianoroll::RollMapping`] though, and pointedly
    /// not by arithmetic of its own: a mark drawn a few pixels off the heat it points at
    /// would misrepresent the very thing it is evidence about.
    ///
    /// A mark is a bar on its **declared** row. That it may sit nowhere near the line is
    /// not a rendering problem — it is the finding.
    fn draw_marks(&self, painter: &egui::Painter, map: pianoroll::RollMapping, playhead_t: f64) {
        let plot = map.plot();
        // Take-second → the renderer's age: the mapping speaks ages, the marks speak
        // take time, and `playhead_t` is the one number that converts between them.
        let x_of_t = |t: f64| map.x_of((playhead_t - t) as f32);

        for mark in &self.marks {
            let y = map.y_of(mark.midi as f32);
            if y < plot.top() || y > plot.bottom() {
                continue; // scrolled out of the pitch window
            }
            let (x0, x1) = (x_of_t(mark.interval.start), x_of_t(mark.interval.end));
            if x1 < plot.left() || x0 > plot.right() {
                continue; // scrolled out of the time window
            }
            let bar = Rect::from_min_max(
                egui::pos2(x0.max(plot.left()), y - MARK_BAR_H * 0.5),
                egui::pos2(x1.min(plot.right()), y + MARK_BAR_H * 0.5),
            );
            painter.rect_filled(bar, 2.0, MARK_FILL);
            // The ends are what the user actually placed, so they are drawn as the
            // crisp thing and the bar as the soft one: a boundary is a claim, its
            // interior is just the consequence.
            for x in [x0, x1] {
                if x >= plot.left() && x <= plot.right() {
                    painter.line_segment(
                        [egui::pos2(x, y - MARK_TICK_H), egui::pos2(x, y + MARK_TICK_H)],
                        Stroke::new(1.5, MARK_EDGE),
                    );
                }
            }
        }

        // The half-made mark: a full-height line at the boundary already fixed. Full
        // height because it has no pitch yet to sit at — the pitch arrives with the
        // declaration, not with the click, and drawing it at the declared row would
        // quietly claim the second click had already happened.
        if let Some(start) = self.draft {
            let x = x_of_t(start);
            if x >= plot.left() && x <= plot.right() {
                painter.line_segment(
                    [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                    Stroke::new(1.5, MARK_DRAFT),
                );
            }
        }
    }

    /// Show the take whole, on both axes: the entire time span, and the pitch range
    /// framed on the take's own data again.
    ///
    /// Both, because "fit" answers one question — *show me what I recorded* — and an
    /// escape hatch that only undid half of a lost window would leave the user hunting
    /// for the other half by hand.
    fn fit(&mut self) {
        self.window = 0.0..self.seconds.max(MIN_WINDOW_S);
        self.framing = PitchFraming::Auto;
        self.reframe();
    }

    /// Visible pitch span in semitones — what one pixel of height is worth.
    fn pitch_span(&self) -> f32 {
        self.view_hi - self.view_lo
    }

    /// Slide the pitch window by `delta_midi` semitones. Hand-aiming it, so the auto
    /// framing stands down (see [`PitchFraming`]).
    fn pan_pitch(&mut self, delta_midi: f32, bank: BankRange) {
        self.framing = PitchFraming::Manual;
        self.view_lo += delta_midi;
        self.view_hi += delta_midi;
        self.clamp_pitch(bank);
    }

    /// Multiply the pitch span by `factor`, keeping `anchor_midi` where it is on screen.
    /// Anchored for the same reason the time zoom is: you zoom to look *at* something.
    fn zoom_pitch_about(&mut self, factor: f32, anchor_midi: f32, bank: BankRange) {
        self.framing = PitchFraming::Manual;
        let fraction = (anchor_midi - self.view_lo) / self.pitch_span();
        let span = self.pitch_span() * factor;
        self.view_lo = anchor_midi - span * fraction;
        self.view_hi = self.view_lo + span;
        self.clamp_pitch(bank);
    }

    /// Keep the hand-aimed pitch window inside the bank's range and wider than
    /// [`MIN_PITCH_SPAN`].
    ///
    /// Bounded by the **bank**, and by the same argument that bounds time to the take
    /// (see [`Self::clamp`]): outside the bank's range there is not quiet evidence,
    /// there is no evidence — the bank has no filters there, so it publishes nothing,
    /// and rows of guaranteed-empty grid would read as "nothing sounded here" when the
    /// truth is "nothing could have been heard here".
    ///
    /// Only the manual path clamps. The auto framing is computed *from* pitches the
    /// bank published, so it is inside the range by construction — and running it
    /// through this would let a clamp silently overrule the framing that is the whole
    /// reason this panel exists.
    fn clamp_pitch(&mut self, bank: BankRange) {
        let whole = bank.span().max(MIN_PITCH_SPAN);
        let span = self.pitch_span().clamp(MIN_PITCH_SPAN, whole);
        // Slide, don't squash — same rule as the time axis: hitting an edge must not
        // silently rescale the zoom the user chose.
        let lo = self.view_lo.clamp(bank.lo, (bank.hi - span).max(bank.lo));
        self.view_lo = lo;
        self.view_hi = lo + span;
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

    /// Pan on both axes at once; zoom one axis at a time.
    ///
    /// ## The wheel is the pane's, and the roll does not touch it
    ///
    /// This panel lives inside the workspace pane's `ScrollArea` (`app::workspace`), and
    /// it is tall — controls, corpus listing, then a 320 px roll — so the pane genuinely
    /// has to scroll, and the roll covers most of the surface the wheel would land on.
    ///
    /// It used to *take* `smooth_scroll_delta.y` on hover and zoom with it, which left
    /// the pane a zero and made the panel unscrollable over its own main body: the roll
    /// was at once the reason to scroll and the thing preventing it. Now the **plain
    /// wheel is simply not ours** — it scrolls the pane, untouched — and the roll zooms
    /// on **ctrl+wheel** (time), which is what egui_plot does and therefore what the
    /// gesture already means to anyone who has used a plot.
    ///
    /// That much is taken from no one, and structurally rather than politely: egui routes
    /// a wheel with the zoom modifier into the zoom factor and leaves
    /// `smooth_scroll_delta` at **zero** (`egui::InputState::begin_pass`). The two
    /// gestures cannot both fire, so there is nothing left to arbitrate.
    ///
    /// ⚠ **shift+wheel is the exception, and it is a real one.** egui has no zoom
    /// modifier to spare here — shift is its *horizontal-scroll* modifier — so the pitch
    /// zoom the user asked for by name does have to take `smooth_scroll_delta.x` from the
    /// pane. Kept as narrow as the claim: only `.x`, only over the roll, only while the
    /// latch says the run began with shift. Sideways scrolling elsewhere in the app, and
    /// over this panel's own controls, is untouched — pinned by
    /// [`tests::shift_wheel_off_the_roll_still_scrolls_the_pane_sideways`].
    ///
    /// `bank` is the resonator's pitch range — the bounds of where evidence can exist at
    /// all (see [`Self::clamp_pitch`]). `marking` is the armed declaration, if any — see
    /// [`Self::mark_click`].
    fn interact(&mut self, ui: &Ui, resp: &Response, plot: Rect, bank: BankRange, marking: Option<Marking>) {
        let px_per_second = plot.width() / self.span_s();
        let t_at = |x: f32| self.window.start as f64 + ((x - plot.left()) / px_per_second) as f64;

        // Double-click = "show me the whole thing" — the way back from any window,
        // without hunting for a button, and the way back to auto pitch framing.
        //
        // 🔑 It comes first, and it wins over marking, because the two gestures cannot
        // collide: a double-click is two clicks in the *same place* within egui's
        // double-click window, and a note's two ends are never in the same place — a
        // note has duration. So the gesture that fits is precisely the gesture that
        // could not have been a mark, and nothing has to be arbitrated. A half-made
        // mark is dropped, which is what asking for the whole take back means.
        //
        // The corollary is worth stating: a note too short to have two *distinguishable*
        // ends at the current zoom cannot be marked at that zoom. That is honest — at
        // that zoom the user cannot see where its ends are either, and the fit they get
        // instead is visible and undoable, not a silently wrong mark.
        if resp.double_clicked() {
            self.draft = None;
            self.fit();
            return;
        }

        if let Some(marking) = marking
            && resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.mark_click(t_at(pos.x), marking);
            return;
        }

        let px_per_second = plot.width() / self.span_s();
        let px_per_semitone = plot.height() / self.pitch_span();
        if resp.dragged() {
            let drag = resp.drag_delta();
            // Drag moves the *take*, not the window: pull right and the take goes right,
            // which means the window walks left, into the past. Same on the pitch axis,
            // with the sign the other way because screen y grows downward while pitch
            // grows upward — drag the take down and you are looking higher.
            self.pan(-drag.x / px_per_second);
            if drag.y != 0.0 {
                self.pan_pitch(drag.y / px_per_semitone, bank);
            }
        }

        // Zoom about the cursor, which has to be inside the plot for "about the cursor"
        // to mean anything.
        let Some(pos) = resp.hover_pos() else {
            return;
        };
        // One axis per gesture: **ctrl+wheel = time, shift+wheel = pitch.** The two axes
        // of this roll answer different questions — "when did the note start" and "what
        // did the detector think it was" — and zooming both at once means never getting
        // to ask either one: the note you zoomed into on the time axis walks off the top
        // while you do it. Shift for the second axis is the user's convention across
        // their tools, asked for by name (07-15).
        //
        // 🔑 This **overrides** egui's own split, which is why the reading is by hand.
        // `zoom_delta_2d` splits on the horizontal/vertical *scroll* modifiers — shift =
        // x, alt = y — so inheriting it would give shift the axis the user wants alt to
        // have, and alt is the one modifier a Linux WM is likely to eat before the app
        // ever sees it. The scalar `zoom_delta` is unaffected by either modifier (the
        // zoom factor sums both wheel components, so shift's collapse of the wheel onto
        // its x component cannot reach it), which is what makes it safe to route here.
        //
        // The axis comes off the **wheel event's own modifiers**, not off the live
        // keyboard, and is latched — see [`ZoomAxis`], which is where that is argued.
        //
        // A real pinch keeps `zoom_delta_2d`: two fingers *measure* both axes, so their
        // split is data, not a convention we get to pick, and there is no modifier in the
        // gesture to latch.
        let zoom_speed = ui
            .ctx()
            .options(|options| options.input_options.scroll_zoom_speed);
        let (pinch, factor, scroll, notch) = ui.input(|input| {
            (
                input.multi_touch().map(|touch| touch.zoom_delta_2d),
                input.zoom_delta(),
                input.smooth_scroll_delta,
                input.events.iter().rev().find_map(|event| {
                    match event {
                        Event::MouseWheel { modifiers, .. } => Some(modifiers.shift),
                        _ => None,
                    }
                }),
            )
        });
        if let Some(shift) = notch {
            self.zoom_axis = if shift { ZoomAxis::Pitch } else { ZoomAxis::Time };
        }
        // A factor rather than a wheel delta of our own: a notch multiplies the span, so
        // zooming out undoes zooming in exactly.
        let zoom = match (pinch, self.zoom_axis) {
            (Some(pinch), _) => pinch,
            (None, ZoomAxis::Time) => Vec2::new(factor, 1.0),
            // ⚠ **The one gesture the roll takes, and it has to.** ctrl is egui's zoom
            // modifier, so a ctrl+wheel arrives as a factor with the scroll already
            // zeroed and nothing to arbitrate. A **plain shift+wheel is not a zoom to
            // egui at all**: shift is its *horizontal-scroll* modifier, so the notch
            // arrives as `smooth_scroll_delta.x` and the pane's `ScrollArea::both` would
            // scroll sideways on the very gesture the user aimed at the roll. Taking it
            // is what makes shift alone mean pitch.
            //
            // Narrow on purpose, because taking the wheel is exactly how the pane got
            // broken before (see above): only `.x`, only while the latch says this run
            // began with shift, and only here — over the roll. The plain wheel is
            // untouched, and shift+wheel anywhere else in the app still scrolls its pane.
            //
            // `factor` carries the ctrl+shift case, where egui *did* mint one and the
            // scroll is already zero; `scroll.x` carries plain shift, where `factor` is
            // 1.0. Multiplying covers both without asking which happened. The formula is
            // egui's own (`InputState::begin_pass`), so a notch of pitch zoom is exactly
            // the size of a notch of time zoom — the two axes must feel like one gesture
            // with a modifier, not like two different tools.
            (None, ZoomAxis::Pitch) => {
                if scroll.x != 0.0 {
                    ui.ctx().input_mut(|input| input.smooth_scroll_delta.x = 0.0);
                }
                Vec2::new(1.0, factor * (zoom_speed * scroll.x).exp())
            }
        };
        if zoom == Vec2::splat(1.0) {
            return; // no zoom gesture this frame — and the wheel, if any, is the pane's
        }
        if zoom.x != 1.0 {
            let anchor_s = self.window.start + (pos.x - plot.left()) / px_per_second;
            // Spread (zoom > 1) = zoom in = the span shrinks, hence the reciprocal.
            self.zoom_about(zoom.x.recip(), anchor_s);
        }
        if zoom.y != 1.0 {
            // Screen y grows downward; pitch grows upward — so the anchor is measured
            // from the plot's *bottom*.
            let anchor_midi = self.view_lo + (plot.bottom() - pos.y) / px_per_semitone;
            self.zoom_pitch_about(zoom.y.recip(), anchor_midi, bank);
        }
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

    /// The declaration row: what you played, said before you mark when.
    ///
    /// This row **is** the defence against the mirror, so it comes before the roll and
    /// not in a menu: the order on screen is the order of the workflow. Declare, then
    /// mark. The field is small because the claim is small — a note name, the same one
    /// already in the take's file name.
    fn draw_declaration(&mut self, ui: &mut Ui) {
        let armed = self.take_roll.armed();
        // Arming is the moment marking begins, and the moment the line stops being a
        // help and starts being an answer sheet. Flip it off *once*, here, rather than
        // hold it off: the user may turn it back on (their stated workflow), and the
        // mark records that they did.
        if armed.is_some() && !self.take_roll.was_armed {
            self.take_roll.show_line = false;
        }
        self.take_roll.was_armed = armed.is_some();

        ui.horizontal(|ui| {
            ui.add(RowCaption::new("Played"));
            ui.add(
                egui::TextEdit::singleline(&mut self.take_roll.declared)
                    .desired_width(80.0)
                    .hint_text("F5"),
            );

            let (text, tint) = match armed {
                Some(declared) => {
                    (
                        format!(
                            "{} — click the note's start, then its end. The pitch is yours, not \
                             the detector's",
                            declared.name()
                        ),
                        color::STATUS_LISTENING,
                    )
                }
                // Not an error — the resting state. Marking is armed by a declaration
                // and by nothing else, so with the field empty a click is just a click.
                None if self.take_roll.declared.trim().is_empty() => {
                    (
                        "Say what you played (e.g. F5) — then mark only *when* it sounded".to_owned(),
                        color::TEXT_HINT,
                    )
                }
                None => {
                    (
                        format!("\"{}\" is not a note name", self.take_roll.declared.trim()),
                        color::STATUS_ERROR,
                    )
                }
            };
            ui.label(RichText::new(text).color(tint).size(12.0));
        });

        let Some(loaded) = self.take_roll.loaded.as_mut() else {
            return;
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let marks = loaded.marks.len();
            ui.label(
                RichText::new(match marks {
                    0 => "no marks yet".to_owned(),
                    1 => "1 mark".to_owned(),
                    n => format!("{n} marks"),
                })
                .color(color::TEXT_HINT)
                .monospace()
                .size(11.0),
            );
            if ui
                .add_enabled(marks > 0, egui::Button::new("Undo last"))
                .clicked()
            {
                loaded.undo_mark();
            }
            if loaded.draft.is_some() {
                ui.label(
                    RichText::new("…click the other end (esc to drop it)")
                        .color(color::STATUS_LISTENING)
                        .size(11.0),
                );
                // Escape drops a half-made mark. Without it the only way out of a
                // mis-click is to complete a mark you do not want and undo it.
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    loaded.draft = None;
                }
            }
            if let Some(error) = &loaded.save_error {
                ui.label(
                    RichText::new(format!("marks NOT saved: {error}"))
                        .color(color::STATUS_ERROR)
                        .size(11.0),
                );
            }
        });
    }

    /// The frozen roll: the take, with the window the user put on it.
    pub(super) fn draw_take_roll(&mut self, ui: &mut Ui) {
        let settings = self.audio.analysis_settings();
        let style = settings.accidental;
        let res_min_midi = settings.resonator.min_midi.as_u8() as i32;
        let res_max_midi = settings.resonator.max_midi.as_u8() as i32;
        // Where evidence can exist: outside the bank's filters nothing is ever
        // published, so the hand-aimed pitch window stops here. See `BankRange`.
        let bank = BankRange {
            lo: res_min_midi as f32,
            hi: res_max_midi as f32,
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new("Take Roll").color(color::TEXT_CAPTION).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The line is a layer you switch on, not part of the picture: it is the
                // detector's answer, and it is the thing a marker would agree with.
                ui.selectable_value(&mut self.take_roll.show_line, true, "line");
                ui.selectable_value(&mut self.take_roll.show_line, false, "no line");
                ui.separator();
                ui.selectable_value(&mut self.take_roll.layer, RollLayer::Salience, "SWIPE′");
                ui.selectable_value(&mut self.take_roll.layer, RollLayer::Spectrum, "spectrum");
            });
        });
        ui.add_space(6.0);
        self.draw_declaration(ui);
        ui.add_space(6.0);

        let layer = self.take_roll.layer;
        let marking = self.take_roll.armed().map(|declared| {
            Marking {
                declared,
                // Recorded, not prevented: the user asked to mark over the detected line
                // and may do so — but the mark says it was made that way, and Ф3 can
                // then score the clean marks separately and find out whether it mattered.
                against: if self.take_roll.show_line {
                    MarkedAgainst::TheLineToo
                } else {
                    MarkedAgainst::Evidence
                },
            }
        });
        let show_line = self.take_roll.show_line;
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
        loaded.interact(ui, &resp, plot, bank, marking);

        let painter = ui.painter_at(rect);
        let time = loaded.time_axis(playhead_t);
        pianoroll::draw_pitch_roll(
            &painter,
            rect,
            &loaded.columns(playhead_t, layer),
            res_min_midi,
            res_max_midi,
            loaded.view_lo,
            loaded.view_hi,
            time,
            style,
            show_line,
        );
        // The marks go on top, through the renderer's own mapping — see `draw_marks`.
        loaded.draw_marks(
            &painter,
            pianoroll::RollMapping::new(rect, loaded.view_lo, loaded.view_hi, time),
            playhead_t,
        );

        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} · {:.2}–{:.2} s of {:.1} · {:.0}–{:.0} midi{} · drag to pan, ctrl+wheel to \
                 zoom, double-click to fit{}",
                loaded.path.file_name().unwrap().to_string_lossy(),
                loaded.window.start,
                loaded.window.end,
                loaded.seconds,
                loaded.view_lo,
                loaded.view_hi,
                // Say when the pitch rows are the user's own window rather than the
                // take's range: the min/max auto framing is what makes a slip visible
                // at all, so "you are not looking at it right now" is worth a word.
                match loaded.framing {
                    PitchFraming::Auto => "",
                    PitchFraming::Manual => " (manual)",
                },
                // Naming the marking gesture only while it is armed: with no
                // declaration a click does nothing, and offering it would be a lie.
                if marking.is_some() {
                    ", click-click to mark"
                } else {
                    ""
                },
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

    use eframe::egui::{
        self,
        Event,
        Modifiers,
        MouseWheelUnit,
        Pos2,
        TouchPhase,
    };
    use egui_kittest::Harness;
    use web_time::Instant;

    use super::*;
    use crate::audio::AudioEngine;
    use crate::core_types::note::AccidentalStyle;

    /// A take: `seconds` long, replayed at the bank's cadence.
    const CADENCE_S: f64 = 0.016;

    /// The bank's default range (G2ish..C7ish) — the pitch clamp's bounds.
    const BANK: BankRange = BankRange { lo: 43.0, hi: 96.0 };

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

    /// The panel's *wiring*, driven through real egui: a pane-sized `ScrollArea::both`
    /// with content taller than it, the roll allocated inside exactly as
    /// [`App::draw_take_roll`] does it.
    ///
    /// 🔑 **This exists because every other test in this file structurally cannot fail
    /// on the two bugs the user found in a minute.** They all call `pan`/`zoom_about`/
    /// `clamp` directly — window arithmetic, which was correct throughout. The bugs were
    /// in `interact` (which delta is taken from whom) and in the layout (a tall panel
    /// inside a `ScrollArea`), and neither is reachable from a test that never builds a
    /// `Ui` or sends an event. Eight tests, four of them proven against live bugs, and
    /// the panel was still unusable (`memory/interaction-needs-kittest-not-state-tests.md`).
    struct Rig {
        take:     LoadedTake,
        /// The armed declaration, if the test is marking. `None` = reading, which is
        /// what every pan/zoom test wants: a click must not quietly become a mark.
        marking:  Option<Marking>,
        /// Where the pane has scrolled to — the thing the roll used to eat.
        offset_y: f32,
        /// The pane's *sideways* offset. Watched for the mirror-image reason: pitch zoom
        /// takes `smooth_scroll_delta.x`, so a shift+wheel that both zooms and scrolls
        /// the pane sideways is the old bug wearing the other axis.
        offset_x: f32,
        /// The roll's plot rect, recorded from inside the layout so the test aims the
        /// pointer at where the roll actually *is* rather than at where arithmetic
        /// says it should be. Getting that wrong is how a test hovers over nothing and
        /// then reports that nothing happened.
        plot:     Rect,
    }

    /// Stand-in for the controls + corpus listing above the roll. Its only job is to be
    /// tall: it is what pushes the panel past the pane's height and therefore what makes
    /// the pane need to scroll at all — which is the entire premise of the regression.
    const HEADER_H: f32 = 260.0;
    const PANE_W: f32 = 700.0;
    const PANE_H: f32 = 420.0;

    fn rig(take: LoadedTake) -> Harness<'static, Rig> {
        Harness::builder()
            .with_size(egui::Vec2::new(PANE_W, PANE_H))
            // A real frame time. The default is 0.25 s/step, which is *longer than
            // egui's double-click window* (0.3 s for two clicks, i.e. two steps) — so a
            // harness at the default cannot produce a double-click at all, and any test
            // asserting about one quietly passes by never testing it. Found exactly that
            // way.
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                |ui, rig: &mut Rig| {
                    // The workspace pane, as `app::workspace::pane_ui` builds it.
                    let out = egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.allocate_exact_size(vec2(PANE_W, HEADER_H), Sense::hover());
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(PANE_W - 20.0, PLOT_HEIGHT),
                                Sense::click_and_drag(),
                            );
                            rig.plot = pianoroll::plot_rect(rect);
                            rig.take.interact(ui, &resp, rig.plot, BANK, rig.marking);
                        });
                    rig.offset_y = out.state.offset.y;
                    rig.offset_x = out.state.offset.x;
                },
                Rig {
                    take,
                    marking: None,
                    offset_y: 0.0,
                    offset_x: 0.0,
                    plot: Rect::ZERO,
                },
            )
    }

    /// A point on the roll that is **actually on screen**.
    ///
    /// The roll's rect runs past the bottom of the pane — with the controls above it the
    /// panel is ~680 px inside a 420 px pane, so the lower half of the roll is below the
    /// fold. That is not a quirk of the rig, it *is* the situation the regression is
    /// about: the panel has to be scrolled to see the rest of the roll, and the wheel
    /// lands on the part you can see. Aiming at `plot.center()` aims off-screen, where
    /// nothing is hovered and every assertion trivially "passes" the wrong way.
    fn on_screen(plot: Rect) -> Pos2 {
        egui::pos2(plot.center().x, plot.top() + 40.0)
    }

    /// One wheel notch over a position, with or without the zoom modifier.
    fn wheel(harness: &Harness<'_, Rig>, at: Pos2, delta_y: f32, modifiers: Modifiers) {
        harness.hover_at(at);
        harness.event_modifiers(
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: egui::Vec2::new(0.0, delta_y),
                phase: TouchPhase::Move,
                modifiers,
            },
            modifiers,
        );
    }

    /// REGRESSION (mine, found by the user in a minute): **the plain wheel belongs to the
    /// pane.**
    ///
    /// The roll used to take `smooth_scroll_delta.y` on hover and zoom with it, leaving
    /// the `ScrollArea` a zero. The panel is ~680 px of controls + listing + roll inside a
    /// pane that is rarely that tall, so it has to scroll — and the roll covers most of
    /// the surface the wheel lands on. The roll was at once the reason to scroll and the
    /// thing that made it impossible: *«рекордер - не скролится по вертикали-горизонтали»*.
    ///
    /// Both halves are asserted, because "the pane scrolls" alone would still pass if the
    /// roll *also* zoomed — which is the two-things-at-once behaviour that made the
    /// original taking look necessary.
    #[test]
    fn the_plain_wheel_scrolls_the_pane_and_the_roll_keeps_its_hands_off() {
        let mut harness = rig(take(10.0));
        harness.run_steps(3); // settle the layout, so `plot` is real
        let plot = harness.state().plot;
        let window_before = harness.state().take.window.clone();
        assert_eq!(harness.state().offset_y, 0.0, "the pane starts unscrolled");

        wheel(&harness, on_screen(plot), -50.0, Modifiers::NONE);
        harness.run_steps(6); // egui smooths the wheel over several frames

        assert!(
            harness.state().offset_y > 0.0,
            "the pane did not scroll ({} px) — the roll ate the wheel again",
            harness.state().offset_y
        );
        assert_eq!(
            harness.state().take.window,
            window_before,
            "the plain wheel must not zoom the roll: it is the pane's gesture"
        );
    }

    /// The other half: **ctrl+wheel zooms the roll's time axis, and the pane holds
    /// still.**
    ///
    /// Nothing has to be taken for this to work, and that is the point. egui routes a
    /// wheel carrying the zoom modifier into the zoom factor and leaves
    /// `smooth_scroll_delta` at zero, so the two gestures cannot both fire — the
    /// arbitration the old code was doing by hand does not exist to be got wrong. This
    /// test is what pins that contract: it is inherited from egui, so nothing in this
    /// repository would otherwise notice it changing.
    ///
    /// The pitch span is asserted *unchanged* here, and that half is not inherited — it
    /// is the axis split (see [`ctrl_shift_wheel_zooms_pitch_and_leaves_time_alone`]).
    #[test]
    fn ctrl_wheel_zooms_time_alone_and_the_pane_holds_still() {
        let mut harness = rig(take(10.0));
        harness.run_steps(3);
        let plot = harness.state().plot;
        let (span_before, pitch_before) = {
            let take = &harness.state().take;
            (take.span_s(), take.pitch_span())
        };

        wheel(&harness, on_screen(plot), 50.0, Modifiers::COMMAND);
        harness.run_steps(6);

        let take = &harness.state().take;
        let span_after = take.span_s();
        assert!(
            span_after < span_before - 0.01,
            "ctrl+wheel must zoom the roll in: span {span_before:.3} → {span_after:.3} s"
        );
        assert!(
            (take.pitch_span() - pitch_before).abs() < 0.01,
            "ctrl+wheel is the *time* axis alone: pitch span {pitch_before:.2} → {:.2} semitones",
            take.pitch_span()
        );
        assert_eq!(
            harness.state().offset_y,
            0.0,
            "the pane must not scroll while the roll zooms — one gesture, one effect"
        );
    }

    /// The axis split, asked for by the user (07-15): **shift+wheel zooms pitch, time
    /// does not move, and the pane does not slide sideways.**
    ///
    /// This one is *not* inherited — it goes against egui twice, which is the whole
    /// reason it needs pinning:
    ///
    /// 1. `zoom_delta_2d` splits on the scroll modifiers (shift = x = time here, alt =
    ///    y = pitch), so taking egui's answer would give this gesture the time axis and
    ///    leave pitch behind alt — the one modifier a Linux WM tends to swallow before
    ///    the app sees it. Nothing else here would notice `interact` drifting back to
    ///    `zoom_delta_2d`: it would still zoom, just the wrong axis.
    /// 2. Shift alone is not a zoom to egui at all — it is *horizontal scroll*, so the
    ///    notch lands in `smooth_scroll_delta.x` and the pane is entitled to it. The
    ///    `offset_x` assertion is what says the roll took it.
    ///
    /// ⚠ The wheel goes **down** here (zoom out), and that is load-bearing rather than
    /// arbitrary: written the other way first, and the `offset_x` assertion passed with
    /// the taking ripped out. An up-notch asks the pane to slide *left*, and it is already
    /// at zero — so "the pane did not move" was a statement about the clamp, not about
    /// this code. Down is the direction that moves it, which is exactly what
    /// [`shift_wheel_off_the_roll_still_scrolls_the_pane_sideways`] demonstrates with the
    /// same notch a few pixels higher: same gesture, same direction, two owners.
    #[test]
    fn shift_wheel_zooms_pitch_and_leaves_time_and_the_pane_alone() {
        let mut harness = rig(take(10.0));
        harness.run_steps(3);
        let plot = harness.state().plot;
        let (span_before, pitch_before) = {
            let take = &harness.state().take;
            (take.span_s(), take.pitch_span())
        };

        wheel(&harness, on_screen(plot), -50.0, Modifiers::SHIFT);
        harness.run_steps(6);

        let take = &harness.state().take;
        assert!(
            take.pitch_span() > pitch_before + 0.01,
            "shift+wheel must zoom the pitch axis out: {pitch_before:.2} → {:.2} semitones",
            take.pitch_span()
        );
        assert!(
            (take.span_s() - span_before).abs() < 0.01,
            "shift+wheel is the *pitch* axis alone: time span {span_before:.3} → {:.3} s",
            take.span_s()
        );
        assert_eq!(
            harness.state().offset_x,
            0.0,
            "the pane must not slide sideways while the roll zooms pitch — one gesture, one effect"
        );
        assert!(
            take.framing == PitchFraming::Manual,
            "a hand-aimed pitch window is manual — otherwise the next frame reframes it away"
        );
    }

    /// The other side of that take, and the reason its `offset_x` assertion means
    /// anything: **a shift+wheel that misses the roll is still the pane's.**
    ///
    /// Without this, "the pane did not slide sideways" would pass just as happily if the
    /// pane could not slide sideways at all — the vacuous-green failure this file has
    /// already been bitten by three times. It also pins the *scope* of the one gesture
    /// the roll takes: over the roll, and nowhere else. In the app, this point is the
    /// controls and the corpus listing above it.
    #[test]
    fn shift_wheel_off_the_roll_still_scrolls_the_pane_sideways() {
        let mut harness = rig(take(10.0));
        harness.run_steps(3);
        let above_the_roll = egui::pos2(harness.state().plot.center().x, 20.0);

        wheel(&harness, above_the_roll, -50.0, Modifiers::SHIFT);
        harness.run_steps(6);

        assert!(
            harness.state().offset_x > 0.0,
            "shift+wheel off the roll is the pane's sideways scroll ({} px) — the roll took a \
             gesture that was never aimed at it",
            harness.state().offset_x
        );
    }

    /// REGRESSION: the roll has a **vertical axis at all**.
    ///
    /// `view_lo`/`view_hi` used to be written only by `reframe`, and `pan` read only
    /// `drag_delta().x` — so the pitch axis could be neither scrolled nor zoomed, and
    /// the user's «скроллить и зумить» was answered on one axis out of two. Driven
    /// through a real drag rather than by calling `pan_pitch`, because the missing half
    /// *was* the wiring: the arithmetic for the pitch axis did not exist to be tested.
    #[test]
    fn dragging_moves_the_pitch_axis_and_not_only_time() {
        let mut harness = rig(take(10.0));
        harness.run_steps(3);
        let plot = harness.state().plot;
        let (lo_before, hi_before) = {
            let take = &harness.state().take;
            (take.view_lo, take.view_hi)
        };

        // Drag straight down. The gesture is "grab the take and move it", the same as on
        // the time axis — so the take follows the hand downward and the window walks
        // *up*, revealing the higher rows that were above the top edge. (Written the
        // other way round first, and this test is what said so.)
        let from = on_screen(plot);
        let to = from + egui::Vec2::new(0.0, 60.0);
        harness.drag_at(from);
        harness.run_steps(2);
        harness.hover_at(to);
        harness.run_steps(2);
        harness.drop_at(to);
        harness.run_steps(2);

        let take = &harness.state().take;
        assert!(
            take.view_lo > lo_before + 0.5 && take.view_hi > hi_before + 0.5,
            "dragging the take down must walk the pitch window up: {lo_before:.1}..\
             {hi_before:.1} → {:.1}..{:.1}",
            take.view_lo,
            take.view_hi
        );
        assert!(
            (take.pitch_span() - (hi_before - lo_before)).abs() < 0.01,
            "panning must slide the window, not resize it"
        );
        assert!(
            take.framing == PitchFraming::Manual,
            "a hand-aimed window is manual — otherwise the next frame reframes it away"
        );
    }

    /// REGRESSION, and the trap in this whole feature: a hand-aimed pitch window must
    /// survive the take streaming in, and a double-click must give the auto framing back.
    ///
    /// The auto framing is min/max over the take and is **the only reason the open-G
    /// sub-octave was ever seen** (`memory/open_g_sub_octave_bursts.md`) — so it must not
    /// be lost for good the moment the user nudges the view; and equally it must not
    /// silently overrule a window the user aimed while the take was still playing, which
    /// is exactly when `reframe` is running on every batch of frames.
    #[test]
    fn a_hand_aimed_pitch_window_survives_reframing_and_a_double_click_undoes_it() {
        let mut loaded = take(10.0);
        play(&mut loaded, 100, Some(69.0)); // A4 — auto-framed around it

        loaded.zoom_pitch_about(0.25, 69.0, BANK);
        let (lo, hi) = (loaded.view_lo, loaded.view_hi);
        assert!(hi - lo < 10.0, "the zoom took: {lo:.1}..{hi:.1}");

        // The take plays on, over a wider range than the user framed: `reframe` runs on
        // every batch and must keep its hands off.
        play(&mut loaded, 100, Some(81.0));
        assert_eq!(
            (loaded.view_lo, loaded.view_hi),
            (lo, hi),
            "reframing stole the window the user aimed"
        );

        // Double-click = fit = the take's own framing, on both axes.
        loaded.fit();
        assert!(
            loaded.framing == PitchFraming::Auto && loaded.view_lo < 69.0 && loaded.view_hi > 81.0,
            "fit must frame the whole played range again: {:.1}..{:.1}",
            loaded.view_lo,
            loaded.view_hi
        );
    }

    /// The hand-aimed window stops where evidence stops: outside the bank's range the
    /// detector has no filters at all, so it can publish nothing, and rows of
    /// guaranteed-empty grid would read as "nothing sounded here" when the truth is
    /// "nothing could have been heard here". Same argument as [`the_window_stays_inside_
    /// the_take`], one axis over.
    #[test]
    fn the_pitch_window_stays_inside_the_bank() {
        let mut loaded = take(10.0);
        loaded.pan_pitch(-100.0, BANK);
        assert!(
            loaded.view_lo >= BANK.lo - 1e-3,
            "panned to {:.1}, below the bank's {:.1}",
            loaded.view_lo,
            BANK.lo
        );
        loaded.pan_pitch(500.0, BANK);
        assert!(
            loaded.view_hi <= BANK.hi + 1e-3,
            "panned to {:.1}, above the bank's {:.1}",
            loaded.view_hi,
            BANK.hi
        );

        // Zoom in past all reason: the span stops, it does not collapse or invert.
        for _ in 0..200 {
            loaded.zoom_pitch_about(0.5, 70.0, BANK);
        }
        assert!(
            (loaded.pitch_span() - MIN_PITCH_SPAN).abs() < 1e-3,
            "pitch span {:.3}; it must stop at MIN_PITCH_SPAN",
            loaded.pitch_span()
        );
        // And out past all reason: the bank, never more.
        for _ in 0..200 {
            loaded.zoom_pitch_about(2.0, 70.0, BANK);
        }
        assert!(
            (loaded.pitch_span() - BANK.span()).abs() < 1e-3,
            "zoomed out is exactly the bank: {:.1}..{:.1}",
            loaded.view_lo,
            loaded.view_hi
        );
    }

    /// A click at a take-second, aimed through the real plot.
    fn click_at_t(harness: &Harness<'_, Rig>, t: f64) {
        let (plot, window) = {
            let rig = harness.state();
            (rig.plot, rig.take.window.clone())
        };
        let frac = (t as f32 - window.start) / (window.end - window.start);
        let at = egui::pos2(plot.left() + frac * plot.width(), on_screen(plot).y);
        harness.hover_at(at);
        harness.drag_at(at);
        harness.drop_at(at);
    }

    /// 🔑 **THE POINT OF THE WHOLE KICKSTART**, driven end to end: two clicks make a
    /// mark, and the mark's pitch is the *declaration's* — never the detector's.
    ///
    /// The take here is fed a line at A4 (69) throughout while the user declares F5
    /// (77). If the mark ever came off the picture instead of off the declaration it
    /// would say 69, and the corpus would be a mirror of what the detector already
    /// thought — the failure this whole feature exists to refuse.
    #[test]
    fn two_clicks_make_a_mark_whose_pitch_is_the_declaration_not_the_line() {
        let mut loaded = take(10.0);
        play(&mut loaded, 400, Some(69.0)); // the detector's line says A4, all take long

        let mut harness = rig(loaded);
        harness.state_mut().marking = Some(Marking {
            declared: Declaration::parse("F5").unwrap(),
            against:  MarkedAgainst::Evidence,
        });
        harness.run_steps(3);

        click_at_t(&harness, 2.0);
        harness.run_steps(2);
        assert!(
            harness.state().take.draft.is_some(),
            "the first click must open the gesture, not make a mark"
        );
        assert!(harness.state().take.marks.is_empty(), "one click is not a note");

        click_at_t(&harness, 4.0);
        harness.run_steps(2);

        let marks = &harness.state().take.marks;
        assert_eq!(marks.len(), 1, "the second click closes the mark");
        assert_eq!(
            marks[0].midi, 77,
            "the mark must carry the DECLARED F5, not the A4 the detector drew under it"
        );
        assert!(
            (marks[0].interval.start - 2.0).abs() < 0.2 && (marks[0].interval.end - 4.0).abs() < 0.2,
            "the mark's time comes from the clicks: {:?}",
            marks[0].interval
        );
        assert!(
            harness.state().take.draft.is_none(),
            "the gesture is finished, not still open"
        );
    }

    /// Marking is armed by the declaration and by **nothing else**: with no note
    /// declared, a click on the roll is just a click.
    ///
    /// This is the construction the whole design rests on — there is no path from a
    /// click to a mark that does not pass through having said what you played first. A
    /// "mark now, name it later" affordance would put the naming *after* the looking,
    /// which is exactly the mirror.
    #[test]
    fn without_a_declaration_a_click_marks_nothing() {
        let mut harness = rig(take(10.0));
        harness.state_mut().marking = None; // nothing declared
        harness.run_steps(3);

        click_at_t(&harness, 2.0);
        harness.run_steps(2);
        click_at_t(&harness, 4.0);
        harness.run_steps(2);

        assert!(harness.state().take.marks.is_empty(), "a click is not a mark");
        assert!(harness.state().take.draft.is_none(), "and no gesture was opened");
    }

    /// The gesture survives scrolling and zooming between its two clicks — which is why
    /// it is click-click and not a drag (the pattern is `main_app`'s, `ui/notes/
    /// annotate.rs`). A note's two ends are routinely not both on screen at the
    /// magnification you need to place either of them.
    #[test]
    fn the_two_clicks_may_be_zoomed_between() {
        let mut loaded = take(10.0);
        play(&mut loaded, 400, Some(69.0));
        let mut harness = rig(loaded);
        harness.state_mut().marking = Some(Marking {
            declared: Declaration::parse("G3").unwrap(),
            against:  MarkedAgainst::Evidence,
        });
        harness.run_steps(3);

        click_at_t(&harness, 2.0);
        harness.run_steps(2);

        // Zoom in hard between the clicks: the draft is in take seconds, so it must not
        // move with the window.
        let plot = harness.state().plot;
        wheel(&harness, on_screen(plot), 60.0, Modifiers::COMMAND);
        harness.run_steps(6);
        assert!(
            harness.state().take.span_s() < 9.0,
            "the zoom did not take, so this test is not testing anything"
        );
        assert_eq!(
            harness.state().take.draft,
            Some(2.0),
            "the half-made mark must stay at the take-second it was clicked at"
        );

        click_at_t(&harness, 3.0);
        harness.run_steps(2);
        let marks = &harness.state().take.marks;
        assert_eq!(marks.len(), 1, "the mark closes across the zoom");
        assert_eq!(marks[0].midi, 55, "G3, as declared");
    }

    /// The two gestures cannot collide, and this is what says so: a **double-click
    /// fits** even while marking is armed, because two clicks in the same place cannot
    /// be a note's two ends.
    ///
    /// Without the rule, marking would silently eat "double-click to fit": the first
    /// click always opens a draft, so a fit could never fire while a declaration was
    /// armed — and the caption under the roll promises it. That is not a collision to
    /// arbitrate, it is a gesture that means one thing.
    #[test]
    fn a_double_click_fits_even_while_marking_is_armed() {
        let mut loaded = take(10.0);
        play(&mut loaded, 400, Some(69.0));
        loaded.window = 2.0..6.0; // a window `fit` will visibly undo

        let mut harness = rig(loaded);
        harness.state_mut().marking = Some(Marking {
            declared: Declaration::parse("F5").unwrap(),
            against:  MarkedAgainst::Evidence,
        });
        harness.run_steps(3);

        // Two clicks in the same place, back to back — a double-click.
        let at = on_screen(harness.state().plot);
        harness.hover_at(at);
        for _ in 0..2 {
            harness.drag_at(at);
            harness.drop_at(at);
        }
        harness.run_steps(4);

        assert_eq!(
            harness.state().take.window,
            0.0..10.0,
            "a double-click must fit the take, armed or not"
        );
        assert!(
            harness.state().take.marks.is_empty(),
            "two clicks in one place are not a note — they have no duration"
        );
        assert!(
            harness.state().take.draft.is_none(),
            "and the gesture must not be left half-open"
        );
    }

    /// REGRESSION, found by the render test below and worse than what it was looking
    /// for: **the framing must contain the marks**, not just the detector's line.
    ///
    /// The auto framing is min/max over the pitches the *engine* published. Declare F5,
    /// have the detector insist on F4, and the window is framed on F4 — so the user's
    /// own marks sit above its top edge and are simply not drawn. That is not a corner
    /// case, it is `g_flageolet_f5`: the take this whole kickstart was started for, where
    /// the detector is wrong by an octave and the marks exist to say so. The marks would
    /// have been invisible in exactly the case they were made for.
    #[test]
    fn the_framing_holds_the_marks_and_not_only_the_detectors_line() {
        let mut loaded = take(10.0);
        play(&mut loaded, 400, Some(65.0)); // the detector hears F4
        assert!(
            loaded.view_hi < 77.0,
            "precondition: framed on the line alone, F5 is outside ({:.1})",
            loaded.view_hi
        );

        loaded.mark_click(1.0, marking_f5());
        loaded.mark_click(3.0, marking_f5()); // the player says it was F5

        assert!(
            loaded.view_hi > 77.0,
            "the mark at F5 (77) is above the window's top ({:.1}) — the player's own \
             answer is off screen precisely where it disagrees with the detector",
            loaded.view_hi
        );
        assert!(
            loaded.view_lo < 65.0,
            "and the line it disagrees with must stay in view too: {:.1}",
            loaded.view_lo
        );

        // Symmetric: dropping the mark lets the window close again.
        loaded.undo_mark();
        assert!(
            loaded.view_hi < 77.0,
            "undo must let the framing close back onto the take: {:.1}",
            loaded.view_hi
        );
    }

    /// An F5 declaration, marked against the evidence.
    fn marking_f5() -> Marking {
        Marking {
            declared: Declaration::parse("F5").unwrap(),
            against:  MarkedAgainst::Evidence,
        }
    }

    /// 🔑 **The design, in pixels**: the mark is painted on the row it was *declared* at,
    /// nowhere near the row the detector drew.
    ///
    /// Every other test here checks the mark's `midi` field, which proves the number is
    /// carried correctly and proves nothing about the picture the user reads. The claim
    /// this panel makes to its user is visual — *this bar is your F5, that line is the
    /// detector's opinion, look how far apart they are* — and a placement bug would keep
    /// every field correct while drawing the answer somewhere it means nothing. Rendered
    /// through the real egui/epaint/wgpu stack, like `tests/pill_layout.rs`.
    ///
    /// The line is off here, which is both the marking default and a necessity for the
    /// test: violet was chosen for marks precisely because nothing else in the roll is
    /// violet, and this is where that pays — the assertion can say "violet ⇒ the player",
    /// which it could not have said about the amber this started as (the intonation
    /// ramp's "slightly flat" is a few units away from it).
    #[test]
    fn a_mark_is_painted_on_its_declared_row_not_on_the_detectors() {
        const RECT_W: f32 = 600.0;
        const RECT_H: f32 = 320.0;
        // The detector's line, all take long. The mark will declare F5 = 77 — an
        // interval away, and the whole point.
        const LINE_MIDI: f32 = 69.0;
        const DECLARED: u8 = 77;

        let mut loaded = take(10.0);
        play(&mut loaded, 400, Some(LINE_MIDI));
        loaded.marks.push(NoteMark::new(
            2.0,
            4.0,
            Declaration::parse("F5").unwrap(),
            MarkedAgainst::Evidence,
        ));
        // The framing counts the marks (see `reframe`) — without this the window is
        // 62..76, framed on the detector's A4, and F5 at 77 is off the top edge. That is
        // the bug this test found.
        loaded.reframe();
        let playhead_t = loaded.playhead_t().unwrap();
        let time = loaded.time_axis(playhead_t);
        let (view_lo, view_hi) = (loaded.view_lo, loaded.view_hi);

        // The rect is *recorded*, not assumed: kittest wraps the ui in a central panel
        // with an 8 px outer margin, so the plot does not start at the screen's origin.
        // Assuming it did put every assertion 8 px out — which this test still passed,
        // by 0.6 px of slack. Measure the layout, do not derive it.
        let mut harness = Harness::builder()
            .with_pixels_per_point(1.0)
            .with_size(egui::Vec2::new(RECT_W + 40.0, RECT_H + 40.0))
            .build_ui_state(
                move |ui, drawn: &mut Rect| {
                    let (rect, _) = ui.allocate_exact_size(vec2(RECT_W, RECT_H), Sense::hover());
                    *drawn = rect;
                    let painter = ui.painter_at(rect);
                    let map = pianoroll::RollMapping::new(rect, view_lo, view_hi, time);
                    pianoroll::draw_pitch_roll(
                        &painter,
                        rect,
                        &loaded.columns(playhead_t, RollLayer::Spectrum),
                        43,
                        96,
                        view_lo,
                        view_hi,
                        time,
                        AccidentalStyle::Sharps,
                        false, // marking: the mirror is off
                    );
                    loaded.draw_marks(&painter, map, playhead_t);
                },
                Rect::ZERO,
            );
        harness.run_steps(3);
        let img = harness.render().expect("wgpu render failed");
        let rect = *harness.state();

        // Violet ink: strongly red+blue, clearly less green. Nothing else in the roll
        // can produce it — the heat is blue-dominant with little red, the grid is grey.
        let violet_rows: Vec<u32> = (0..img.height())
            .filter(|&y| {
                (0..img.width()).any(|x| {
                    let p = img.get_pixel(x, y).0;
                    p[0] > 110
                        && p[2] > 150
                        && (p[0] as i32 - p[1] as i32) > 45
                        && (p[2] as i32 - p[1] as i32) > 45
                })
            })
            .collect();
        assert!(
            !violet_rows.is_empty(),
            "the mark was not painted at all — nothing violet in the render"
        );

        // Where the mark landed, back in MIDI — against the rect the layout actually used.
        let map = pianoroll::RollMapping::new(rect, view_lo, view_hi, time);
        let (top, bottom) = (
            *violet_rows.iter().min().unwrap() as f32,
            *violet_rows.iter().max().unwrap() as f32,
        );
        let center_y = 0.5 * (top + bottom);
        let declared_y = map.y_of(DECLARED as f32);
        let line_y = map.y_of(LINE_MIDI);

        println!("\n=== where the mark landed ===");
        println!("  violet rows : {top}..{bottom} (centre {center_y:.1})");
        println!("  F5 (declared, {DECLARED}) row : y = {declared_y:.1}");
        println!("  A4 (the line, {LINE_MIDI}) row : y = {line_y:.1}");

        // Two pixels, not "about a bar's height": the mark is centred on its row, so the
        // ink is symmetric about it and this can afford to be exact. A loose bound here
        // passed while the whole picture sat 8 px off, and said nothing.
        assert!(
            (center_y - declared_y).abs() < 2.0,
            "the mark is painted at y={center_y:.1}; F5 — what the player DECLARED — is \
             the row at y={declared_y:.1}"
        );
        assert!(
            (center_y - line_y).abs() > MARK_BAR_H * 2.0,
            "the mark landed on the detector's own row (y={line_y:.1}) — the declaration \
             is being ignored, which is the one failure this whole feature exists to \
             prevent"
        );
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
