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
//! panel's own: which clef, which key signature. The stateless staff/clef drawing
//! lives in [`crate::ui::staff`].
//!
//! **The waterfall behind the notes is one waterfall.** Its two layers — the bank's
//! heat and the pitch trail — are two views of the same [`MelodyFrame`], placed by the
//! same `t` through the same [`TimeRuler`]. They used to be two buffers on two clocks
//! (the trail per UI frame, the heat per bank column, each spread over the width by its
//! own count), which had them scrolling at ~200 px/s and ~375 px/s against each other —
//! reported live as "два водопада… один медленнее другой быстрее". A shared span is
//! not something the two layers agree on now; there is only one of it.
//!
//! **Two rulers are drawn here, deliberately.** The waterfall is placed by TIME
//! ([`TimeRuler`]); the noteheads are placed by COUNT, one column per note
//! ([`note_columns`]). So a written note and the heat that produced it sit at different
//! x, and drift further apart the longer the note is held. That is engraving, not a
//! bug — see [`note_columns`] for why, and do not "fix" it into one ruler.
//!
//! **The drawing is split by region, and each region is one function**, in the order
//! the ink lands: [`staff_geom`] → [`draw_engraving`] → [`Waterfall::draw`] →
//! [`draw_noteheads`] → [`draw_note_names`] → [`draw_intonation_bar`]. [`draw_staff`]
//! is the orchestrator and paints nothing itself.

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
use crate::audio::{
    MelodyFrame,
    MelodyHistory,
    NoteLine,
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

/// How far back the waterfall reaches, **in seconds of played audio**.
///
/// One span for one waterfall. Its two layers — the bank's heat and the pitch trail
/// drawn on top of it — used to measure the width in different units entirely: the
/// trail in 240 *UI frames* (≈4 s at 60 fps, ≈8 s at 30), the heat in 52 *bank
/// columns* (≈0.83 s) and then squeezed into a `clamp(2.0, 6.0)` step so it did not
/// even reach the left edge. Two layers of the same sound scrolling at ~200 px/s and
/// ~375 px/s: reported live as "два водопада… один медленнее другой быстрее".
///
/// They cannot disagree now — both are placed from the same [`MelodyFrame::t`], and
/// the frames they read are the same frames.
const WATERFALL_SECONDS: f64 = 4.0;

/// What the staff panel itself owns.
///
/// Note capture is **not** here: it is the engine's, on the engine's clock (see the
/// module docs). What is left is the reading surface — which clef, which key — plus
/// the waterfall's frames, which are the engine's too; the panel only holds them long
/// enough to draw them.
pub struct StaffTrainer {
    /// The bank frames behind the notes, oldest → newest, trimmed to
    /// [`WATERFALL_SECONDS`] of played audio.
    ///
    /// **Both** waterfall layers read this, which is the point: the heat and the trail
    /// are two views of one frame, so they are aligned 1:1 and travel at one speed by
    /// construction rather than by two constants agreeing. The trail used to be
    /// sampled per UI frame and the heat pulled from a second buffer entirely.
    frames: MelodyHistory,
    /// How far the panel has read the engine's history. Ours, not the engine's — the
    /// pitch roll holds its own and reads the same frames (`AudioEngine::melody_since`).
    cursor: Option<u64>,
    /// The clef the staff is drawn in — user-selectable (default treble). One
    /// staff at a time: notes never migrate between clefs.
    clef:   Clef,
    /// The key signature drawn after the clef (default C major = none). It fixes
    /// the sharps/flats at the clef and, in turn, how each note is spelled and
    /// whether it carries its own accidental (see [`KeySignature::note_glyph`]).
    key:    KeySignature,
}

impl Default for StaffTrainer {
    fn default() -> Self {
        Self {
            frames: MelodyHistory::with_retention(WATERFALL_SECONDS),
            cursor: None,
            clef:   Clef::default(),
            key:    KeySignature::default(),
        }
    }
}

impl StaffTrainer {
    /// Take the bank frames the engine has published since the last read.
    ///
    /// Empty, one, or several per repaint — all ordinary. The waterfall is paced by
    /// the audio; the repaint only decides when the frames are collected.
    pub fn update(&mut self, fresh: Vec<MelodyFrame>) {
        let Some(last_seq) = fresh.last().map(|f| f.seq) else {
            return;
        };
        self.cursor = Some(last_seq);
        // The ring trims by time and heals across a stream restart — its rules, not
        // the panel's. See `MelodyHistory::push`.
        for frame in fresh {
            self.frames.push(frame);
        }
    }

    /// The newest frame's timestamp: the playhead, in audio time.
    fn now(&self) -> Option<f64> {
        self.frames.newest().map(|f| f.t)
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

        // The written line arrives DECIDED. Note starts, ends, the glitch/release
        // grace and the cents average are all settled in `audio::dsp::segmenter`, at
        // the resonator bank's cadence on a sample clock. This panel used to run that
        // state machine itself off `ui.input(|i| i.time)`, which made a dropped frame
        // able to commit a note early and made every note's duration a count of
        // renderer ticks. Reading it is all that is left to do.
        let note_line = reading.as_ref().map(|r| r.note_line.clone()).unwrap_or_default();

        // The waterfall's frames, taken by cursor at the bank's cadence — the melody
        // line, its heat and the audio time of both, in one record. Not `reading()`
        // sampled per repaint: that is what made the trail a count of renderer ticks
        // while the heat next to it counted bank columns, and the two scroll at
        // different speeds the moment their rulers differ.
        let fresh = self.audio.melody_since(self.staff.cursor);
        self.staff.update(fresh);

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                draw_card_header(ui, &note_line, style);
                draw_pickers(ui, &mut self.staff);

                ui.add_space(10.0);

                let width = ui.available_width();
                let (rect, _resp) = ui.allocate_exact_size(vec2(width, 260.0), Sense::hover());
                let painter = ui.painter_at(rect);
                // The bank's pitch range: what maps a heat bin to a pitch. The heat
                // itself rides the frames now (`MelodyFrame::heat`) rather than a
                // second history off the reading — which is what let it drift from the
                // trail drawn on top of it.
                let bank = BankRange {
                    min_midi: settings.resonator.min_midi.as_u8() as i32,
                    max_midi: settings.resonator.max_midi.as_u8() as i32,
                };
                draw_staff(&painter, rect, &self.staff, &note_line, style, bank);
            });
    }
}

