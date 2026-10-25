//! Pitch-roll panel — a horizontal "waterfall table" of what you play/sing.
//!
//! Rows are notes (one per semitone), time flows right → left, and the live
//! detected pitch is traced as a continuous graph across the grid. Unlike the
//! staff panel it does not quantise into written noteheads — it just mirrors the
//! raw pitch line, so glide, vibrato and intonation drift are visible directly.
//!
//! State (the drained bank frames + the eased view window) lives in [`PitchRoll`],
//! held on `App`. The stateless grid/graph drawing lives in
//! [`crate::ui::pianoroll`]; this module owns only the history + auto-framing.
//!
//! **The history is the engine's, drained by cursor — not sampled per UI frame.**
//! The panel used to push one `melody_pitch` reading per repaint into a 600-slot
//! ring, which made the x axis a count of *renderer ticks*: its own comment admitted
//! "~10 s at 60 fps, ~20 s at 30 fps". Worse than the wrong ruler, it was the wrong
//! data — the bank publishes at ~62 Hz, so a 60 fps panel dropped a few percent of
//! its frames and a 30 fps one dropped half, decimating the very trills and vibrato
//! the fast path exists to show. Now every frame the engine published is drawn, at
//! the audio time it was played (`MelodyFrame::t`).

use eframe::egui::{
    self,
    Align,
    Frame,
    Layout,
    RichText,
    Sense,
    Stroke,
    Ui,
    vec2,
};

use super::App;
#[cfg(any(target_os = "android", test))]
use super::mobile_split::Part;
use super::roll_motion::{
    FlingHand,
    Framing,
    PitchWindow,
    Semitones,
    SemitonesPerSec,
};
use crate::audio::types::{
    BankRange,
    MelodyFrame,
    MelodyHistory,
};
use crate::core_types::pitch::Midi;
use crate::ui::pianoroll::{
    self,
    Gutters,
    HeatGrid,
    PitchPoint,
    RollColumn,
    RollMode,
    TimeAxis,
};
use crate::ui::theme;
use crate::ui::tokens::{
    color,
    space,
};

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
/// How much of the past the roll keeps **and shows**, in seconds of played audio.
///
/// One number, two jobs, and they must not become two numbers: it is the history's
/// retention *and* the width of the plot's time axis. Two copies would be two opinions
/// about the span, and the moment they drifted the ruler would quietly start lying —
/// which is the failure this whole path exists to undo, not one to re-introduce.
///
/// It replaces a count of 600 UI frames, which was a span of seconds only if you knew
/// the frame rate and it never changed ("~10 s at 60 fps, ~20 s at 30 fps"). Enforced
/// against [`MelodyFrame::t`], so 30 fps, 60 fps and a stuttering frame all hold the
/// same ten seconds.
///
/// It lives here rather than in the renderer because it is *this panel's* window: the
/// live roll watches a fixed span ending at the playhead. The take roll reads the same
/// renderer with a window it scrolls and zooms, and the day a constant one module down
/// decided the span for both was the day the second one could not exist.
pub(super) const VISIBLE_SECONDS: f64 = 10.0;
/// Padding (semitones) kept above/below the played range so the line never rides
/// the very edge of the view.
///
/// Shared with the take roll: the two panels disagree about *what* range to frame — and
/// sharply, on purpose (`take_roll`) — but not about a line needing room to breathe once
/// the range is chosen.
pub(super) const VIEW_PAD: f32 = 2.5;
/// Minimum visible pitch span (semitones) so a single sustained note still shows a
/// sensible band of rows around it instead of one giant row. Shared with the take roll,
/// like [`VIEW_PAD`] and for the same reason.
pub(super) const MIN_SPAN: f32 = 14.0;

/// Frames of *recent* pitch the framing looks at — a fraction of the visible span on
/// purpose (~2.9 s at the bank's default 16 ms cadence, against the view's 10 s).
///
/// Bank frames, so unlike the old buffer this is stable against the frame rate; it is
/// still a *count* rather than a duration, which is tolerable only because it decides
/// nothing musical — it is how much recent pitch the view is framed on, and the view
/// is allowed to be a little tighter or looser when the user retunes the cadence.
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
/// [`PitchRoll::reframe`] always keeps the note sounding right now inside the view.
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

/// The live roll's time axis, and it never varies: a fixed [`VISIBLE_SECONDS`] window
/// whose right edge is the playhead and whose ruler counts backwards from it, so the
/// newest audio is always in the same place and the eye can stop hunting for it.
///
/// A constant, where the take roll's is state, because that difference *is* the
/// difference between the two panels: one watches, the other reads.
const LIVE_TIME_AXIS: TimeAxis = TimeAxis {
    near_s:     0.0,
    far_s:      VISIBLE_SECONDS as f32,
    zero_age_s: 0.0,
    mode:       RollMode::Live,
};

/// Which evidence layer is painted under the melody line.
///
/// **The two are not interchangeable views of one thing, and the default is not a taste.**
/// They differ in what they can prove:
///
/// - [`Self::Spectrum`] is an *independent* witness — the physical column the bank heard.
///   It is free to disagree with the line, and when it does, that disagreement is the
///   evidence. It is the only layer that can convict the detector, so it is the default.
/// - [`Self::Salience`] is the curve the line was decoded *from*. It agrees with the line
///   by construction — a mirror, not a witness. It answers "why did the Viterbi choose
///   this", which the spectrum cannot, and it cannot answer "was that right".
///
/// Lives here, in UI state, and pointedly **not** in `ResonatorSettings`: that struct is
/// `PartialEq`-compared to decide whether the bank is rebuilt (`dsp::resonator`), so a
/// display toggle parked in it would drop and rebuild the detector's filterbank every time
/// you looked at the other layer. Four leaks of a display setting into the detector are on
/// the record already (`memory/display_settings_must_not_steer_the_detector.md`); this
/// would have been the fifth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RollLayer {
    /// The bank's spectrum — the ground truth the line is judged against.
    #[default]
    Spectrum,
    /// SWIPE′'s salience curve — the evidence the line was decoded from. A debug layer.
    Salience,
}

