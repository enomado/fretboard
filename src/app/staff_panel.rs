//! Violin trainer panel — renders live notation.
//!
//! The panel "writes" what you play onto a single staff: the note sounding right
//! now is drawn large at the right with an intonation colour + cents readout, and
//! each finished note scrolls left, building a running line of notation — a
//! reading/intonation trainer. The clef (treble / bass / tenor) is user-selectable,
//! so a violin, cello or viola player each reads in their own.
//!
//! **The panel decides nothing about the music.** Where one note ends and the next
//! begins is settled in the engine, on an audio clock, and arrives finished on
//! `TunerReading::note_line` — see [`crate::audio::dsp::segmenter`] for why it may
//! not be decided here. What lives in [`StaffTrainer`] is what is genuinely the
//! panel's own: which clef, which key signature, and the decorative pitch trail.
//! The stateless staff/clef drawing lives in [`crate::ui::staff`].

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
use crate::audio::NoteLine;
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

/// Frames of continuous pitch kept for the live "waterfall" trail behind the
/// notes (~a few seconds at UI frame rate).
const TRAIL_CAP: usize = 240;

/// One frame of continuous pitch for the waterfall trail: `(fractional MIDI,
/// level)`. `None` marks a silent frame (a gap in the trail).
type TrailPoint = Option<(f32, f32)>;

/// What the staff panel itself owns.
///
/// Note capture is **not** here: it is the engine's, on the engine's clock (see the
/// module docs). What is left is the reading surface — which clef, which key — plus
/// the trail, which is a decoration rather than a decision.
#[derive(Default)]
pub struct StaffTrainer {
    /// Recent continuous pitch, oldest → newest, for the trail behind the notes.
    ///
    /// Still sampled per UI frame, and that is tolerable *only* because nothing is
    /// decided from it: it is a fading glow showing glide and vibrato, so a dropped
    /// frame costs one dot. Anything read off it would inherit the UI clock, which is
    /// the mistake `dsp::segmenter` exists to undo — the pitch roll's history has the
    /// same shape and the same caveat (`docs/note_detection.md` §4).
    trail: VecDeque<TrailPoint>,
    /// The clef the staff is drawn in — user-selectable (default treble). One
    /// staff at a time: notes never migrate between clefs.
    clef:  Clef,
    /// The key signature drawn after the clef (default C major = none). It fixes
    /// the sharps/flats at the clef and, in turn, how each note is spelled and
    /// whether it carries its own accidental (see [`KeySignature::note_glyph`]).
    key:   KeySignature,
}

impl StaffTrainer {
    /// Feed one UI frame of melody pitch into the trail. `pitch` is the melody line's
    /// fractional MIDI (`None` = silence or a rejected slip, already decided in the
    /// engine); `level` sets the trail's intensity.
    pub fn update(&mut self, pitch: Option<f32>, level: f32) {
        self.trail.push_back(pitch.map(|midi_f| (midi_f, level)));
        while self.trail.len() > TRAIL_CAP {
            self.trail.pop_front();
        }
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

        // The written line arrives DECIDED. Note starts, ends, the glitch/release
        // grace and the cents average are all settled in `audio::dsp::segmenter`, at
        // the resonator bank's cadence on a sample clock. This panel used to run that
        // state machine itself off `ui.input(|i| i.time)`, which made a dropped frame
        // able to commit a note early and made every note's duration a count of
        // renderer ticks. Reading it is all that is left to do.
        let note_line = reading.as_ref().map(|r| r.note_line.clone()).unwrap_or_default();

        // The trail's own pitch: the melody line, already silence-gated and octave-
        // decided upstream (`audio::dsp::melody`). `level` only sets its intensity.
        let pitch = reading
            .as_ref()
            .and_then(|r| r.melody_pitch)
            .map(|(midi_f, _)| midi_f);
        self.staff.update(pitch, level);

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
                            match note_line.current {
                                Some(note) => {
                                    pill_colored(
                                        ui,
                                        &format!(
                                            "{}  {:+.0}\u{00A2}",
                                            style.midi_name(note.midi),
                                            note.cents
                                        ),
                                        PILL_INK_ON_INTONATION,
                                        intonation_color(note.cents),
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
                draw_staff(
                    &painter,
                    rect,
                    &self.staff,
                    &note_line,
                    style,
                    res_wf,
                    res_min,
                    res_max,
                );
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
/// `line` is the engine's decided note line — history + the note being held. `res_wf`
/// is the resonator bank's magnitude history (newest last), `res_min/max_midi` the
/// bank's MIDI range — together they drive the fast pitch-energy waterfall drawn on
/// the lines behind the notes.
#[allow(clippy::too_many_arguments)]
fn draw_staff(
    painter: &Painter,
    rect: Rect,
    trainer: &StaffTrainer,
    line: &NoteLine,
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
    let mut items: Vec<(i32, f32, bool)> = line.history.iter().map(|n| (n.midi, n.cents, false)).collect();
    if let Some(note) = line.current {
        items.push((note.midi, note.cents, true));
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
    if let Some(note) = line.current {
        draw_intonation_bar(painter, rect, note.cents);
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