/// The card's title block, and — pinned to the right — the live-note chip: what is
/// sounding right now, painted on its own intonation colour.
fn draw_card_header(ui: &mut Ui, line: &NoteLine, style: AccidentalStyle) {
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
                match line.current {
                    Some(note) => {
                        pill_colored(
                            ui,
                            &format!("{}  {:+.0}\u{00A2}", style.midi_name(note.midi), note.cents),
                            PILL_INK_ON_INTONATION,
                            intonation_color(note.cents),
                        )
                    }
                    None => pill_muted(ui, "\u{2014}"),
                }
            },
        );
    });
}

/// The two things the panel itself owns: which clef the staff is read in, and which
/// key it is written in. Both write straight into [`StaffTrainer`] — everything else
/// on this card is the engine's and is only borrowed to draw.
///
/// Drawn *after* the frame's `style`/`key` are read, so a click here lands on the next
/// frame. The alternative — re-reading the key below the picker — would spell the
/// notes in one key while the signature at the clef still showed the old one.
fn draw_pickers(ui: &mut Ui, trainer: &mut StaffTrainer) {
    // Clef picker — one staff at a time (a violin reads treble, a cello/bass the
    // lower clefs). The choice lives in `StaffTrainer`.
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
                .add(SegmentedButton::new(clef.label(), trainer.clef == clef))
                .clicked()
            {
                trainer.clef = clef;
            }
        }

        ui.add_space(18.0);

        // Key-signature picker — the circle of fifths. Selecting a key both draws
        // its sharps/flats at the clef and re-spells the notes accordingly (see
        // `KeySignature`).
        ui.add(RowCaption::new("Key").font_size(12.0).color(color::TEXT_HINT));
        // The snug popup style these short rows need is baked into `PillCombo`, so
        // it is no longer spelled out here.
        PillCombo::new("staff_key_sig", key_label(trainer.key)).show(ui, |ui| {
            for &(fifths, name) in CIRCLE_OF_FIFTHS.iter() {
                let k = KeySignature { fifths };
                ui.selectable_value(&mut trainer.key, k, key_label(k))
                    .on_hover_text(format!("{name} major"));
            }
        });
    });
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
/// `line` is the engine's decided note line — history + the note being held; `bank` is
/// the resonator bank's MIDI span, which is what makes a heat bin mean a pitch.
///
/// **This function paints nothing.** It is the running order — each region below is one
/// function, and they are called back-to-front so the notation lands on top of the
/// waterfall. The only things it decides itself are the two anchors every region
/// measures from: `right_x` (the playhead) and the [`TimeRuler`].
fn draw_staff(
    painter: &Painter,
    rect: Rect,
    trainer: &StaffTrainer,
    line: &NoteLine,
    style: AccidentalStyle,
    bank: BankRange,
) {
    let mut geom = staff_geom(rect);
    draw_engraving(painter, &mut geom, trainer.clef, trainer.key);

    // The current note (and the waterfall's newest frame) live at this x; notes step
    // left from here, the waterfall flows into it from the left. The one x the two
    // rulers share — they agree on *now* and on nothing else.
    let right_x = geom.notes_right - geom.gap * 1.6;

    if let Some(now) = trainer.now() {
        Waterfall {
            geom: &geom,
            frames: &trainer.frames,
            now,
            ruler: TimeRuler::new(&geom, right_x),
            clef: trainer.clef,
            style,
            bank,
        }
        .draw(painter);
    }

    let columns = note_columns(line, right_x, geom.gap * 3.2);
    draw_noteheads(painter, &geom, &columns, style, trainer.clef, trainer.key);
    draw_note_names(painter, &geom, &columns, style, rect.top());

    // Intonation needle for the note currently sounding.
    if let Some(note) = line.current {
        draw_intonation_bar(painter, rect, note.cents);
    } else if columns.is_empty() {
        draw_empty_hint(painter, rect, geom.gap);
    }
}