/// Live pitch-roll state for the panel.
pub struct PitchRoll {
    /// The bank frames on screen, oldest → newest, trimmed to [`VISIBLE_SECONDS`] of
    /// **played** time. Every frame the engine published while this panel was open —
    /// none dropped, none doubled, whatever the frame rate did.
    ///
    /// The line and both column layers live in the same record, so they cannot fall
    /// out of step: whichever layer is painted is what the line is judged against by
    /// eye, and it can only do that job if it is the *same instant*.
    ///
    /// The engine's own [`MelodyHistory`], just held longer — the ring's rules
    /// (ordered, unique, self-healing when the sample clock restarts) are the same
    /// rules here, and a second hand-rolled `VecDeque` is exactly how one of them
    /// would come to be implemented only in the other place.
    frames: MelodyHistory,
    /// How far the panel has read the engine's history — the `seq` of the last frame
    /// taken. The cursor is ours, not the engine's, so the staff can hold its own and
    /// read the same frames (see `AudioEngine::melody_since`).
    cursor: Option<u64>,
    /// The pitch window on screen — a moving body with inertia and an acceleration cap
    /// (`roll_motion`). The auto framing only *aims* it ([`Self::reframe`]); a finger
    /// drags, flings and pinches it; [`Self::tick`] moves it.
    view:   PitchWindow,
    /// Which layer is painted under the line — see [`RollLayer`].
    layer:  RollLayer,
}

impl Default for PitchRoll {
    fn default() -> Self {
        // Default window ≈ violin open strings (G3=55 .. E5=76) with headroom, so
        // the panel looks sensible before the first note eases it to real data.
        Self {
            frames: MelodyHistory::with_retention(VISIBLE_SECONDS),
            cursor: None,
            view:   PitchWindow::new(Midi(53.0), Midi(79.0)),
            layer:  RollLayer::default(),
        }
    }
}

impl PitchRoll {
    /// Take everything the engine has published since the last read.
    ///
    /// `fresh` is oldest → newest and may be empty (the UI outruns the bank about
    /// as often as not), one frame, or several (a stutter, or wasm's snapshots
    /// arriving per ~85 ms sample block). All three are ordinary: the panel's history
    /// is paced by the audio, and the repaint that drains it only decides *when* the
    /// frames are collected, never how many there are.
    pub fn update(&mut self, fresh: Vec<MelodyFrame>) {
        let Some(last_seq) = fresh.last().map(|f| f.seq) else {
            return; // nothing new — hold the view rather than let it drift
        };
        self.cursor = Some(last_seq);
        // The ring does the trimming and the stream-restart healing (both are about
        // `t`, and both are its rules, not the panel's).
        for frame in fresh {
            self.frames.push(frame);
        }
        self.reframe();
    }

    /// The newest frame's timestamp — the playhead, in audio time.
    fn now(&self) -> Option<f64> {
        self.frames.newest().map(|f| f.t)
    }

    /// Advance the window's motion by `dt` seconds of wall time — once per UI frame,
    /// whether or not the bank published, so a glide in flight keeps gliding.
    fn tick(&mut self, dt: f32, bank: BankRange) {
        self.view.tick(dt, bank);
    }

