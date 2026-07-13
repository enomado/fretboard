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
    Color32,
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
use crate::ui::theme::PANEL_FILL;

/// Voiced clarity below which the fused pYIN pitch reads as "no note this frame"
/// for the melody line (pYIN reports its voiced probability as `clarity`).
const CLARITY_GATE: f32 = 0.5;
/// Input level (RMS-ish, 0..1) below this is silence — the engine never declares
/// silence itself, so without this the detector traces room noise. Gates *both*
/// layers: the line goes to a gap and the heat column is blanked.
const LEVEL_GATE: f32 = 0.02;
/// Accepted pitch range (MIDI), matching the pYIN tracker's own grid: C1..≈G7.
const MIDI_MIN: i32 = 24;
const MIDI_MAX: i32 = 103;

/// Reject a sample this many semitones or more off the local median. The fast
/// resonator-bank pitch occasionally crowns an overtone → a lone **+12** octave
/// spike (the "peaks at C6/A5" artefact); a trill or vibrato interval is at most a
/// few semitones. Gating on the *interval* rather than the *duration* is what lets
/// even the fastest trill through while dropping octave jumps — a duration filter
/// (median/glitch-length) can't tell a 2-frame trill note from a 2-frame spike.
const OCTAVE_REJECT: f32 = 7.0;
/// Window (frames) of recent raw pitch whose median is the spike-rejection
/// reference. Short on purpose: a lone spike never moves the median (rejected),
/// but a *sustained* real octave leap re-establishes it within ~3 frames and is
/// then accepted — so genuine leaps aren't permanently folded away.
const MEDIAN_WINDOW: usize = 5;

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

/// Live pitch-roll state for the panel.
pub struct PitchRoll {
    /// Per-frame melody-line pitch (fused pYIN), oldest → newest, *after* spike
    /// rejection. `None` = a silent frame or a rejected octave glitch (a gap).
    samples:    VecDeque<Option<PitchPoint>>,
    /// Per-frame resonator column aligned 1:1 with `samples` (same index = same
    /// instant), oldest → newest. An *empty* `Vec` marks a silent frame. This is
    /// the spectral-heat ground truth; unlike the line it makes no octave decision.
    heat:       VecDeque<Vec<f32>>,
    /// Recent *raw* (pre-filter) voiced pitches, for the spike-rejection median.
    /// Cleared on silence so a new phrase starts fresh. See [`OCTAVE_REJECT`].
    raw_recent: VecDeque<f32>,
    /// Eased fractional-MIDI window shown on screen (`view_lo` bottom, `view_hi`
    /// top). Auto-frames the graph on the played range without per-frame jumps.
    view_lo:    f32,
    view_hi:    f32,
}

impl Default for PitchRoll {
    fn default() -> Self {
        // Default window ≈ violin open strings (G3=55 .. E5=76) with headroom, so
        // the panel looks sensible before the first note eases it to real data.
        Self {
            samples:    VecDeque::with_capacity(HISTORY_FRAMES),
            heat:       VecDeque::with_capacity(HISTORY_FRAMES),
            raw_recent: VecDeque::with_capacity(MEDIAN_WINDOW),
            view_lo:    53.0,
            view_hi:    79.0,
        }
    }
}

