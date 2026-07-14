//! Violin trainer panel — renders live notation.
//!
//! The panel watches the monophonic pitch detector (`AudioEngine::reading`) and
//! "writes" what you play onto a single staff: the note sounding right now is
//! drawn large at the right with an intonation colour + cents readout, and each
//! finished note scrolls left, building a running line of notation — a
//! reading/intonation trainer. The clef (treble / bass / tenor) is user-
//! selectable, so a violin, cello or viola player each reads in their own.
//!
//! State (the note history + the note currently being held) lives in
//! [`StaffTrainer`], held on `App`. The stateless staff/clef drawing lives in
//! [`crate::ui::staff`]; this module owns only the capture logic and layout.

use std::collections::VecDeque;

use eframe::egui::{
    Align2,
    Color32,
    CornerRadius,
    FontId,
    Frame,
    Margin,
    Painter,
    Rect,
    RichText,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use super::{
    App,
    pill_colored,
    pill_muted,
};
use crate::core_types::note::{
    AccidentalStyle,
    CIRCLE_OF_FIFTHS,
    KeySignature,
};
use crate::ui::segmented::{
    PillCombo,
    RowCaption,
    SegmentedButton,
};
use crate::ui::staff::{
    self,
    Clef,
    StaffGeom,
};
use crate::ui::theme::intonation_color;
use crate::ui::tokens::color;

/// Ink for the live-note chip, which is painted on `intonation_color(cents)` —
/// a saturated green→red that shifts under the text as you play. Near-black so
/// it stays legible across that whole ramp; it is not [`color::TEXT_BADGE`]
/// because the surface it sits on is data, not a fixed badge fill.
const PILL_INK_ON_INTONATION: Color32 = Color32::from_rgb(20, 22, 26);

/// Engraving ink: the clef and the key signature. File-local — this is the
/// staff's own ink, and no other panel engraves. Warmer and brighter than the
/// staff *lines* it sits on, which stay a plain grey so the glyphs lead.
const STAFF_INK: Color32 = Color32::from_rgb(216, 208, 196);
/// The five staff lines. Cool and dim against the warm [`STAFF_INK`] glyphs, so
/// the notation leads and the ruling recedes.
const STAFF_LINE: Color32 = Color32::from_rgb(96, 104, 116);
/// The live pitch trail flowing into the current note — the one blue in the
/// panel, so it never reads as intonation (which is the green→red ramp).
const TRAIL_BLUE: Color32 = Color32::from_rgb(96, 176, 214);

/// Input level (RMS-ish, 0..1) below this is treated as silence. The audio engine
/// never declares silence on its own (`SILENCE_RMS_THRESHOLD == 0.0`), so without
/// this gate the pitch detector latches onto room noise and "writes" ghost notes.
const LEVEL_GATE: f32 = 0.02;
/// A held note shorter than this (seconds) is discarded as a glitch, not written.
const MIN_NOTE_SECONDS: f64 = 0.06;
/// Grace period of silence before the held note is committed and cleared, so a
/// brief pitch dropout mid-note does not chop it in two.
const RELEASE_SECONDS: f64 = 0.14;
/// Smoothing for the live cents of the held note (per-frame EMA weight).
const CENTS_EMA: f32 = 0.25;
/// How many finished notes to keep. Only those that fit on screen are drawn; the
/// rest have already scrolled off, but we keep a few extra for when the panel is
/// widened.
const HISTORY_CAP: usize = 96;
/// Frames of continuous pitch kept for the live "waterfall" trail behind the
/// notes (~a few seconds at UI frame rate).
const TRAIL_CAP: usize = 240;

/// One finished note on the staff.
pub struct StaffNote {
    pub midi:  i32,
    /// Deviation from equal temperament, cents. Colours the notehead so past
    /// intonation stays visible.
    pub cents: f32,
}

/// The note currently sounding, still accumulating.
struct HeldNote {
    midi:  i32,
    cents: f32,
    onset: f64,
    last:  f64,
}

/// One frame of continuous pitch for the waterfall trail: `(fractional MIDI,
/// level)`. `None` marks a silent frame (a gap in the trail).
type TrailPoint = Option<(f32, f32)>;

/// Live-notation state for the violin trainer panel.
#[derive(Default)]
pub struct StaffTrainer {
    /// Finished notes, oldest → newest.
    history:        VecDeque<StaffNote>,
    /// The note sounding right now (drawn emphasised at the right edge).
    current:        Option<HeldNote>,
    /// Recent continuous pitch, oldest → newest, for the trail behind the notes.
    trail:          VecDeque<TrailPoint>,
    /// The clef the staff is drawn in — user-selectable (default treble). One
    /// staff at a time: notes never migrate between clefs.
    clef:           Clef,
    /// The key signature drawn after the clef (default C major = none). It fixes
    /// the sharps/flats at the clef and, in turn, how each note is spelled and
    /// whether it carries its own accidental (see [`KeySignature::note_glyph`]).
    key:            KeySignature,
    /// Last onset counter seen from the engine; a change means a fresh attack, used
    /// to split a re-bowed repeat of the same pitch into a new note.
    last_onset_seq: u64,
}

impl StaffTrainer {
    /// Feed one UI frame. `pitch` is `Some((midi, cents, fractional_midi))` when a
    /// confident, in-range pitch was detected, `None` for silence. `level` is the
    /// input level for trail intensity. `now` is the egui clock in seconds.
    pub fn update(&mut self, pitch: Option<(i32, f32, f32)>, level: f32, now: f64, onset_seq: u64) {
        // Feed the continuous trail every frame (newest at the back).
        self.trail.push_back(pitch.map(|(_, _, midi_f)| (midi_f, level)));
        while self.trail.len() > TRAIL_CAP {
            self.trail.pop_front();
        }

        let detected = pitch.map(|(midi, cents, _)| (midi, cents));
        match detected {
            Some((midi, cents)) => {
                // A fresh attack (onset counter moved) forces a new note even at the
                // same pitch, so a re-bowed repeat isn't merged into the held note.
                // Consumed only here, in the voiced branch: an onset during the
                // attack's brief unvoiced flicker stays pending until the pitch
                // returns, so it still splits.
                let onset = onset_seq != self.last_onset_seq;
                self.last_onset_seq = onset_seq;
                let same = self.current.as_ref().is_some_and(|h| h.midi == midi);
                if same && !onset {
                    // Same note still sounding: extend it and smooth the cents.
                    let h = self.current.as_mut().unwrap();
                    h.cents = h.cents * (1.0 - CENTS_EMA) + cents * CENTS_EMA;
                    h.last = now;
                } else {
                    // Different pitch, a re-attack of the same pitch, or the first
                    // note: finish the old, begin new.
                    self.end_current();
                    self.current = Some(HeldNote {
                        midi,
                        cents,
                        onset: now,
                        last: now,
                    });
                }
            }
            None => {
                // Commit the held note once the silence outlasts the grace period.
                let expired = self
                    .current
                    .as_ref()
                    .is_some_and(|h| now - h.last > RELEASE_SECONDS);
                if expired {
                    self.end_current();
                }
            }
        }
    }

    /// Move the held note into history if it lasted long enough to count.
    fn end_current(&mut self) {
        let Some(h) = self.current.take() else {
            return;
        };
        if h.last - h.onset >= MIN_NOTE_SECONDS {
            self.history.push_back(StaffNote {
                midi:  h.midi,
                cents: h.cents,
            });
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
    }

    /// The note sounding right now, `(midi, cents)`, if any.
    fn current(&self) -> Option<(i32, f32)> {
        self.current.as_ref().map(|h| (h.midi, h.cents))
    }
}

impl App {
    pub(super) fn draw_staff_card(&mut self, ui: &mut Ui) {
        // Live panel: keep repainting so the notation tracks the audio thread.
        ui.ctx().request_repaint();
        // The played-note source is now the resonator bank (`fast_pitch`), and the
        // bank only runs while a consumer keeps asking for it — without this it
        // parks and `fast_pitch` stays `None` ("play a note…" forever).
        self.audio.request_resonator();

        let settings = self.audio.analysis_settings();
        // The key signature governs spelling: a sharp key spells the black notes as
        // sharps, a flat key as flats. C major (no signature) has nothing to spell,
        // so there the global sharps/flats preference still applies.
        let key = self.staff.key;
        let style = key.style().unwrap_or(settings.accidental);
        let reading = self.audio.reading();
        let level = self.audio.input_level();

        // The note to write comes from `melody_pitch`: the resonator bank's fast fine
        // pitch with its octave pinned by pYIN (see `audio::dsp::melody`). Nearest
        // semitone = the note, the fractional part = cents.
        //
        // NOT `reading.frequency_hz` — that is pYIN alone, which cannot follow a note
        // change in under ~128 ms because its HMM will not leave a note while the
        // analysis window still holds a trace of it. The bank follows in 8–29 ms,
        // inside the ~30–40 ms perceptual threshold. Writing the staff from
        // `frequency_hz` is exactly the regression this panel had.
        //
        // Gated on absolute input level, which is the real silence gate: the bank's
        // column is normalized, so it reports *some* fundamental even for room noise.
        // No range clamp: the engine's range is the bank's grid, and a second opinion
        // about it here could only drift from that one — the panel's old ceiling of
        // G7 already sat below the grid's C8.
        let pitch = (level >= LEVEL_GATE)
            .then_some(reading.as_ref())
            .flatten()
            .and_then(|r| r.melody_pitch)
            .map(|(midi_f, _strength)| {
                let midi = midi_f.round() as i32;
                (midi, (midi_f - midi as f32) * 100.0, midi_f)
            });
        let now = ui.input(|i| i.time);
        let onset_seq = reading.as_ref().map_or(0, |r| r.onset_seq);
        self.staff.update(pitch, level, now, onset_seq);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Violin Staff")
                                .size(20.0)
                                .color(color::TEXT_HEADING),
                        );
                        ui.label(
                            RichText::new("Play — your notes are written on the staff")
                                .color(color::TEXT_HINT)
                                .size(12.0),
                        );
                    });
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            match self.staff.current() {
                                Some((midi, cents)) => {
                                    pill_colored(
                                        ui,
                                        &format!("{}  {:+.0}\u{00A2}", style.midi_name(midi), cents),
                                        PILL_INK_ON_INTONATION,
                                        intonation_color(cents),
                                    )
                                }
                                None => pill_muted(ui, "\u{2014}"),
                            }
                        },
                    );
                });

                // Clef picker — one staff at a time (a violin reads treble, a
                // cello/bass the lower clefs). The choice lives in `StaffTrainer`.
                ui.horizontal(|ui| {
                    // Both captions keep their smaller, dimmer secondary look; only the
                    // vertical band comes from `RowCaption`, so they line up with the
                    // pills instead of riding above them.
                    ui.add(RowCaption::new("Clef").font_size(12.0).color(color::TEXT_HINT));
                    // `selectable_value` drew a fill only when selected, leaving Bass and
                    // Tenor as bare text with no outline — the exact inconsistency the
                    // canonical pill exists to kill.
                    for clef in Clef::ALL {
                        if ui
                            .add(SegmentedButton::new(clef.label(), self.staff.clef == clef))
                            .clicked()
                        {
                            self.staff.clef = clef;
                        }
                    }

                    ui.add_space(18.0);

                    // Key-signature picker — the circle of fifths. Selecting a key
                    // both draws its sharps/flats at the clef and re-spells the
                    // notes accordingly (see `KeySignature`).
                    ui.add(RowCaption::new("Key").font_size(12.0).color(color::TEXT_HINT));
                    // The snug popup style these short rows need is baked into
                    // `PillCombo`, so it is no longer spelled out here.
                    PillCombo::new("staff_key_sig", key_label(self.staff.key)).show(ui, |ui| {
                        for &(fifths, name) in CIRCLE_OF_FIFTHS.iter() {
                            let k = KeySignature { fifths };
                            ui.selectable_value(&mut self.staff.key, k, key_label(k))
                                .on_hover_text(format!("{name} major"));
                        }
                    });
                });

                ui.add_space(10.0);

                let width = ui.available_width();
                let (rect, _resp) = ui.allocate_exact_size(vec2(width, 260.0), Sense::hover());
                let painter = ui.painter_at(rect);
                // Fast, low-latency pitch-energy history straight from the resonator
                // bank (published ~60 Hz), independent of the slow YIN note-commit.
                // Empty until the first reading; its bin→pitch mapping spans the
                // bank's own MIDI range (see `draw_resonator_waterfall`).
                let res_wf = reading
                    .as_ref()
                    .map_or(&[][..], |r| r.resonator_waterfall.as_slice());
                let res_min = settings.resonator.min_midi.as_u8() as i32;
                let res_max = settings.resonator.max_midi.as_u8() as i32;
                draw_staff(&painter, rect, &self.staff, style, res_wf, res_min, res_max);
            });
    }
}