    /// Aim the view window at what is being played *now*. Aim only: the window gets
    /// there under `roll_motion`'s spring and acceleration cap, never in one frame.
    ///
    /// Three rules, each earning its keep (see the constants for why):
    /// 1. only the last [`FRAMING_FRAMES`] are considered — the scrolled-off past
    ///    gets no vote;
    /// 2. the bounds are *quantiles*, so a lone slip or grace note cannot pin the
    ///    view wide, with the currently sounding note always aimed into view;
    /// 3. growing is wanted at once, shrinking needs [`VIEW_SHRINK_SLACK`] of slack —
    ///    the hysteresis that stops the rows creeping.
    ///
    /// Holds the last aim while fully silent, so the rows don't drift when nothing
    /// is playing. Does nothing while a hand has the window (`Framing::Manual`).
    fn reframe(&mut self) {
        if self.view.framing() == Framing::Manual {
            return;
        }
        // Rule 1: recent pitch only.
        let recent = self.frames.len().saturating_sub(FRAMING_FRAMES);
        let mut pitches: Vec<f32> = self
            .frames
            .iter()
            .skip(recent)
            .filter_map(|frame| frame.pitch.map(|Midi(midi)| midi))
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
        if let Some(now) = self.sounding() {
            data_lo = data_lo.min(now);
            data_hi = data_hi.max(now);
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
        let (view_lo, view_hi) = (self.view.lo().0, self.view.hi().0);
        if target_lo > view_lo && target_lo - view_lo < VIEW_SHRINK_SLACK {
            target_lo = view_lo;
        }
        if target_hi < view_hi && view_hi - target_hi < VIEW_SHRINK_SLACK {
            target_hi = view_hi;
        }

        // ⚠ No snap open here any more, deliberately. It used to widen the view to the
        // sounding note in the same frame, so one octave slip of the detector yanked
        // the whole grid open by an octave and the glide then took it back — the roll
        // breathed with every slip ("колбасит", 10-25). Opening is a fast, capped glide
        // now (`roll_motion::GROW`): a leap is off screen for a fraction of a second
        // instead of the grid jumping under the eye.
        self.view.aim(Midi(target_lo), Midi(target_hi));
    }

    /// The most recent pitch the engine actually gave us — the note sounding now,
    /// looking back past any silent/rejected frames at the tail. Raw MIDI: its only
    /// callers are the view window's arithmetic, which is plain `f32`.
    fn sounding(&self) -> Option<f32> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.pitch)
            .map(|Midi(midi)| midi)
    }

    /// The frames as the renderer wants them: aged against the playhead, with the
    /// heat gated for display.
    fn columns(&self, now: f64) -> Vec<RollColumn<'_>> {
        columns_of(self.frames.iter(), now, self.layer)
    }

    /// One UI frame of the window: the hand's gestures on `plot`, then the motion.
    ///
    /// Wall time (`stable_dt`), not audio time: the window is a body on screen, and it
    /// must keep moving between bank publications (wasm delivers them ~85 ms apart).
    fn interact(&mut self, ui: &Ui, resp: &egui::Response, plot: egui::Rect, bank: BankRange) {
        self.handle_gestures(ui, resp, plot);
        let dt = ui.input(|input| input.stable_dt);
        self.tick(dt, bank);
    }

    /// The roll under the hand. Pitch only — time stays live: the live roll watches,
    /// browsing the past is the take roll's job.
    ///
    /// - **drag** pans pitch, the rows following the finger, and a release flings it
    ///   on (`roll_motion` coasts it to rest, further for a finger than a mouse);
    /// - **pinch** — or ctrl+wheel on a desktop — zooms about the fingers;
    /// - **double-tap** hands the window back to the auto framing.
    ///
    /// Either of the first two takes the window from the auto framing
    /// (`Framing::Manual`): a framing that kept aiming would yank it from under the
    /// finger the moment it lifted.
    ///
    /// The scalar `zoom_delta`, not the take roll's per-axis split: there is one axis
    /// here, so a pinch at any angle means it, and making a sideways pinch do nothing
    /// would only be a trap.
    fn handle_gestures(&mut self, ui: &Ui, resp: &egui::Response, plot: egui::Rect) {
        if resp.double_clicked() {
            self.view.resume_auto();
            self.reframe();
            return;
        }
        let px_per_semitone = plot.height() / self.view.span().0;
        if resp.dragged() {
            let (touch, velocity) = ui.input(|input| (input.any_touches(), input.pointer.velocity()));
            let hand = if touch {
                FlingHand::Touch
            } else {
                FlingHand::Pointer
            };
            // Screen y grows downward and pitch upward, so pulling the rows down looks
            // higher: the window moves by +dy.
            self.view.drag(
                Semitones(resp.drag_delta().y / px_per_semitone),
                SemitonesPerSec(velocity.y / px_per_semitone),
                hand,
            );
        }
        let (zoom, pinch_center) = ui.input(|input| {
            (
                input.zoom_delta(),
                input.multi_touch().map(|touch| touch.center_pos),
            )
        });
        // Only a zoom aimed at the roll: a pinch's anchor is between the fingers, a
        // wheel's under the cursor, and a wheel elsewhere in the app is not ours.
        if zoom != 1.0
            && let Some(at) = pinch_center.or(resp.hover_pos())
            && plot.contains(at)
        {
            let anchor = Midi(self.view.lo().0 + (plot.bottom() - at.y) / px_per_semitone);
            self.view.zoom_about(zoom, anchor);
        }
    }
}

/// Frames → columns the renderer can draw: aged against `playhead_t`, the chosen layer
/// picked out, the heat gated for display.
///
/// Free, and shared with the take roll, because both rolls draw the same frames through
/// the same renderer and only disagree about *which* frames and *when*. The gate in
/// particular must exist once: it is subtle enough (see [`HEAT_LEVEL_GATE`]) that a
/// second copy would be a second policy within a release or two, and the two rolls would
/// then paint the same take differently — with no way to tell which one was lying.
///
/// The gate is **display only**, and is not the engine's silence gate wearing a disguise.
/// It decides nothing: the line's `pitch` arrives already gated and already octave-judged.
pub(super) fn columns_of<'a>(
    frames: impl Iterator<Item = &'a MelodyFrame>,
    playhead_t: f64,
    layer: RollLayer,
) -> Vec<RollColumn<'a>> {
    frames
        .map(|frame| {
            // Both layers are normalized to their own column's max upstream, so both need
            // the same silence gate for the same reason — see [`HEAT_LEVEL_GATE`]. A quiet
            // salience column is room noise scored and stretched to full scale, which paints
            // the rests just as convincingly as a quiet spectrum does.
            //
            // The grid rides alongside the data: the spectrum layer *is* the bank, so it has
            // none of its own and the renderer maps it on the plot's bank span; salience
            // carries its detector's grid, which need not be the bank's (see `SalienceHeat`).
            let (heat, grid): (&[f32], Option<HeatGrid>) = if frame.level >= HEAT_LEVEL_GATE {
                match layer {
                    RollLayer::Spectrum => (frame.heat.as_slice(), None),
                    // `None` salience = a column with no energy to score at all, which is
                    // the same picture as a gated one: a gap. Nothing is lost by folding
                    // them together — neither has evidence in it.
                    RollLayer::Salience => {
                        match &frame.salience {
                            Some(s) => {
                                (
                                    s.data.as_slice(),
                                    Some(HeatGrid {
                                        min_midi:          s.min_midi,
                                        bins_per_semitone: s.bins_per_semitone,
                                    }),
                                )
                            }
                            None => (&[], None),
                        }
                    }
                }
            } else {
                (&[], None)
            };
            RollColumn {
                age_s: (playhead_t - frame.t) as f32,
                pitch: frame.pitch.map(|midi| {
                    PitchPoint {
                        midi,
                        level: frame.level,
                    }
                }),
                heat,
                grid,
            }
        })
        .collect()
}