/// Where the staff sits inside `rect` — before the key signature has its say.
///
/// `notes_left` is only a floor here: [`draw_engraving`] pushes it right past whatever
/// the signature engraves. Nothing may place a note from this geometry directly.
fn staff_geom(rect: Rect) -> StaffGeom {
    // Gap scales with height; the middle line sits at the vertical centre so there
    // is head-room for ledger lines both above and below.
    let gap = (rect.height() / 15.0).clamp(9.0, 20.0);
    let left = rect.left() + 12.0;
    StaffGeom {
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
    }
}

/// The engraved furniture: the five lines, the clef glyph, and the key signature
/// between them and the notes.
///
/// Takes `geom` by `&mut` because **the signature is part of the geometry**: however
/// many sharps or flats it engraves push `notes_left` right, so the noteheads and the
/// waterfall both start clear of it (a no-op in C major, which engraves nothing). That
/// feedback is the reason this is one function and not two — the region cannot be
/// measured before it is drawn.
fn draw_engraving(painter: &Painter, geom: &mut StaffGeom, clef: Clef, key: KeySignature) {
    staff::draw_staff_lines(painter, geom, STAFF_LINE);
    staff::draw_clef(painter, geom, clef, STAFF_INK);
    let ksig_right = staff::draw_key_signature(painter, geom, clef, key, STAFF_INK);
    geom.notes_left = geom.notes_left.max(ksig_right + geom.gap * 0.8);
}