/// Compact label for the key picker, e.g. `"C"`, `"G  1\u{266F}"`, `"E\u{266D}  3\u{266D}"`.
/// The tonic name comes from the circle-of-fifths table; the suffix is the
/// accidental count so the signature is readable without opening the dropdown.
fn key_label(key: KeySignature) -> String {
    let name = CIRCLE_OF_FIFTHS
        .iter()
        .find(|&&(f, _)| f == key.fifths)
        .map_or("?", |&(_, n)| n);
    match key.count() {
        0 => name.to_string(),
        n => format!("{name}  {n}{}", key.accidental().glyph()),
    }
}

/// Paint the staff in `trainer.clef`, its clef glyph, the written notes (newest
/// at the right) and the intonation bar into `rect`.
///
/// `res_wf` is the resonator bank's magnitude history (newest last), `res_min/max_midi`
/// the bank's MIDI range — together they drive the fast pitch-energy waterfall
/// drawn on the lines behind the notes.
#[allow(clippy::too_many_arguments)]
fn draw_staff(
    painter: &Painter,
    rect: Rect,
    trainer: &StaffTrainer,
    style: AccidentalStyle,
    res_wf: &[Vec<f32>],
    res_min_midi: i32,
    res_max_midi: i32,
) {
    let clef = trainer.clef;
    let key = trainer.key;
    // Gap scales with height; the middle line sits at the vertical centre so there
    // is head-room for ledger lines both above and below.
    let gap = (rect.height() / 15.0).clamp(9.0, 20.0);
    let left = rect.left() + 12.0;
    let mut geom = StaffGeom {
        gap,
        // Anchor the staff ~3.6 gaps below the card's top edge — enough for the
        // note-name header row plus a couple of ledger lines of high-register
        // head-room — rather than centring it. Centring left ~5.5 gaps of dead
        // space above the top line, so the (usually empty) staff read as having
        // slid down to the bottom of the card. The remaining head-room now falls
        // below, where the intonation bar and the low ledger lines live.
        bottom_y: rect.top() + gap * 7.6,
        staff_left: left,
        clef_x: left + gap * 2.2,
        notes_left: left + gap * 6.2,
        notes_right: rect.right() - gap * 1.4,
    };

    let staff_col = STAFF_LINE;
    staff::draw_staff_lines(painter, &geom, staff_col);
    staff::draw_clef(painter, &geom, clef, STAFF_INK);

    // Key signature between the clef and the notes; push the note region right so
    // noteheads never collide with the signature (no-op for C major).
    let ksig_right = staff::draw_key_signature(painter, &geom, clef, key, STAFF_INK);
    geom.notes_left = geom.notes_left.max(ksig_right + gap * 0.8);

    // The current note (and the trail's newest sample) live at this x; notes step
    // left from here, the trail flows into it from the left.
    let right_x = geom.notes_right - gap * 1.6;

    // Fast resonator waterfall on the very bottom layer: the low-latency
    // pitch-energy heat that flows into the notes far sooner than the YIN commit.
    draw_resonator_waterfall(
        painter,
        &geom,
        clef,
        res_wf,
        res_min_midi,
        res_max_midi,
        style,
        right_x,
    );

    // Live pitch "waterfall" trail behind the notes, aligned to the staff lines:
    // the continuous detected pitch as a fading glow (shows glide, vibrato, lag).
    draw_trail(painter, &geom, clef, trainer, style, right_x);

    // Notes newest-last. Place right→left so a new note enters at the right and
    // older ones scroll toward the clef, dropping off once past `notes_left`.
    let mut items: Vec<(i32, f32, bool)> = trainer.history.iter().map(|n| (n.midi, n.cents, false)).collect();
    if let Some((midi, cents)) = trainer.current() {
        items.push((midi, cents, true));
    }

    let advance = gap * 3.2;
    let n = items.len();
    for (i, &(midi, cents, emphasize)) in items.iter().enumerate() {
        let x = right_x - (n - 1 - i) as f32 * advance;
        if x < geom.notes_left {
            continue;
        }
        let color = intonation_color(cents);
        staff::draw_note(
            painter, &geom, x, midi, style, clef, key, color, staff_col, emphasize,
        );
        // Name *every* note above the staff: a header row of note letters (C, D,
        // F#, …), each aligned to its note's column, so the written line reads
        // back as named pitches. The label is the pitch-class name only (no
        // octave — "abcdef"), coloured to match the notehead's intonation; the
        // note sounding right now is drawn a touch larger for emphasis. A fixed
        // top row (rather than following each notehead's varying height) keeps the
        // letters on one clean readable line above the staff.
        let name = style.pitch_class_name(midi.rem_euclid(12) as usize);
        let name_size = if emphasize { gap * 1.3 } else { gap * 1.05 };
        painter.text(
            pos2(x, rect.top() + gap * 0.5),
            Align2::CENTER_TOP,
            name,
            FontId::proportional(name_size),
            color,
        );
    }

    // Intonation needle for the note currently sounding.
    if let Some((_midi, cents)) = trainer.current() {
        draw_intonation_bar(painter, rect, cents);
    } else if items.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "play a note…",
            FontId::proportional(gap * 1.1),
            color::TEXT_MUTED,
        );
    }
}