impl PitchRoll {
    /// Feed one UI frame. `pitch` is the melody-line pitch `Some(fractional_midi)`
    /// when confident+in-range, `None` for silence; `level` fades the line;
    /// `heat_col` is this frame's resonator column (empty = silent → a gap in the
    /// heat). Isolated octave glitches in the *line* are rejected here (see
    /// [`OCTAVE_REJECT`]) so they never reach the curve or the framing — the heat,
    /// which makes no octave decision, is passed through untouched as ground truth.
    pub fn update(&mut self, pitch: Option<f32>, level: f32, heat_col: Vec<f32>) {
        let accepted = match pitch {
            Some(midi_f) => {
                self.raw_recent.push_back(midi_f);
                while self.raw_recent.len() > MEDIAN_WINDOW {
                    self.raw_recent.pop_front();
                }
                // Judge against the local median once there is enough history; a
                // sample an octave (≥ OCTAVE_REJECT semitones) off it is a rare
                // tracker octave slip, not a note — drop it to a gap. Trill/vibrato
                // intervals are far smaller and always pass.
                let spike =
                    self.raw_recent.len() >= 3 && (midi_f - median(&self.raw_recent)).abs() >= OCTAVE_REJECT;
                (!spike).then_some(PitchPoint { midi_f, level })
            }
            None => {
                // Silence ends the phrase: forget the median so the next note is
                // judged on its own, not against the previous note's octave.
                self.raw_recent.clear();
                None
            }
        };
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

    /// Ease the view window toward the pitch range currently in the buffer. Holds
    /// the last window while fully silent, so the rows don't drift when nothing is
    /// playing.
    fn reframe(&mut self) {
        let mut data_lo = f32::MAX;
        let mut data_hi = f32::MIN;
        for sample in &self.samples {
            if let Some(point) = sample {
                data_lo = data_lo.min(point.midi_f);
                data_hi = data_hi.max(point.midi_f);
            }
        }
        if data_lo > data_hi {
            return; // no pitch in the buffer → keep the current framing
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
        self.view_lo += (target_lo - self.view_lo) * VIEW_EASE;
        self.view_hi += (target_hi - self.view_hi) * VIEW_EASE;
    }
}

/// Median of a small non-empty window — the robust reference for octave-spike
/// rejection (one or two outliers can't move it, unlike a mean).
fn median(values: &VecDeque<f32>) -> f32 {
    let mut v: Vec<f32> = values.iter().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
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
        let reference = settings.concert_pitch_hz;
        let res_min_midi = settings.resonator.min_midi.as_u8() as i32;
        let res_max_midi = settings.resonator.max_midi.as_u8() as i32;
        let reading = self.audio.reading();
        let level = self.audio.input_level();
        let voiced = level >= LEVEL_GATE;

        // MELODY LINE source = the fused pYIN pitch (`frequency_hz`): octave-stable
        // and smoothed, so it reads the melody as one clean curve. It cannot show a
        // fast trill (its window + HMM "stay" bias smear it) — that is the heat's
        // job below. Gated on level + voiced clarity + range.
        let pitch = voiced.then(|| reading.as_ref()).flatten().and_then(|r| {
            if r.frequency_hz <= 0.0 || r.clarity < CLARITY_GATE {
                return None;
            }
            let midi_f = 69.0 + 12.0 * (r.frequency_hz / reference).log2();
            (MIDI_MIN..=MIDI_MAX)
                .contains(&(midi_f.round() as i32))
                .then_some(midi_f)
        });
        // HEAT source = the resonator bank's newest column (fast, per bank column),
        // painted as-is so trills/overtones show with no octave decision. Blanked
        // (empty column) when the input is silent, so rests are clean gaps rather
        // than normalized noise (each column is normalized to its own max).
        let heat_col = if voiced {
            reading
                .as_ref()
                .and_then(|r| r.resonator_waterfall.last().cloned())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.pitch_roll.update(pitch, level, heat_col);

        Frame::new()
            .fill(PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 74)))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Pitch Roll")
                            .size(20.0)
                            .color(Color32::from_rgb(228, 220, 208)),
                    );
                    ui.label(
                        RichText::new("Rows are notes; your pitch scrolls in from the right")
                            .color(Color32::from_rgb(145, 151, 160))
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

    /// A lone +12 octave slip in the line is dropped to a gap, while a trill-sized
    /// interval right after it is kept.
    #[test]
    fn octave_spike_is_rejected_but_trill_kept() {
        let mut roll = PitchRoll::default();
        for _ in 0..5 {
            line(&mut roll, Some(69.0), 0.5); // establish A4
        }
        line(&mut roll, Some(81.0), 0.5); // +12 octave slip → gap
        assert!(roll.samples.back().unwrap().is_none(), "octave slip rejected");
        line(&mut roll, Some(71.0), 0.5); // a whole tone (trill neighbour) → kept
        assert!(roll.samples.back().unwrap().is_some(), "trill interval kept");
    }

    /// A *sustained* octave leap is accepted once the short median follows it, so
    /// real leaps aren't permanently folded away.
    #[test]
    fn sustained_octave_leap_is_accepted() {
        let mut roll = PitchRoll::default();
        for _ in 0..5 {
            line(&mut roll, Some(69.0), 0.5); // A4
        }
        for _ in 0..5 {
            line(&mut roll, Some(81.0), 0.5); // hold A5
        }
        assert!(
            roll.samples.back().unwrap().is_some(),
            "a held octave leap is accepted after the median catches up"
        );
    }
}