/// One written note, placed. See [`note_columns`].
struct NoteColumn {
    /// Centre x of the notehead — and of the name above it.
    x:         f32,
    midi:      i32,
    cents:     f32,
    /// The note sounding right now: drawn larger, as the one being played.
    emphasize: bool,
}

/// Place the written notes right→left: the newest at `right_x`, each older one one
/// `advance` further toward the clef, so a new note enters at the right and the line
/// scrolls off past `notes_left`.
///
/// **This is the notation's own ruler, and it is deliberately not the waterfall's.**
/// A column is one *note* wide however long that note was held, so a written note and
/// the heat that produced it ([`TimeRuler`], seconds) sit at different x, drifting
/// further apart the longer the note. That is what engraving is — a whole and an eighth
/// take the same width on paper — and the staff is a reading surface, not a time plot.
/// Confirmed as the intended design (2026-07-15) when the split below was cut: placing
/// the heads at `ruler.x_of(t)` instead would make this a piano roll, which is what the
/// pitch-roll panel already is. Two rulers, on purpose.
///
/// Columns scrolled off the left edge are **kept, not dropped**: they are invisible
/// (each drawing pass skips them) but they still mean "something has been played", and
/// [`draw_staff`] reads that to decide the empty-state hint.
fn note_columns(line: &NoteLine, right_x: f32, advance: f32) -> Vec<NoteColumn> {
    let mut items: Vec<(i32, f32, bool)> = line.history.iter().map(|n| (n.midi, n.cents, false)).collect();
    if let Some(note) = line.current {
        items.push((note.midi, note.cents, true));
    }
    let n = items.len();
    items
        .iter()
        .enumerate()
        .map(|(i, &(midi, cents, emphasize))| {
            NoteColumn {
                // Newest is last, and it is the one that sits at `right_x`.
                x: right_x - (n - 1 - i) as f32 * advance,
                midi,
                cents,
                emphasize,
            }
        })
        .collect()
}

/// The written line itself: a notehead per column, coloured by its own intonation,
/// with the accidentals and ledger lines `staff::draw_note` decides from the key.
fn draw_noteheads(
    painter: &Painter,
    geom: &StaffGeom,
    columns: &[NoteColumn],
    style: AccidentalStyle,
    clef: Clef,
    key: KeySignature,
) {
    for col in columns {
        if col.x < geom.notes_left {
            continue; // scrolled off past the clef
        }
        staff::draw_note(
            painter,
            geom,
            col.x,
            col.midi,
            style,
            clef,
            key,
            intonation_color(col.cents),
            STAFF_LINE,
            col.emphasize,
        );
    }
}

/// Name *every* note above the staff: a header row of note letters (C, D, F#, …), each
/// aligned to its note's column, so the written line reads back as named pitches.
///
/// The label is the pitch-class name only (no octave — "abcdef"), coloured to match the
/// notehead's intonation; the note sounding right now is drawn a touch larger for
/// emphasis. A fixed row at `top_y` — rather than following each notehead's varying
/// height — keeps the letters on one clean readable line.
fn draw_note_names(
    painter: &Painter,
    geom: &StaffGeom,
    columns: &[NoteColumn],
    style: AccidentalStyle,
    top_y: f32,
) {
    for col in columns {
        if col.x < geom.notes_left {
            continue;
        }
        let name = style.pitch_class_name(col.midi.rem_euclid(12) as usize);
        let name_size = if col.emphasize {
            geom.gap * 1.3
        } else {
            geom.gap * 1.05
        };
        painter.text(
            pos2(col.x, top_y + geom.gap * 0.5),
            Align2::CENTER_TOP,
            name,
            FontId::proportional(name_size),
            intonation_color(col.cents),
        );
    }
}