/// A horizontal in-tune meter along the bottom: centre = in tune, marker slides
/// to ±50 cents and takes the intonation colour.
fn draw_intonation_bar(painter: &Painter, rect: Rect, cents: f32) {
    let w = (rect.width() * 0.5).min(320.0);
    let cx = rect.center().x;
    let y = rect.bottom() - 14.0;
    let (l, r) = (cx - w * 0.5, cx + w * 0.5);
    painter.line_segment(
        [pos2(l, y), pos2(r, y)],
        Stroke::new(3.0, color::GRID_LINE_STRONG),
    );
    // Centre tick (perfectly in tune).
    painter.line_segment(
        [pos2(cx, y - 7.0), pos2(cx, y + 7.0)],
        Stroke::new(2.0, color::TEXT_MUTED),
    );
    let t = (cents / 50.0).clamp(-1.0, 1.0);
    painter.circle_filled(pos2(cx + t * w * 0.5, y), 6.0, intonation_color(cents));
}

/// The live pitch trail: each recent frame's continuous pitch as a small fading
/// dot at its exact staff height, newest at `right_x` flowing left. Reads as a
/// soft "waterfall" of what the ear/detector hears, right on the staff lines —
/// the continuous counterpart to the quantised noteheads on top.
fn draw_trail(
    painter: &Painter,
    geom: &StaffGeom,
    clef: Clef,
    trainer: &StaffTrainer,
    style: AccidentalStyle,
    right_x: f32,
) {
    let n = trainer.trail.len();
    if n == 0 {
        return;
    }
    let dx = ((right_x - geom.notes_left) / TRAIL_CAP as f32).max(0.8);
    let radius = (geom.gap * 0.17).max(1.2);
    for (i, point) in trainer.trail.iter().enumerate() {
        let Some((midi_f, level)) = *point else {
            continue; // silent frame → gap in the trail
        };
        let age = (n - 1 - i) as f32; // 0 = newest
        let x = right_x - age * dx;
        if x < geom.notes_left {
            continue;
        }
        let y = staff::midi_to_y(geom, style, clef, midi_f);
        // Fade with age, intensify with level.
        let recency = 1.0 - age / n as f32;
        let alpha = (level.clamp(0.0, 1.0).sqrt() * recency * 165.0).clamp(0.0, 165.0) as u8;
        painter.circle_filled(
            pos2(x, y),
            radius,
            TRAIL_BLUE.gamma_multiply(alpha as f32 / 255.0),
        );
    }
}