/// The live roll's plot height off a touch screen. On the phone the roll is the screen of
/// a split panel and fills it instead (`draw_pitch_roll_part`).
const LIVE_PLOT_HEIGHT: f32 = 360.0;
/// The least a roll's plot is given on a touch screen, however short the screen. Under it
/// the rows of the minimum auto span ([`MIN_SPAN`]) drop below
/// [`crate::ui::pianoroll::LABEL_MIN_ROW_H`] and lose their names.
const TOUCH_PLOT_FLOOR: f32 = 160.0;

/// A roll's plot height: `desktop_h` under a mouse; on a touch screen no taller than fits
/// the visible pane with a finger's strip to spare.
///
/// For the take roll, which sits in the take recorder's scrolling page: it was a fixed
/// 320 px, and a landscape phone's pane is ~270 px tall under the panel picker, so it could
/// never be seen whole. Worse, it takes the drag (pan) itself, so once scrolled to, it
/// covered the whole pane and there was nothing left to put a finger on to scroll the page
/// back. Two touch rows ([`space::TOUCH_H`]) of the pane stay free for exactly that. (The
/// live roll does not scroll with a page: on the phone it fills its split screen.)
///
/// Measured against the pane's visible rect (`clip_rect` inside its `ScrollArea`), not the
/// screen, so whatever sits above the pane is already paid for. Never taller than
/// `desktop_h`.
pub(super) fn roll_height(ui: &Ui, desktop_h: f32) -> f32 {
    if !theme::is_touch_screen(ui.ctx()) {
        return desktop_h;
    }
    let fits = ui.clip_rect().height() - 2.0 * space::TOUCH_H;
    fits.min(desktop_h).max(TOUCH_PLOT_FLOOR)
}

/// One of a roll's layer switches — the look of `selectable_value`, finger-sized on a
/// touch screen ([`theme::min_target_h`]).
pub(super) fn roll_switch<T: PartialEq>(ui: &mut Ui, current: &mut T, value: T, label: &str) {
    let height = theme::min_target_h(ui.ctx());
    let switch = egui::Button::selectable(*current == value, label).min_size(vec2(0.0, height));
    if ui.add(switch).clicked() {
        *current = value;
    }
}

/// The card's title, and the hint under it naming what is *under* the line — the two
/// layers carry different authority and the picture cannot say so itself: one may
/// convict the line, the other cannot (see `RollLayer`).
fn title_and_hint(ui: &mut Ui, layer: RollLayer) {
    ui.label(RichText::new("Pitch Roll").size(20.0).color(color::TEXT_HEADING));
    ui.add(
        egui::Label::new(
            RichText::new(match layer {
                RollLayer::Spectrum => "Rows are notes; your pitch scrolls in from the right",
                RollLayer::Salience => "Under the line: what SWIPE′ scored — evidence, not ground truth",
            })
            .color(color::TEXT_HINT)
            .size(12.0),
        )
        .wrap(),
    );
}

/// The layer switches, in reading order.
fn layer_switches(ui: &mut Ui, layer: &mut RollLayer) {
    roll_switch(ui, layer, RollLayer::Spectrum, "spectrum");
    roll_switch(ui, layer, RollLayer::Salience, "SWIPE′");
}