/// Nothing has been written yet — the staff is drawn, but empty. Shown only when the
/// line is *completely* empty (not merely scrolled off), and never while a note sounds.
fn draw_empty_hint(painter: &Painter, rect: Rect, gap: f32) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "play a note…",
        FontId::proportional(gap * 1.1),
        color::TEXT_MUTED,
    );
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

/// The waterfall's one ruler: how old a frame is, in audio seconds, → where it is drawn.
///
/// **It is a type so that it can only be one.** "The heat and the trail scroll together"
/// used to be a property two constants had to maintain, and they did not: the trail
/// measured its span in 240 UI frames (~200 px/s), the heat in 52 bank columns squeezed
/// by a `clamp(2.0, 6.0)` (~375 px/s) — reported live as "два водопада… один медленнее
/// другой быстрее". Both layers now take this value and place a frame through nothing
/// else, so there is no second number left to disagree with.
///
/// Not to be confused with the notation's ruler, which counts notes — see
/// [`note_columns`]. Those two *are* meant to differ.
#[derive(Clone, Copy)]
struct TimeRuler {
    /// x of age zero: the playhead, where the newest frame lands.
    right_x:       f32,
    /// The scroll speed, and the width of one second of audio.
    px_per_second: f32,
}

impl TimeRuler {
    /// Spread [`WATERFALL_SECONDS`] of audio across the note region, ending at
    /// `right_x` — so the oldest frame the history keeps lands on `notes_left`.
    fn new(geom: &StaffGeom, right_x: f32) -> Self {
        Self {
            right_x,
            px_per_second: (right_x - geom.notes_left) / WATERFALL_SECONDS as f32,
        }
    }

    /// Where a frame `age_s` seconds old is drawn.
    fn x_of(&self, age_s: f32) -> f32 {
        self.right_x - age_s * self.px_per_second
    }
}

/// The resonator bank's pitch span, as the panel reads it: what a heat bin's *index*
/// means. A newtype, so the two ends cannot be quietly swapped at a call site — the
/// whole bin→pitch mapping is derived from their difference.
#[derive(Clone, Copy)]
struct BankRange {
    min_midi: i32,
    max_midi: i32,
}

impl BankRange {
    /// How many semitones the bank spans. Zero or negative = a degenerate range and
    /// nothing to map; the heat layer draws nothing rather than dividing by it.
    fn semitones(&self) -> i32 {
        self.max_midi - self.min_midi
    }
}

/// One waterfall — the frame context both of its layers draw from.
///
/// Everything that places a frame lives here and is therefore *shared by construction*:
/// the same `frames`, the same `now`, the same [`TimeRuler`]. The layers below are
/// methods taking `&self` and nothing else, so neither can reach for a second buffer or
/// invent its own span — which is exactly how the two of them ended up on two clocks
/// (see [`TimeRuler`]). Built only when there is a playhead to measure from.
struct Waterfall<'a> {
    geom:   &'a StaffGeom,
    /// Both layers' single source: melody pitch, heat and the audio time of both.
    frames: &'a MelodyHistory,
    /// The playhead in audio time — the newest frame's `t`.
    now:    f64,
    ruler:  TimeRuler,
    clef:   Clef,
    style:  AccidentalStyle,
    bank:   BankRange,
}