/// Below this normalised magnitude a resonator bin is treated as noise and not
/// painted — keeps the waterfall sparse (a note + its partials, not a wash) and
/// the shape count low.
const RES_WF_GATE: f32 = 0.18;

/// Fast pitch-energy "waterfall" from the resonator bank, painted on the staff
/// lines behind everything else. Columns are the bank's magnitude history
/// (newest at `right_x`, older to the left); within a column each bin above
/// [`RES_WF_GATE`] becomes a small ~gap-sized rect at *its own* pitch height, so
/// energy lands exactly on the line/space of the note it belongs to.
///
/// This is the low-latency counterpart to the YIN noteheads: the bank publishes
/// at ~60 Hz with a per-sample response, so a new note lights up here well before
/// the windowed detector commits it — the whole point of "fighting the latency".
///
/// Bin→pitch: the bank spans `res_min_midi..=res_max_midi`; the row length tells
/// us the bins-per-semitone (it changes with the reassignment toggle, so we
/// derive it rather than hard-code it). `midi_to_y` handles the diatonic (non
/// uniform) vertical mapping, the same one the noteheads use.
#[allow(clippy::too_many_arguments)]
fn draw_resonator_waterfall(
    painter: &Painter,
    geom: &StaffGeom,
    clef: Clef,
    waterfall: &[Vec<f32>],
    res_min_midi: i32,
    res_max_midi: i32,
    style: AccidentalStyle,
    right_x: f32,
) {
    let n = waterfall.len();
    let bin_count = waterfall.first().map_or(0, Vec::len);
    if n == 0 || bin_count < 2 || res_max_midi <= res_min_midi {
        return;
    }
    // Derived, not assumed: (bins − 1) spread over the semitone span.
    let bins_per_semitone = (bin_count - 1) as f32 / (res_max_midi - res_min_midi) as f32;

    // Columns march left from the playhead. Clamp the step so the strip stays a
    // thin band of small rects ("напротив линий") rather than fat bars.
    let dx = ((right_x - geom.notes_left) / n as f32).clamp(2.0, 6.0);
    let cell_w = dx.min(5.0);
    let cell_h = (geom.gap * 0.5).max(3.0);
    let clip = painter.clip_rect();

    for (col, row) in waterfall.iter().enumerate() {
        let age = (n - 1 - col) as f32; // 0 = newest, at right_x
        let x = right_x - age * dx;
        if x < geom.notes_left {
            continue;
        }
        let recency = 1.0 - age / n as f32; // fade older columns
        for (bin, &value) in row.iter().enumerate() {
            if value < RES_WF_GATE {
                continue;
            }
            let midi = res_min_midi as f32 + bin as f32 / bins_per_semitone;
            let y = staff::midi_to_y(geom, style, clef, midi);
            // Most of the bank's range sits off the selected clef's staff; clipping
            // here keeps the strip to the visible lines and the cost low.
            if y < clip.top() || y > clip.bottom() {
                continue;
            }
            let alpha = (value * recency * 190.0).clamp(0.0, 190.0) as u8;
            painter.rect_filled(
                Rect::from_center_size(pos2(x, y), vec2(cell_w, cell_h)),
                1.0,
                TRAIL_BLUE.gamma_multiply(alpha as f32 / 255.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LVL: f32 = 0.5; // any level above the gate; the gate itself is in the panel

    /// A gated pitch this frame: `(midi, cents, fractional_midi)`.
    fn play(midi: i32, cents: f32) -> Option<(i32, f32, f32)> {
        Some((midi, cents, midi as f32 + cents / 100.0))
    }

    /// No onset this frame — the counter the engine last reported, unchanged.
    const NO_ONSET: u64 = 0;

    /// REGRESSION: the staff must show a note change promptly, end to end.
    ///
    /// Drives the real melody path — real `ResonatorAnalyzer` + real `PitchTracker`
    /// for the octave anchor → real `dsp::melody` snap → real `OctaveGate` → real
    /// `StaffTrainer` — at UI frame rate, and measures the ms from the true note
    /// change until the staff *shows* the new note. This is the number the user
    /// perceives, and its absence is exactly how the panel came to be driven from
    /// pYIN alone (128 ms ordinary / 328 ms on an octave) without anyone noticing.
    #[test]
    fn end_to_end_latency_probe() {
        use std::f32::consts::TAU;

        use crate::audio::dsp::melody::MelodyTracker;
        use crate::audio::dsp::pyin::PitchTracker;
        use crate::audio::dsp::resonator::ResonatorAnalyzer;
        use crate::core_types::note::AccidentalStyle;

        fn violin_tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
            let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
            (0..len)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    partials
                        .iter()
                        .enumerate()
                        .map(|(h, a)| a * (TAU * frequency_hz * (h + 1) as f32 * t).sin())
                        .sum::<f32>()
                        / 3.0
                })
                .collect()
        }

        /// Drive the whole stack and return ms until the staff shows `to_hz`.
        fn probe(from_hz: f32, to_hz: f32) -> f32 {
            let sr = 48_000.0f32;
            let window_size = 6144usize;
            let analysis_hop = 1920usize; // ANALYSIS_INTERVAL = 40 ms
            let bank_publish_ms = 16.0f32; // ResonatorSettings::update_ms
            let hold = (sr * 0.6) as usize;
            let mut sig = violin_tone(from_hz, sr, hold);
            sig.extend(violin_tone(to_hz, sr, hold));
            let change_ms = hold as f32 / sr * 1000.0;
            let target_midi = (69.0 + 12.0 * (to_hz / 440.0).log2()).round() as i32;

            let mut tracker = PitchTracker::new();
            let mut bank = ResonatorAnalyzer::new(sr);
            let mut melody = MelodyTracker::default();
            let mut staff = StaffTrainer::default();
            // Prime the bank with everything up to the first UI frame, as the live
            // bank (fed continuously from the audio callback) would already be.
            bank.process_samples(&sig[..window_size], true);

            // What the two publish paths have last put on the reading.
            let mut anchor: Option<(f32, f32)> = None;
            let mut melody_pitch: Option<(f32, f32)> = None;
            let mut next_analysis = window_size;
            let mut bank_fed = window_size;
            let mut next_bank_publish_ms = 0.0f32;

            // UI loop at 60 fps, exactly like `draw_staff_card`.
            let frame_ms = 1000.0 / 60.0;
            let mut t_ms = window_size as f32 / sr * 1000.0;
            while t_ms < 1150.0 {
                let now_samples = ((t_ms * sr / 1000.0) as usize).min(sig.len());
                // The bank consumes audio continuously and republishes every ~16 ms.
                if now_samples > bank_fed {
                    bank.process_samples(&sig[bank_fed..now_samples], true);
                    bank_fed = now_samples;
                }
                if t_ms >= next_bank_publish_ms {
                    let fast = bank.snapshot(true, AccidentalStyle::Sharps).fundamental;
                    melody_pitch = melody.update(fast, anchor);
                    next_bank_publish_ms = t_ms + bank_publish_ms;
                }
                // The pYIN path rebuilds the octave anchor every 40 ms.
                if now_samples >= next_analysis && next_analysis + window_size <= sig.len() {
                    let win = &sig[next_analysis - window_size..next_analysis];
                    anchor = tracker
                        .process(win, sr, None, false)
                        .map(|(f, c)| (69.0 + 12.0 * (f / 440.0).log2(), c));
                    next_analysis += analysis_hop;
                }
                let pitch = melody_pitch
                    .map(|(midi_f, _)| (midi_f.round() as i32, (midi_f - midi_f.round()) * 100.0, midi_f));
                staff.update(pitch, LVL, t_ms as f64 / 1000.0, NO_ONSET);
                if staff.current().map(|(m, _)| m) == Some(target_midi) {
                    return t_ms - change_ms;
                }
                t_ms += frame_ms;
            }
            f32::INFINITY
        }

        // Two budgets, because one interval is genuinely harder than the rest.
        //
        // Any leap that CHANGES PITCH CLASS is caught by `melody`'s agreement guard
        // on the first frame, so it costs only the bank (8–29 ms) + the bank's 16 ms
        // publish tick + a 60 fps UI frame. pYIN alone measured 128 ms for these.
        const BUDGET_MS: f32 = 60.0;
        // An OCTAVE leap is the one interval no single frame can tell from the bank
        // slipping an octave (A4 and A5 are the same pitch class), so it additionally
        // pays `melody::LEAP_CONFIRM_FRAMES` × the bank's 16 ms cadence before the
        // leap is believed. That is the deliberate price of not re-breaking the
        // octave wandering the snap exists to fix, and it is the *only* interval that
        // pays it — it was 328 ms when the anchor was trusted outright.
        const OCTAVE_BUDGET_MS: f32 = 100.0;
        println!("\n=== staff end-to-end latency (what the user sees) ===");
        for (name, from, to, budget) in [
            ("A4->B4  (2nd)  ", 440.0f32, 493.88f32, BUDGET_MS),
            ("A4->D5  (4th)  ", 440.0, 587.33, BUDGET_MS),
            ("A4->E5  (5th)  ", 440.0, 659.25, BUDGET_MS),
            ("G3->D4  (violin G->D)", 196.0, 293.66, BUDGET_MS),
            ("A4->A5  (8ve)  ", 440.0, 880.0, OCTAVE_BUDGET_MS),
        ] {
            let ms = probe(from, to);
            println!("{name} -> {ms:>7.1} ms  (budget {budget:.0})");
            assert!(
                ms <= budget,
                "{name}: staff took {ms:.1} ms to show the note, budget {budget:.0} ms"
            );
        }
    }

    /// A one-frame blip shorter than `MIN_NOTE_SECONDS` is shown live but never
    /// committed to the written line.
    #[test]
    fn short_blip_is_not_written() {
        let mut t = StaffTrainer::default();
        t.update(play(69, 3.0), LVL, 0.0, NO_ONSET);
        assert_eq!(t.current(), Some((69, 3.0))); // shown live immediately
        t.update(None, 0.0, RELEASE_SECONDS + 0.01, NO_ONSET); // silence past the grace period
        assert_eq!(t.current(), None);
        assert!(t.history.is_empty()); // held 0 s → discarded
    }

    /// A sustained note commits once released, and its cents are the smoothed EMA.
    #[test]
    fn sustained_note_commits_on_release() {
        let mut t = StaffTrainer::default();
        t.update(play(69, 2.0), LVL, 0.0, NO_ONSET);
        t.update(play(69, 4.0), LVL, 0.10, NO_ONSET); // same pitch: extend + smooth cents
        assert_eq!(t.current().unwrap().0, 69);
        t.update(None, 0.0, 0.10 + RELEASE_SECONDS + 0.01, NO_ONSET);
        assert_eq!(t.current(), None);
        assert_eq!(t.history.len(), 1);
        assert_eq!(t.history[0].midi, 69);
        // EMA: 2*(1-0.25) + 4*0.25 = 2.5
        assert!((t.history[0].cents - 2.5).abs() < 1e-4);
    }

    /// Moving to a new pitch finishes the previous note and starts the new one.
    #[test]
    fn pitch_change_writes_previous() {
        let mut t = StaffTrainer::default();
        t.update(play(69, 0.0), LVL, 0.0, NO_ONSET);
        t.update(play(69, 0.0), LVL, 0.10, NO_ONSET);
        t.update(play(71, 0.0), LVL, 0.12, NO_ONSET); // A4 → B4
        assert_eq!(t.history.len(), 1);
        assert_eq!(t.history[0].midi, 69);
        assert_eq!(t.current().unwrap().0, 71);
    }

    /// A dropout shorter than the grace period does not chop the held note.
    #[test]
    fn brief_dropout_keeps_current() {
        let mut t = StaffTrainer::default();
        t.update(play(62, 0.0), LVL, 0.0, NO_ONSET);
        t.update(None, 0.0, 0.05, NO_ONSET); // < RELEASE_SECONDS → keep sounding
        assert_eq!(t.current().unwrap().0, 62);
        t.update(play(62, 0.0), LVL, 0.06, NO_ONSET); // pitch returns, still the same note
        assert_eq!(t.current().unwrap().0, 62);
        assert!(t.history.is_empty());
    }

    /// REGRESSION: a dropped frame mid-note must not tear the held note apart.
    ///
    /// This is the staff's half of the octave-slip contract. The engine
    /// (`audio::dsp::melody`) judges the octave and hands a rejected slip over as
    /// `None`, exactly like silence — it does that *because* of the asymmetry this
    /// test pins: a `None` is absorbed by `RELEASE_SECONDS`, whereas a wrong-octave
    /// frame would read as a pitch *change*, commit the held note early, open a bogus
    /// one and — the real damage — restart the note's `onset` timer. With slips
    /// recurring, nothing would ever reach `MIN_NOTE_SECONDS` and the staff would
    /// write nothing at all, which is precisely how this was reported live.
    #[test]
    fn a_dropped_frame_does_not_tear_the_held_note() {
        let mut t = StaffTrainer::default();
        for i in 0..5 {
            t.update(play(69, 0.0), LVL, i as f64 * 0.02, NO_ONSET); // hold A4
        }
        // The engine rejected a frame (a slip, or a momentary dropout).
        t.update(None, LVL, 0.10, NO_ONSET);
        assert_eq!(
            t.current().unwrap().0,
            69,
            "a dropped frame must not disturb the held note"
        );
        assert!(
            t.history.is_empty(),
            "a dropped frame must not commit the held note"
        );

        // The note carries on and, once released, is written as exactly ONE note —
        // not the two fragments a slip used to produce.
        t.update(play(69, 0.0), LVL, 0.12, NO_ONSET);
        t.update(None, 0.0, 0.12 + RELEASE_SECONDS + 0.01, NO_ONSET);
        assert_eq!(t.history.len(), 1, "the held note should survive the gap intact");
        assert_eq!(t.history[0].midi, 69);
    }

    /// A leap to a new octave is tracked — the staff must not go deaf after a big
    /// interval. (Whether the leap is real is settled upstream; by the time it
    /// reaches here it is simply a new pitch.)
    #[test]
    fn octave_leap_is_still_written() {
        let mut t = StaffTrainer::default();
        for i in 0..5 {
            t.update(play(69, 0.0), LVL, i as f64 * 0.02, NO_ONSET); // A4
        }
        for i in 0..6 {
            t.update(play(81, 0.0), LVL, 0.10 + i as f64 * 0.02, NO_ONSET); // hold A5
        }
        assert_eq!(t.current().unwrap().0, 81, "a held octave leap must be tracked");
    }

    /// A re-attack on the *same* pitch (onset counter advances) splits into a new
    /// note instead of extending the held one — the repeated-note capability.
    #[test]
    fn re_attack_splits_same_pitch() {
        let mut t = StaffTrainer::default();
        t.update(play(69, 0.0), LVL, 0.0, 1); // first stroke starts
        t.update(play(69, 0.0), LVL, 0.10, 1); // same onset → extend
        assert!(t.history.is_empty());
        t.update(play(69, 0.0), LVL, 0.12, 2); // NEW onset, same pitch → split
        assert_eq!(t.history.len(), 1, "the first stroke should be committed");
        assert_eq!(t.history[0].midi, 69);
        assert_eq!(t.current().unwrap().0, 69); // the second stroke is now current
    }
}
