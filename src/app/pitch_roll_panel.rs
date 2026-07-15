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
    Align,
    CornerRadius,
    Frame,
    Layout,
    Margin,
    RichText,
    Sense,
    Stroke,
    Ui,
    vec2,
};

use super::App;
use crate::audio::{
    MelodyFrame,
    MelodyHistory,
};
use crate::ui::pianoroll::{
    self,
    PitchPoint,
    RollColumn,
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
/// How much of the past the roll keeps, **in seconds of played audio** — exactly what
/// the plot shows, read from the renderer so there is one number and not two.
///
/// It replaces a count of 600 UI frames, which was a span of seconds only if you knew
/// the frame rate and it never changed ("~10 s at 60 fps, ~20 s at 30 fps"). Enforced
/// against [`MelodyFrame::t`], so 30 fps, 60 fps and a stuttering frame all hold the
/// same ten seconds.
const VISIBLE_SECONDS: f64 = pianoroll::VISIBLE_SECONDS as f64;
/// Padding (semitones) kept above/below the played range so the line never rides
/// the very edge of the view.
const VIEW_PAD: f32 = 2.5;
/// Minimum visible pitch span (semitones) so a single sustained note still shows a
/// sensible band of rows around it instead of one giant row.
const MIN_SPAN: f32 = 14.0;
/// Per-frame easing for the view window toward its target range. Small = the rows
/// glide rather than jump when the played range changes.
const VIEW_EASE: f32 = 0.08;

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
    frames:  MelodyHistory,
    /// How far the panel has read the engine's history — the `seq` of the last frame
    /// taken. The cursor is ours, not the engine's, so the staff can hold its own and
    /// read the same frames (see `AudioEngine::melody_since`).
    cursor:  Option<u64>,
    /// Eased fractional-MIDI window shown on screen (`view_lo` bottom, `view_hi`
    /// top). Auto-frames the graph on the played range without per-frame jumps.
    view_lo: f32,
    view_hi: f32,
    /// Which layer is painted under the line — see [`RollLayer`].
    layer:   RollLayer,
}