impl Waterfall<'_> {
    /// Heat first, trail on top: the bank's energy lights a note up long before the
    /// windowed detector commits it, and the lag between the two layers is readable
    /// precisely because they line up.
    fn draw(&self, painter: &Painter) {
        self.draw_heat(painter);
        self.draw_trail(painter);
    }

    /// The live pitch trail: each recent frame's continuous pitch as a small fading
    /// dot at its exact staff height, newest at the playhead flowing left. Reads as a
    /// soft "waterfall" of what the ear/detector hears, right on the staff lines —
    /// the continuous counterpart to the quantised noteheads on top.
    fn draw_trail(&self, painter: &Painter) {
        let radius = (self.geom.gap * 0.17).max(1.2);
        for frame in self.frames.iter() {
            let Some(midi_f) = frame.pitch else {
                continue; // silence, or a rejected slip → a gap in the trail
            };
            let age_s = (self.now - frame.t) as f32;
            let x = self.ruler.x_of(age_s);
            if x < self.geom.notes_left {
                continue;
            }
            let y = staff::midi_to_y(self.geom, self.style, self.clef, midi_f);
            // Fade with age, intensify with level. Age is in *seconds* now, so the fade
            // is a real half-life rather than "how much of the buffer ago" — at 30 fps
            // the old trail faded over twice as much time as at 60.
            let recency = 1.0 - (age_s / WATERFALL_SECONDS as f32).clamp(0.0, 1.0);
            let alpha = (frame.level.clamp(0.0, 1.0).sqrt() * recency * 165.0).clamp(0.0, 165.0) as u8;
            painter.circle_filled(
                pos2(x, y),
                radius,
                TRAIL_BLUE.gamma_multiply(alpha as f32 / 255.0),
            );
        }
    }

    /// Fast pitch-energy heat from the resonator bank, painted on the staff lines
    /// behind everything else. One column per frame (newest at the playhead, older to
    /// the left); within a column each bin above [`RES_WF_GATE`] becomes a small
    /// ~gap-sized rect at *its own* pitch height, so energy lands exactly on the
    /// line/space of the note it belongs to.
    ///
    /// This is the low-latency counterpart to the YIN noteheads: the bank publishes at
    /// ~60 Hz with a per-sample response, so a new note lights up here well before the
    /// windowed detector commits it — the whole point of "fighting the latency".
    ///
    /// Bin→pitch: the bank spans [`BankRange`]; the row length tells us the
    /// bins-per-semitone (it changes with the reassignment toggle, so we derive it
    /// rather than hard-code it). `midi_to_y` handles the diatonic (non uniform)
    /// vertical mapping, the same one the noteheads use.
    fn draw_heat(&self, painter: &Painter) {
        let n = self.frames.len();
        let bin_count = self
            .frames
            .iter()
            .map(|f| f.heat.as_slice())
            .find(|c| !c.is_empty())
            .map_or(0, <[f32]>::len);
        if n < 2 || bin_count < 2 || self.bank.semitones() <= 0 {
            return;
        }
        // Derived, not assumed: (bins − 1) spread over the semitone span.
        let bins_per_semitone = (bin_count - 1) as f32 / self.bank.semitones() as f32;

        // A cell is one *frame*, so it is as wide as the gap between frames — measured
        // rather than assumed (the bank's cadence is a user setting, 8..80 ms, and the
        // publish jitters around it). It used to be `clamp(2.0, 6.0)`, which is what
        // squeezed ~0.8 s of heat into ~300 px and set it scrolling at nearly twice the
        // trail's speed; the step is not a look to be dialled in, it is what the clock
        // says.
        let mean_gap_s = ((self.now - self.frames.oldest().unwrap().t) / (n - 1) as f64) as f32;
        let cell_w = (mean_gap_s * self.ruler.px_per_second + 0.6).clamp(1.0, 5.0);
        let cell_h = (self.geom.gap * 0.5).max(3.0);
        let clip = painter.clip_rect();

        for frame in self.frames.iter() {
            let age_s = (self.now - frame.t) as f32;
            let x = self.ruler.x_of(age_s);
            if x < self.geom.notes_left {
                continue;
            }
            // Fades over seconds, exactly like the trail above it — one clock, one fade.
            let recency = 1.0 - (age_s / WATERFALL_SECONDS as f32).clamp(0.0, 1.0);
            for (bin, &value) in frame.heat.iter().enumerate() {
                if value < RES_WF_GATE {
                    continue;
                }
                let midi = self.bank.min_midi as f32 + bin as f32 / bins_per_semitone;
                let y = staff::midi_to_y(self.geom, self.style, self.clef, midi);
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
}

/// Below this normalised magnitude a resonator bin is treated as noise and not
/// painted — keeps the waterfall sparse (a note + its partials, not a wash) and
/// the shape count low.
const RES_WF_GATE: f32 = 0.18;

#[cfg(test)]
mod tests {
    use eframe::egui::pos2;

    use super::*;
    use crate::audio::StaffNote;

    fn note(midi: i32) -> StaffNote {
        StaffNote { midi, cents: 0.0 }
    }

    /// A card-sized panel rect, the shape the staff is actually drawn in.
    fn geom() -> StaffGeom {
        staff_geom(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 260.0)))
    }

    /// **The design decision, pinned.** The written line is placed by COUNT: one
    /// column per note, evenly spaced, newest at the playhead — never by how long a
    /// note was held. That is why a written note and its own heat sit at different x
    /// (the waterfall runs on [`TimeRuler`]), and it is deliberate: notation is a
    /// reading surface, not a time plot — a whole and an eighth take one width on
    /// paper. Confirmed by the user 2026-07-15.
    ///
    /// If you are here because you are making the heads follow time, you are turning
    /// the staff into the pitch-roll panel. Change the design on purpose, or not at
    /// all — do not let this drift.
    #[test]
    fn the_written_line_is_placed_by_count_not_by_time() {
        let line = NoteLine {
            history: vec![note(64), note(66), note(67)],
            current: None,
        };
        let cols = note_columns(&line, 700.0, 32.0);

        let xs: Vec<f32> = cols.iter().map(|c| c.x).collect();
        // Newest last and at the playhead; each older note exactly one advance further
        // left, whatever its duration was — `StaffNote` does not even carry a time.
        assert_eq!(xs, vec![636.0, 668.0, 700.0]);
    }

    /// The note sounding right now is the one at the playhead, and the only one drawn
    /// emphasised — it is "being written" and has not joined the history yet.
    #[test]
    fn the_current_note_is_emphasised_at_the_playhead() {
        let line = NoteLine {
            history: vec![note(64)],
            current: Some(note(69)),
        };
        let cols = note_columns(&line, 700.0, 32.0);

        assert_eq!(cols.len(), 2);
        assert!(!cols[0].emphasize);
        assert_eq!(cols[0].x, 668.0);
        assert!(cols[1].emphasize);
        assert_eq!(cols[1].midi, 69);
        assert_eq!(cols[1].x, 700.0);
    }

    /// Notes that have scrolled off past the clef are **kept** as columns, invisible
    /// but present. Dropping them here would silently bring back "play a note…" over a
    /// staff that has been played on — the hint means *nothing written*, not *nothing
    /// on screen*.
    #[test]
    fn columns_scrolled_off_the_left_are_kept_not_dropped() {
        let g = geom();
        let line = NoteLine {
            history: (0..40).map(|_| note(64)).collect(),
            current: None,
        };
        let cols = note_columns(&line, 700.0, g.gap * 3.2);

        assert_eq!(cols.len(), 40);
        assert!(
            cols[0].x < g.notes_left,
            "40 notes should have scrolled the oldest off past the clef; \
             it must still count as written"
        );
    }

    /// The waterfall's span is **seconds of audio**, and the whole region is exactly
    /// [`WATERFALL_SECONDS`] wide: the playhead at age 0, the note region's left edge
    /// at the oldest frame the history keeps. This is the ruler that used to be two
    /// (~200 px/s against ~375 px/s — the reported parallax).
    #[test]
    fn the_waterfall_ruler_measures_the_region_in_audio_seconds() {
        let g = geom();
        let right_x = g.notes_right - g.gap * 1.6;
        let ruler = TimeRuler::new(&g, right_x);

        assert_eq!(ruler.x_of(0.0), right_x);
        let oldest_x = ruler.x_of(WATERFALL_SECONDS as f32);
        assert!(
            (oldest_x - g.notes_left).abs() < 0.001,
            "the oldest frame kept should land on notes_left, got {oldest_x} vs {}",
            g.notes_left
        );
    }
}