impl App {
    pub(super) fn draw_pitch_roll_card(&mut self, ui: &mut Ui) {
        self.take_live_frames(ui);
        let (pad, radius) = theme::card_pad(ui.ctx(), 14, 22);
        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(radius)
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(pad)
            .show(ui, |ui| {
                let layer = &mut self.pitch_roll.layer;
                // Title + hint (~330 dp) beside the right-aligned switches (~150 dp).
                // Narrower, the switches were laid right-to-left *over* the hint, so
                // there they take their own row under it, in reading order.
                const HEADER_W: f32 = 500.0;
                if ui.available_width() >= HEADER_W {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| title_and_hint(ui, *layer));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            roll_switch(ui, layer, RollLayer::Salience, "SWIPE′");
                            roll_switch(ui, layer, RollLayer::Spectrum, "spectrum");
                        });
                    });
                } else {
                    title_and_hint(ui, *layer);
                    ui.horizontal(|ui| layer_switches(ui, layer));
                }

                ui.add_space(10.0);
                self.draw_live_roll(ui, LIVE_PLOT_HEIGHT);
            });
    }

    /// The phone's Pitch Roll, a split panel (`mobile_split`): the screen is the title
    /// and the roll — as tall as the screen part, which is the whole pane while the
    /// config is hidden — and the config is the layer switches and the detector picker.
    #[cfg(any(target_os = "android", test))]
    pub(super) fn draw_pitch_roll_part(&mut self, ui: &mut Ui, part: Part) {
        match part {
            Part::Screen => {
                self.take_live_frames(ui);
                let (pad, radius) = theme::card_pad(ui.ctx(), 14, 22);
                Frame::new()
                    .fill(color::PANEL_FILL)
                    .corner_radius(radius)
                    .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
                    .inner_margin(pad)
                    .show(ui, |ui| {
                        title_and_hint(ui, self.pitch_roll.layer);
                        ui.add_space(10.0);
                        // Inside the frame `available_height` already ends the card's
                        // margin above the screen's bottom (egui shrinks a frame's
                        // content rect by it), so the card ends at the screen's edge.
                        let height = ui.available_height().max(TOUCH_PLOT_FLOOR);
                        self.draw_live_roll(ui, height);
                    });
            }
            Part::Config => {
                ui.horizontal(|ui| layer_switches(ui, &mut self.pitch_roll.layer));
                // The roll's line is the detector's decision, so which detector draws it
                // is chosen here too — on the phone the Resonator FFT config, its other
                // home, is a whole panel away.
                ui.add_space(8.0);
                let mut settings = self.audio.analysis_settings();
                if super::controls::detector_picker(ui, &mut settings) {
                    self.audio.set_analysis_settings(settings);
                }
            }
        }
    }

    /// Keep the live panel running and take the engine's fresh frames into the roll —
    /// once per UI frame, before the roll is drawn.
    fn take_live_frames(&mut self, ui: &Ui) {
        // Live panel: keep repainting to track the audio thread, and keep the
        // resonator bank alive so the fused pitch stays low-latency (it parks when
        // no consumer asks — same reason the staff panel requests it).
        ui.ctx().request_repaint();
        self.audio.request_resonator();

        // SOURCE = the engine's melody history, taken by cursor: every bank frame
        // published since the last repaint, each carrying the line's pitch, the heat
        // column of that same instant, and the audio time it was played.
        //
        // NOT `reading()` sampled per frame. That is the *instant*, and asking for it
        // once per repaint silently made the renderer the sampler: at 60 fps against
        // the bank's ~62 Hz a few percent of frames were dropped, at 30 fps half of
        // them — decimating the trills and vibrato this panel exists to show, and
        // making its x axis a count of frames rather than a span of seconds.
        //
        // Both layers arrive finished: the line is silence-gated and octave-decided in
        // `audio::dsp::melody`, the heat is raw ground truth by design.
        let fresh = self.audio.melody_since(self.pitch_roll.cursor);
        self.pitch_roll.update(fresh);
    }

    /// The live roll itself, `height` tall across the whole width left in `ui`.
    fn draw_live_roll(&mut self, ui: &mut Ui, height: f32) {
        let settings = self.audio.analysis_settings();
        let style = settings.accidental;
        let bank = settings.resonator.bank_range();
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        // Named for TalkBack, and for the phone shell's tests to find the roll by.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Pitch roll"));
        let gutters = Gutters::for_screen(ui.ctx());
        self.pitch_roll
            .interact(ui, &resp, pianoroll::plot_rect(rect, gutters), bank);
        let painter = ui.painter_at(rect);
        // The playhead is the newest frame's own timestamp, not "now": there is no
        // reading of the clock here at all, so a late repaint draws the same picture,
        // just later. No frame yet — an empty roll.
        let columns = match self.pitch_roll.now() {
            Some(now) => self.pitch_roll.columns(now),
            None => Vec::new(),
        };
        pianoroll::draw_pitch_roll(
            &painter,
            rect,
            gutters,
            &columns,
            bank,
            self.pitch_roll.view.lo().0,
            self.pitch_roll.view.hi().0,
            LIVE_TIME_AXIS,
            style,
            // The live roll IS the line: it is the mirror you play into, and there is
            // nothing here to contaminate — see `app::take_marks`.
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::types::SalienceHeat;

    /// The bank's default publish cadence (`ResonatorSettings::update_ms` = 16 ms).
    const BANK_CADENCE_S: f64 = 0.016;

    /// A stand-in for the engine's publishing side: hands out bank frames on the
    /// audio clock, so a test can choose the *audio* cadence and the *delivery*
    /// batching independently — which is the whole point of the change.
    struct Bank {
        seq: u64,
        t:   f64,
    }

    impl Bank {
        fn new() -> Self {
            Self { seq: 0, t: 0.0 }
        }

        /// The next frame, one cadence later. The column layers are left empty: they ride
        /// the same record by construction now, so there is nothing here for a test to
        /// check that the type system does not already.
        fn frame(&mut self, pitch: Option<f32>, level: f32) -> MelodyFrame {
            self.t += BANK_CADENCE_S;
            self.seq += 1;
            MelodyFrame {
                seq: self.seq,
                t: self.t,
                pitch: pitch.map(Midi),
                frontend: Default::default(),
                raw_pitch: None,
                reference_pitch: None,
                level,
                heat: Vec::new(),
                salience: None,
            }
        }
    }

    /// A bank span wide enough that no test window touches it.
    fn test_bank() -> BankRange {
        use crate::core_types::pitch::PNote;
        BankRange::new(PNote::new(24).unwrap(), PNote::new(108).unwrap())
    }

    /// Feed the line one frame at a time (delivery = the bank's own cadence), with one
    /// UI frame of motion per bank frame.
    fn line(bank: &mut Bank, roll: &mut PitchRoll, pitch: Option<f32>, level: f32) {
        let frame = bank.frame(pitch, level);
        roll.update(vec![frame]);
        roll.tick(BANK_CADENCE_S as f32, test_bank());
    }

    /// The window's edges, as raw MIDI for the arithmetic of the assertions.
    fn edges(roll: &PitchRoll) -> (f32, f32) {
        (roll.view.lo().0, roll.view.hi().0)
    }

    /// The toggle swaps the layer, and both layers obey the same silence gate.
    ///
    /// Worth a test despite looking like a `match`: swapping the layers is invisible to the
    /// type system and a crossed wire would paint a plausible field of the wrong quantity —
    /// the failure would read as "SWIPE′ looks a lot like the spectrum", which is a
    /// conclusion, not a bug report. The gate has to be repeated per layer for the same
    /// reason it exists at all: both are normalized to their own column's max, so both turn
    /// room noise into a full-scale wash across the rests. (The salience layer now carries
    /// its own grid rather than borrowing the spectrum's — see `SalienceHeat`.)
    #[test]
    fn the_toggle_swaps_the_layer_and_both_obey_the_gate() {
        let sh = |data: Vec<f32>| {
            SalienceHeat {
                data,
                min_midi: Midi(12.0),
                bins_per_semitone: 8.0,
            }
        };
        let loud = |salience| {
            vec![MelodyFrame {
                seq: 1,
                t: 0.0,
                pitch: Some(Midi(69.0)),
                frontend: Default::default(),
                raw_pitch: None,
                reference_pitch: None,
                level: 0.5, // over HEAT_LEVEL_GATE
                heat: vec![1.0, 0.0],
                salience,
            }]
        };

        let mut roll = PitchRoll::default();
        roll.update(loud(Some(sh(vec![0.0, 1.0]))));
        assert_eq!(
            roll.columns(0.0)[0].heat,
            [1.0, 0.0],
            "default layer is the spectrum"
        );

        roll.layer = RollLayer::Salience;
        assert_eq!(
            roll.columns(0.0)[0].heat,
            [0.0, 1.0],
            "the toggle must paint the salience, not the spectrum wearing its name"
        );

        // No energy to score → the same picture as a gated column: a gap.
        let mut unscored = PitchRoll {
            layer: RollLayer::Salience,
            ..PitchRoll::default()
        };
        unscored.update(loud(None));
        assert!(
            unscored.columns(0.0)[0].heat.is_empty(),
            "an unscored column is a gap"
        );

        // Below the gate the layer is blanked whichever one is selected.
        let mut quiet = PitchRoll {
            layer: RollLayer::Salience,
            ..PitchRoll::default()
        };
        quiet.update(vec![MelodyFrame {
            seq:             1,
            t:               0.0,
            pitch:           None,
            frontend:        Default::default(),
            raw_pitch:       None,
            reference_pitch: None,
            level:           HEAT_LEVEL_GATE * 0.5,
            heat:            vec![1.0, 0.0],
            salience:        Some(sh(vec![0.0, 1.0])),
        }]);
        assert!(
            quiet.columns(0.0)[0].heat.is_empty(),
            "a silent salience column is room noise stretched to full scale — gate it too"
        );
    }

    /// REGRESSION, and the reason this whole path exists: **the history is paced by
    /// the audio, not by the renderer**.
    ///
    /// The panel used to sample `melody_pitch` once per repaint, which made the UI the
    /// sampler: against the bank's ~62 Hz, a 60 fps panel dropped a few percent of the
    /// frames and a 30 fps one dropped *half*. What it dropped was exactly what the
    /// fast path is for — this feeds a trill, the fastest thing a violinist does, and
    /// checks that a panel repainting half as often still holds every alternation.
    ///
    /// Both rolls are fed the identical audio; only the *batching* differs.
    #[test]
    fn history_is_paced_by_the_audio_not_the_frame_rate() {
        // A D5↔E5 trill at the bank's cadence: every frame is a note change.
        let mut bank = Bank::new();
        let played: Vec<MelodyFrame> = (0..240)
            .map(|i| bank.frame(Some(if i % 2 == 0 { 74.0 } else { 76.0 }), 0.5))
            .collect();

        // 60 fps: roughly one bank frame per repaint. 30 fps: two.
        let mut fast = PitchRoll::default();
        for frame in played.iter() {
            fast.update(vec![frame.clone()]);
        }
        let mut slow = PitchRoll::default();
        for pair in played.chunks(2) {
            slow.update(pair.to_vec());
        }

        let pitches =
            |roll: &PitchRoll| -> Vec<Option<Midi>> { roll.frames.iter().map(|f| f.pitch).collect() };
        assert_eq!(
            pitches(&slow),
            pitches(&fast),
            "a panel repainting half as often must still hold every frame the bank \
             published — dropping them is what decimated the trill"
        );
        assert_eq!(slow.frames.len(), played.len(), "no frame lost, none doubled");
        assert_eq!(
            slow.cursor,
            Some(played.last().unwrap().seq),
            "cursor tracks the last frame taken"
        );
    }

    /// REGRESSION: the visible span is **seconds of audio**, whatever the cadence.
    ///
    /// The old ring held 600 frames, which its own comment admitted was "~10 s at
    /// 60 fps, ~20 s at 30 fps" — a ruler that changed length depending on how busy
    /// the renderer was. `update_ms` is user-adjustable over a 10× range too, so even
    /// the *bank's* frames are not a fixed span. Only time is.
    #[test]
    fn the_visible_span_is_seconds_not_frames() {
        let mut bank = Bank::new();
        let mut roll = PitchRoll::default();
        // Play for well over the visible span, so the trim is doing the work.
        for _ in 0..((15.0 / BANK_CADENCE_S) as usize) {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        let held = roll.now().unwrap() - roll.frames.oldest().unwrap().t;
        assert!(
            held <= VISIBLE_SECONDS && held > VISIBLE_SECONDS - 4.0 * BANK_CADENCE_S,
            "history spans {held:.3} s of audio; it must be {VISIBLE_SECONDS} s"
        );
    }

    /// A new stream restarts the sample clock, so the old stream's frames must go —
    /// otherwise the roll would splice two unrelated moments together and draw the
    /// join as a glide. Enforced by the ring being strictly increasing in `t`, which
    /// needs no epoch counter to stay in sync with.
    #[test]
    fn a_restarted_clock_clears_the_history() {
        let mut bank = Bank::new();
        let mut roll = PitchRoll::default();
        for _ in 0..50 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        // The engine reset: same monotonic `seq` (a panel's cursor survives a device
        // switch), but the sample clock starts over.
        roll.update(vec![MelodyFrame {
            seq:             bank.seq + 1,
            t:               0.0,
            pitch:           Some(Midi(60.0)),
            frontend:        Default::default(),
            raw_pitch:       None,
            reference_pitch: None,
            level:           0.5,
            heat:            Vec::new(),
            salience:        None,
        }]);
        assert_eq!(
            roll.frames.len(),
            1,
            "the previous stream's frames must not survive it"
        );
    }

    /// Silence keeps the framing put (no drift while nothing plays).
    #[test]
    fn silence_holds_the_view() {
        let mut bank = Bank::new();
        let mut roll = PitchRoll::default();
        let before = edges(&roll);
        for _ in 0..30 {
            line(&mut bank, &mut roll, None, 0.0);
        }
        assert_eq!(edges(&roll), before);
    }

    /// A sustained note eases the window to frame it, keeping the min span.
    #[test]
    fn note_reframes_within_min_span() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(69.0), 0.5); // A4
        }
        let (lo, hi) = edges(&roll);
        assert!(lo < 69.0 && hi > 69.0, "note is inside the view");
        assert!(hi - lo >= MIN_SPAN - 0.5, "span stays at least the minimum");
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..200 {
            line(&mut bank, &mut roll, Some(48.0), 0.5); // C3, low phrase
        }
        for _ in 0..600 {
            line(&mut bank, &mut roll, Some(84.0), 0.5); // C6, settle up high
        }
        let (lo, hi) = edges(&roll);
        let span = hi - lo;
        assert!(
            span < 24.0,
            "view still spans {span:.1} semitones after moving on; the old phrase \
             should have stopped counting"
        );
        assert!(lo < 84.0 && hi > 84.0, "the note being played must be in view");
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        // A realistic phrase: a violin first position wandering over a fifth or so.
        for midi in [67.0f32, 69.0, 71.0, 72.0, 74.0, 71.0, 69.0]
            .iter()
            .cycle()
            .take(600)
        {
            line(&mut bank, &mut roll, Some(*midi), 0.5);
        }
        let span = roll.view.span().0;
        let row_h = PLOT_H / span;
        assert!(
            row_h >= LABEL_MIN_ROW_H,
            "span {span:.1} semitones gives {row_h:.1} px rows in a {PLOT_H:.0} px plot; \
             under {LABEL_MIN_ROW_H} only the C rows get labelled"
        );
    }

    /// A lone octave slip must not pin the view an octave wider — that is what the
    /// quantile bounds are for — and must not even *flash* it wider: the slip is the
    /// sounding note for one frame, and the old snap-open threw the grid open by an
    /// octave on exactly that frame. Now the window only starts gliding toward it.
    #[test]
    fn a_lone_slip_does_not_stretch_the_view() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(69.0), 0.5); // steady A4
        }
        let span_before = roll.view.span().0;
        line(&mut bank, &mut roll, Some(81.0), 0.5); // one slipped frame, an octave up
        let mut widest = roll.view.span().0;
        for _ in 0..200 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
            widest = widest.max(roll.view.span().0);
        }
        let span_after = roll.view.span().0;
        assert!(
            span_after < span_before + 2.0,
            "one slip stretched the view from {span_before:.1} to {span_after:.1}"
        );
        assert!(
            widest < span_before + 0.5,
            "one slip flashed the view open from {span_before:.1} to {widest:.1}"
        );
    }

    /// The currently sounding note is framed, even when it is the outlier the quantiles
    /// would exclude — and it gets there by a glide, not a cut.
    ///
    /// It used to be framed *in the same frame*: the view snapped open to the note, and
    /// that snap was the jolt of every octave slip ("колбасит", 10-25). Now opening is a
    /// capped glide, so the bound is a time: a leap of over an octave is in view within
    /// half a second, and no single frame jumps the top edge more than a step of it.
    #[test]
    fn current_note_glides_into_view() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(60.0), 0.5); // settled on C4
        }
        // Leap up and hold — far too few frames, at first, to move the quantiles.
        let frames = (0.5 / BANK_CADENCE_S) as usize;
        let mut worst_step: f32 = 0.0;
        for _ in 0..frames {
            let before = edges(&roll).1;
            line(&mut bank, &mut roll, Some(79.0), 0.5); // G5
            worst_step = worst_step.max((edges(&roll).1 - before).abs());
        }
        let hi = edges(&roll).1;
        assert!(
            hi >= 79.0,
            "after 0.5 s the note at 79 is still outside a view topping out at {hi:.1}"
        );
        assert!(
            worst_step < 1.0,
            "the top edge jumped {worst_step:.2} semitones in one frame — a cut, not a glide"
        );
    }

    /// Hysteresis: a settled note must not make the rows creep frame after frame.
    #[test]
    fn settled_note_does_not_creep() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..800 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        let (lo, hi) = edges(&roll);
        for _ in 0..60 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        let (lo_now, hi_now) = edges(&roll);
        assert!(
            (lo_now - lo).abs() < 0.5 && (hi_now - hi).abs() < 0.5,
            "view crept from ({lo:.2}, {hi:.2}) to ({lo_now:.2}, {hi_now:.2}) on a held note"
        );
    }

    /// The history stays bounded while playing forever — the trim actually runs.
    ///
    /// It used to be a count (`HISTORY_FRAMES`), which is why the span was a lie; the
    /// bound this asserts is the one that matters, and `the_visible_span_is_seconds_
    /// not_frames` above pins what it means in seconds.
    #[test]
    fn history_stays_bounded() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        let ceiling = (VISIBLE_SECONDS / BANK_CADENCE_S) as usize + 1;
        for _ in 0..(ceiling + 400) {
            line(&mut bank, &mut roll, Some(60.0), 0.4);
        }
        assert!(
            roll.frames.len() <= ceiling,
            "history grew to {} frames, past the {ceiling} that fit in {VISIBLE_SECONDS} s",
            roll.frames.len()
        );
    }

    /// A rejected frame reads as a gap, exactly like silence.
    ///
    /// The octave *judgement* itself is no longer this panel's business — it happens
    /// once, upstream, in `audio::dsp::melody`, and is tested there against the bank's
    /// cadence rather than the frame rate. All this panel owes is not to invent a
    /// point where the engine gave it none.
    #[test]
    fn a_rejected_frame_is_a_gap() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..5 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        line(&mut bank, &mut roll, None, 0.5); // engine rejected this frame (slip or silence)
        assert!(
            roll.frames.newest().unwrap().pitch.is_none(),
            "rejected frame is a gap"
        );
        line(&mut bank, &mut roll, Some(71.0), 0.5);
        assert!(
            roll.frames.newest().unwrap().pitch.is_some(),
            "next real frame is drawn"
        );
    }

    // ── The hand, through real touch events (`touch_fingers`) ──
    //
    // State tests cannot catch a gesture that never reaches the handler — a `Sense` that
    // does not take drags, a pinch read off the wrong input — so these drive the same
    // `interact` the panel calls, with touches as a phone sends them. The App around the
    // roll is left out: it needs an audio engine, and it adds nothing between the
    // response and `interact`.

    use eframe::egui::{
        Rect,
        pos2,
    };
    use egui_kittest::Harness;

    use crate::app::touch_fingers::{
        Finger,
        gesture,
    };

    type Rig = (PitchRoll, Rect);

    /// A roll settled on a held A4, in a 360×360 plot, at 60 fps.
    fn touch_rig() -> Harness<'static, Rig> {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        Harness::builder()
            .with_size(vec2(400.0, 400.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                |ui, (roll, plot): &mut Rig| {
                    let (rect, resp) = ui.allocate_exact_size(vec2(360.0, 360.0), Sense::click_and_drag());
                    *plot = rect;
                    roll.interact(ui, &resp, rect, test_bank());
                },
                (roll, Rect::NOTHING),
            )
    }

    /// A finger pulls the rows down — the window looks higher — and the window keeps
    /// coasting after the finger lifts, then stops. The auto framing has let go: the
    /// note it would aim at no longer moves the window.
    #[test]
    fn a_finger_drags_the_rows_and_flings_them() {
        let mut harness = touch_rig();
        harness.run_steps(2);
        let plot = harness.state().1;
        let before = edges(&harness.state().0);
        let mid = plot.center();
        // A quick downward flick: 90 px in 5 frames.
        gesture(
            &mut harness,
            &[(Finger::FIRST, mid - vec2(0.0, 45.0), mid + vec2(0.0, 45.0))],
            5,
        );
        let released = edges(&harness.state().0);
        assert!(
            released.0 > before.0 + 1.0,
            "pulling the rows down must look higher: lo {:.2} → {:.2}",
            before.0,
            released.0
        );
        harness.run_steps(60);
        let coasted = edges(&harness.state().0);
        assert!(
            coasted.0 > released.0 + 0.5,
            "the window did not coast after the release: lo {:.2} → {:.2}",
            released.0,
            coasted.0
        );
        assert_eq!(harness.state().0.view.framing(), Framing::Manual);
        harness.run_steps(120);
        let rested = edges(&harness.state().0);
        harness.run_steps(10);
        assert_eq!(edges(&harness.state().0), rested, "the coast never came to rest");
    }

    /// A double-tap hands the window back: it glides home to the note being played —
    /// the auto framing's own aim (A4 in the middle), not wherever the window happened
    /// to be when the finger took it.
    #[test]
    fn a_double_tap_gives_the_window_back_to_the_auto_framing() {
        let mut harness = touch_rig();
        harness.run_steps(2);
        let home = edges(&harness.state().0);
        let mid = harness.state().1.center();
        gesture(&mut harness, &[(Finger::FIRST, mid, mid + vec2(0.0, 120.0))], 6);
        harness.run_steps(120);
        assert!(
            edges(&harness.state().0).0 > home.0 + 2.0,
            "the drag did not move the window"
        );

        gesture(&mut harness, &[(Finger::FIRST, mid, mid)], 1);
        gesture(&mut harness, &[(Finger::FIRST, mid, mid)], 1);
        assert_eq!(harness.state().0.view.framing(), Framing::Auto);
        harness.run_steps(180);
        let back = edges(&harness.state().0);
        let center = 0.5 * (back.0 + back.1);
        assert!(
            (center - 69.0).abs() < 0.1 && back.1 - back.0 >= MIN_SPAN - 0.1,
            "the window did not glide home to A4: {back:?}"
        );
    }

    /// Two fingers spreading up-down zoom in about the point between them: the span
    /// shrinks by the spread, and the pitch under the fingers stays under them.
    #[test]
    fn a_pinch_zooms_about_the_fingers() {
        let mut harness = touch_rig();
        harness.run_steps(2);
        let plot = harness.state().1;
        let at = pos2(plot.center().x, plot.top() + 120.0);
        let pitch_at = |rig: &Rig| {
            let (lo, hi) = edges(&rig.0);
            lo + (rig.1.bottom() - at.y) / rig.1.height() * (hi - lo)
        };
        let (span_before, under_before) = (harness.state().0.view.span().0, pitch_at(harness.state()));
        gesture(
            &mut harness,
            &[
                (Finger::FIRST, at - vec2(0.0, 20.0), at - vec2(0.0, 40.0)),
                (Finger::SECOND, at + vec2(0.0, 20.0), at + vec2(0.0, 40.0)),
            ],
            10,
        );
        let span = harness.state().0.view.span().0;
        assert!(
            (span - span_before / 2.0).abs() < 0.1 * span_before,
            "a 2× spread must halve the span: {span_before:.2} → {span:.2}"
        );
        let under = pitch_at(harness.state());
        assert!(
            (under - under_before).abs() < 0.5,
            "the pitch between the fingers moved: {under_before:.2} → {under:.2}"
        );
    }
}