impl Default for PitchRoll {
    fn default() -> Self {
        // Default window ≈ violin open strings (G3=55 .. E5=76) with headroom, so
        // the panel looks sensible before the first note eases it to real data.
        Self {
            frames:  MelodyHistory::with_retention(VISIBLE_SECONDS),
            cursor:  None,
            view_lo: 53.0,
            view_hi: 79.0,
            layer:   RollLayer::default(),
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
        let recent = self.frames.len().saturating_sub(FRAMING_FRAMES);
        let mut pitches: Vec<f32> = self
            .frames
            .iter()
            .skip(recent)
            .filter_map(|frame| frame.pitch)
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
        if let Some(now) = self.sounding() {
            self.view_lo = self.view_lo.min(now - VIEW_PAD);
            self.view_hi = self.view_hi.max(now + VIEW_PAD);
        }
    }

    /// The most recent pitch the engine actually gave us — the note sounding now,
    /// looking back past any silent/rejected frames at the tail.
    fn sounding(&self) -> Option<f32> {
        self.frames.iter().rev().find_map(|frame| frame.pitch)
    }

    /// The frames as the renderer wants them: aged against the playhead, with the
    /// heat gated for display.
    ///
    /// The gate is **display only**, and is not the engine's silence gate wearing a
    /// disguise (see [`HEAT_LEVEL_GATE`]). It stays here because it decides nothing:
    /// the line's `pitch` arrives already gated and already octave-judged.
    fn columns(&self, now: f64) -> Vec<RollColumn<'_>> {
        self.frames
            .iter()
            .map(|frame| {
                RollColumn {
                    age_s: (now - frame.t) as f32,
                    pitch: frame.pitch.map(|midi_f| {
                        PitchPoint {
                            midi_f,
                            level: frame.level,
                        }
                    }),
                    // Both layers are normalized to their own column's max upstream, so
                    // both need the same silence gate for the same reason — see
                    // [`HEAT_LEVEL_GATE`]. A quiet salience column is room noise scored
                    // and stretched to full scale, which paints the rests just as
                    // convincingly as a quiet spectrum does.
                    heat:  if frame.level >= HEAT_LEVEL_GATE {
                        match self.layer {
                            RollLayer::Spectrum => frame.heat.as_slice(),
                            // `None` = a column with no energy to score at all, which is
                            // the same picture as a gated one: a gap. Nothing is lost by
                            // folding them together — neither has evidence in it.
                            RollLayer::Salience => frame.salience.as_deref().unwrap_or(&[]),
                        }
                    } else {
                        &[]
                    },
                }
            })
            .collect()
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

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Pitch Roll").size(20.0).color(color::TEXT_HEADING));
                        // The hint names what is *under* the line, because the two layers
                        // carry different authority and the picture cannot say so itself:
                        // one may convict the line, the other cannot (see `RollLayer`).
                        ui.label(
                            RichText::new(match self.pitch_roll.layer {
                                RollLayer::Spectrum => "Rows are notes; your pitch scrolls in from the right",
                                RollLayer::Salience => {
                                    "Under the line: what SWIPE′ scored — evidence, not ground truth"
                                }
                            })
                            .color(color::TEXT_HINT)
                            .size(12.0),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.selectable_value(&mut self.pitch_roll.layer, RollLayer::Salience, "SWIPE′");
                        ui.selectable_value(&mut self.pitch_roll.layer, RollLayer::Spectrum, "spectrum");
                    });
                });

                ui.add_space(10.0);

                let width = ui.available_width();
                let (rect, _resp) = ui.allocate_exact_size(vec2(width, 360.0), Sense::hover());
                let painter = ui.painter_at(rect);
                // The playhead is the newest frame's own timestamp, not "now": there is
                // no reading of the clock here at all, so a late repaint draws the same
                // picture, just later.
                let Some(now) = self.pitch_roll.now() else {
                    pianoroll::draw_pitch_roll(
                        &painter,
                        rect,
                        &[],
                        res_min_midi,
                        res_max_midi,
                        self.pitch_roll.view_lo,
                        self.pitch_roll.view_hi,
                        style,
                    );
                    return;
                };
                pianoroll::draw_pitch_roll(
                    &painter,
                    rect,
                    &self.pitch_roll.columns(now),
                    res_min_midi,
                    res_max_midi,
                    self.pitch_roll.view_lo,
                    self.pitch_roll.view_hi,
                    style,
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                pitch,
                level,
                heat: Vec::new(),
                salience: None,
            }
        }
    }

    /// Feed the line one frame at a time (delivery = the bank's own cadence).
    fn line(bank: &mut Bank, roll: &mut PitchRoll, pitch: Option<f32>, level: f32) {
        let frame = bank.frame(pitch, level);
        roll.update(vec![frame]);
    }

    /// The toggle swaps the layer, and both layers obey the same silence gate.
    ///
    /// Worth a test despite looking like a `match`: the two layers are the *same length on
    /// the same grid* by design, so swapping them is invisible to the type system and a
    /// crossed wire would paint a plausible field of the wrong quantity — the failure would
    /// read as "SWIPE′ looks a lot like the spectrum", which is a conclusion, not a bug
    /// report. The gate has to be repeated per layer for the same reason it exists at all:
    /// both are normalized to their own column's max, so both turn room noise into a
    /// full-scale wash across the rests.
    #[test]
    fn the_toggle_swaps_the_layer_and_both_obey_the_gate() {
        let loud = |salience| {
            vec![MelodyFrame {
                seq: 1,
                t: 0.0,
                pitch: Some(69.0),
                level: 0.5, // over HEAT_LEVEL_GATE
                heat: vec![1.0, 0.0],
                salience,
            }]
        };

        let mut roll = PitchRoll::default();
        roll.update(loud(Some(vec![0.0, 1.0])));
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
            seq:      1,
            t:        0.0,
            pitch:    None,
            level:    HEAT_LEVEL_GATE * 0.5,
            heat:     vec![1.0, 0.0],
            salience: Some(vec![0.0, 1.0]),
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
            |roll: &PitchRoll| -> Vec<Option<f32>> { roll.frames.iter().map(|f| f.pitch).collect() };
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
            seq:      bank.seq + 1,
            t:        0.0,
            pitch:    Some(60.0),
            level:    0.5,
            heat:     Vec::new(),
            salience: None,
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
        let (lo, hi) = (roll.view_lo, roll.view_hi);
        for _ in 0..30 {
            line(&mut bank, &mut roll, None, 0.0);
        }
        assert_eq!(roll.view_lo, lo);
        assert_eq!(roll.view_hi, hi);
    }

    /// A sustained note eases the window to frame it, keeping the min span.
    #[test]
    fn note_reframes_within_min_span() {
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(69.0), 0.5); // A4
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..200 {
            line(&mut bank, &mut roll, Some(48.0), 0.5); // C3, low phrase
        }
        for _ in 0..600 {
            line(&mut bank, &mut roll, Some(84.0), 0.5); // C6, settle up high
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        // A realistic phrase: a violin first position wandering over a fifth or so.
        for midi in [67.0f32, 69.0, 71.0, 72.0, 74.0, 71.0, 69.0]
            .iter()
            .cycle()
            .take(600)
        {
            line(&mut bank, &mut roll, Some(*midi), 0.5);
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(69.0), 0.5); // steady A4
        }
        let span_before = roll.view_hi - roll.view_lo;
        line(&mut bank, &mut roll, Some(81.0), 0.5); // one slipped frame, an octave up
        for _ in 0..200 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..400 {
            line(&mut bank, &mut roll, Some(60.0), 0.5); // settled on C4
        }
        // Leap up and hold briefly — far too few frames to move the quantiles.
        for _ in 0..3 {
            line(&mut bank, &mut roll, Some(79.0), 0.5); // G5
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
        let (mut bank, mut roll) = (Bank::new(), PitchRoll::default());
        for _ in 0..800 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        let (lo, hi) = (roll.view_lo, roll.view_hi);
        for _ in 0..60 {
            line(&mut bank, &mut roll, Some(69.0), 0.5);
        }
        assert!(
            (roll.view_lo - lo).abs() < 0.5 && (roll.view_hi - hi).abs() < 0.5,
            "view crept from ({lo:.2}, {hi:.2}) to ({:.2}, {:.2}) on a held note",
            roll.view_lo,
            roll.view_hi
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
}
