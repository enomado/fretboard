//! Music-staff renderer for the violin trainer panel.
//!
//! Like the other `ui::*` modules this is a *pure* renderer: it owns no state and
//! paints from borrowed inputs. It knows two things — the geometry of a five-line
//! staff under a chosen [`Clef`] (treble, bass or the tenor C-clef), and how to
//! place a MIDI pitch on it as a notehead with the right ledger lines and
//! accidental.
//!
//! ## Staff coordinates
//! We measure vertical position in **diatonic staff steps** (half a line-gap
//! each), *not* in semitones — that is what a written staff does. A note's
//! vertical slot is decided by its *letter* (C D E F G A B); sharps and flats
//! share the slot of their natural letter and carry an accidental glyph. Two
//! enharmonic spellings of the same pitch therefore sit on different lines, so
//! the spelling (`AccidentalStyle`) is an input to the geometry.
//!
//! Step 0 is the bottom staff line. *Which* pitch that is depends on the [`Clef`]
//! (E4 for treble, G2 for bass, D3 for the tenor C-clef). Each `+1` is one slot
//! up (line→space→line…); the five lines are steps 0 2 4 6 8 (E4 G4 B4 D5 F5 in
//! treble).

use eframe::egui::{
    Align2,
    Color32,
    Painter,
    Pos2,
    Shape,
    Stroke,
    pos2,
};

use crate::core_types::note::{
    Accidental,
    AccidentalStyle,
    KeySignature,
};

/// Placement of the staff inside a panel rect. All fields are screen pixels.
pub struct StaffGeom {
    /// Distance between two adjacent staff lines.
    pub gap:         f32,
    /// Y of the bottom staff line (E4, step 0). Steps grow upward = smaller y.
    pub bottom_y:    f32,
    /// Left edge where the staff lines begin (the clef overhangs from here).
    pub staff_left:  f32,
    /// X of the clef's spiral centre (it curls around the G4 line).
    pub clef_x:      f32,
    /// Left edge of the region where noteheads may be drawn.
    pub notes_left:  f32,
    /// Right edge of the staff / note region.
    pub notes_right: f32,
}

impl StaffGeom {
    /// Y of a note sitting `step` diatonic slots above the bottom line (E4).
    /// A slot is half a line-gap, so lines land on even steps, spaces on odd.
    pub fn step_y(&self, step: i32) -> f32 {
        self.bottom_y - step as f32 * self.gap * 0.5
    }

    /// Y of staff line `i`, `0` = bottom line, `4` = top line.
    pub fn line_y(&self, i: usize) -> f32 {
        self.step_y(2 * i as i32)
    }
}

/// A clef fixes two things: which written pitch sits on the bottom staff line,
/// and which glyph is drawn. *Everything* else about the staff geometry (line
/// spacing, steps, ledger lines, stems) is identical across clefs — only this
/// anchor moves, so the whole renderer stays clef-agnostic once a `Clef` is
/// threaded through the pitch→step mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clef {
    /// Treble / G clef. Bottom line E4; the spiral eye curls around G4 (line 1).
    #[default]
    Treble,
    /// Bass / F clef. Bottom line G2; the two dots straddle F3 (line 3, the 4th
    /// line).
    Bass,
    /// Tenor C-clef («виолончельный»). Bottom line D3; the clef is centred on C4
    /// (line 3, the 4th line — the cello's upper register reads here).
    Tenor,
}

/// How to draw a clef's glyph: the music-font character, the staff line it is
/// registered to, and the vertical anchor tuning.
struct ClefGlyph {
    /// Music-font code point (Noto Music, Musical Symbols block).
    glyph:            &'static str,
    /// Staff step of the line the glyph registers to (its "hot spot" line).
    ref_step:         i32,
    /// egui draws the glyph with `Align2::CENTER_CENTER`, so we place the galley
    /// centre this fraction of the font size *above* the reference line. Equal to
    /// `(galley-centre-above-baseline) − (reference-feature-above-baseline)` in
    /// em. The galley constant is unknown/egui-specific, so instead of trusting a
    /// derived value we anchor to the hand-tuned treble offset (0.235) and shift
    /// by the *difference* of registered-feature heights, which cancels it:
    ///   `center_above_ref = 0.235 + (eye − feature)`
    /// with the em heights read straight from the Noto Music outlines — treble eye
    /// 0.251, bass dots 0.644, C-clef symmetric centre 0.501. (The implied galley
    /// centre, 0.486 em, matches the font's OS/2 winAscent/Descent midpoint.)
    center_above_ref: f32,
}

impl Clef {
    /// All clefs, in picker/display order (the default, treble, first).
    pub const ALL: [Clef; 3] = [Clef::Treble, Clef::Bass, Clef::Tenor];

    /// Short human label for the clef picker.
    pub fn label(self) -> &'static str {
        match self {
            Clef::Treble => "Treble",
            Clef::Bass => "Bass",
            Clef::Tenor => "Tenor",
        }
    }

    /// Diatonic index (letter + 7·octave, with C=0 … B=6) of the note on the
    /// bottom staff line (step 0). This single number re-anchors every pitch→step
    /// mapping to the clef; see [`note_staff_step`].
    fn bottom_line_diatonic(self) -> i32 {
        // letter values: C=0 D=1 E=2 F=3 G=4 A=5 B=6.
        match self {
            Clef::Treble => 2 + 7 * 4, // E4
            Clef::Bass => 4 + 7 * 2,   // G2
            Clef::Tenor => 1 + 7 * 3,  // D3 → puts C4 on the 4th line (step 6)
        }
    }

    fn glyph(self) -> ClefGlyph {
        match self {
            Clef::Treble => {
                ClefGlyph {
                    glyph:            "\u{1D11E}", // 𝄞 G clef
                    ref_step:         2,           // curls around the G4 line
                    center_above_ref: 0.235,
                }
            }
            Clef::Bass => {
                ClefGlyph {
                    glyph:            "\u{1D122}", // 𝄢 F clef
                    ref_step:         6,           // dots straddle the F3 line
                    center_above_ref: -0.158,
                }
            }
            Clef::Tenor => {
                ClefGlyph {
                    glyph:            "\u{1D121}", // 𝄡 C clef
                    ref_step:         6,           // centred on the C4 line
                    center_above_ref: -0.015,
                }
            }
        }
    }

    /// Where the key-signature accidentals sit on this clef's staff (see
    /// [`KeySigLayout`] and [`key_sig_steps`]).
    fn key_sig_layout(self) -> KeySigLayout {
        match self {
            // Treble: F♯ on the top line (F5 = step 8); B♭ on the middle line
            // (B4 = step 4). These are the standard engraved heights.
            Clef::Treble => {
                KeySigLayout {
                    sharp_first: 8,
                    sharp_floor: 3,
                    flat_first:  4,
                    flat_ceil:   7,
                }
            }
            // Bass: the whole pattern one staff line (two steps) lower than treble.
            Clef::Bass => {
                KeySigLayout {
                    sharp_first: 6,
                    sharp_floor: 1,
                    flat_first:  2,
                    flat_ceil:   5,
                }
            }
            // Tenor C-clef: derived to keep the glyphs on the staff with the
            // correct letters. (This is a clean derivation, not the rarely-used
            // canonical tenor-clef exception, which nudges a couple of glyphs by
            // an octave to further compress the run — a minor cosmetic difference.)
            Clef::Tenor => {
                KeySigLayout {
                    sharp_first: 9,
                    sharp_floor: 0,
                    flat_first:  5,
                    flat_ceil:   8,
                }
            }
        }
    }
}

/// Where a clef places a key signature's accidentals, as staff steps.
///
/// The accidentals march along the fifths cycle (F C G D A E B for sharps, its
/// reverse for flats). Vertically each one steps by a fourth from the previous —
/// *down* a fourth for sharps (`−3` steps), *up* a fourth for flats (`+3`) — but
/// folds back by a fifth in the opposite direction when it would run off the
/// staff, which is what produces the familiar zig-zag. `*_first` is the first
/// accidental's step; the threshold (`sharp_floor` / `flat_ceil`) is the step the
/// walk is not allowed to cross before folding.
struct KeySigLayout {
    sharp_first: i32,
    sharp_floor: i32,
    flat_first:  i32,
    flat_ceil:   i32,
}

/// Staff steps of a key signature's accidentals, in draw order, under `clef`.
/// Empty for C major. See [`KeySigLayout`] for the walk.
pub fn key_sig_steps(clef: Clef, key: KeySignature) -> Vec<i32> {
    let n = key.count().min(7);
    if n == 0 {
        return Vec::new();
    }
    let lay = clef.key_sig_layout();
    let sharps = key.fifths >= 0;
    let mut cur = if sharps { lay.sharp_first } else { lay.flat_first };
    let mut steps = Vec::with_capacity(n);
    steps.push(cur);
    for _ in 1..n {
        cur = if sharps {
            let c = cur - 3; // down a fourth
            if c < lay.sharp_floor { cur + 4 } else { c } // else fold up a fifth
        } else {
            let c = cur + 3; // up a fourth
            if c > lay.flat_ceil { cur - 4 } else { c } // else fold down a fifth
        };
        steps.push(cur);
    }
    steps
}

/// The letter (0=C … 6=B) and accidental a pitch class is spelled with, in the
/// given accidental style. This is what decides the vertical staff slot.
///
/// The tables mirror `AccidentalStyle::pitch_class_name` exactly, but keep the
/// letter and the accidental sign as separate data (that split is what the staff
/// needs; the string form throws it away).
fn spelling(pc: usize, style: AccidentalStyle) -> (i32, Accidental) {
    // letter values: C=0 D=1 E=2 F=3 G=4 A=5 B=6
    const SHARPS: [(i32, Accidental); 12] = [
        (0, Accidental::Natural), // C
        (0, Accidental::Sharp),   // C#
        (1, Accidental::Natural), // D
        (1, Accidental::Sharp),   // D#
        (2, Accidental::Natural), // E
        (3, Accidental::Natural), // F
        (3, Accidental::Sharp),   // F#
        (4, Accidental::Natural), // G
        (4, Accidental::Sharp),   // G#
        (5, Accidental::Natural), // A
        (5, Accidental::Sharp),   // A#
        (6, Accidental::Natural), // B
    ];
    const FLATS: [(i32, Accidental); 12] = [
        (0, Accidental::Natural), // C
        (1, Accidental::Flat),    // Db
        (1, Accidental::Natural), // D
        (2, Accidental::Flat),    // Eb
        (2, Accidental::Natural), // E
        (3, Accidental::Natural), // F
        (4, Accidental::Flat),    // Gb
        (4, Accidental::Natural), // G
        (5, Accidental::Flat),    // Ab
        (5, Accidental::Natural), // A
        (6, Accidental::Flat),    // Bb
        (6, Accidental::Natural), // B
    ];
    match style {
        AccidentalStyle::Sharps => SHARPS[pc % 12],
        AccidentalStyle::Flats => FLATS[pc % 12],
    }
}

/// Where a MIDI note lands on the staff under `clef`: `(step above the bottom
/// line, accidental)`.
///
/// `step` is in diatonic slots (half-gaps) counted from the clef's bottom line.
/// In treble E4=0, F5=8, middle C4=-2, open-G string G3=-5. Accidental is the
/// sign to draw left of the notehead.
pub fn note_staff_step(midi: i32, style: AccidentalStyle, clef: Clef) -> (i32, Accidental) {
    let pc = midi.rem_euclid(12) as usize;
    // Scientific octave: MIDI 60 = C4. `div_euclid` keeps this correct below 0.
    let octave = midi.div_euclid(12) - 1;
    let (letter, acc) = spelling(pc, style);
    // Diatonic index of this note vs. the diatonic index of the clef's bottom line.
    let diatonic = letter + 7 * octave;
    (diatonic - clef.bottom_line_diatonic(), acc)
}

/// The staff letter (0 = C … 6 = B) a MIDI note is spelled with in `style`. This
/// is the letter whose vertical slot the note occupies — what a key signature
/// keys its accidental off (see [`KeySignature::note_glyph`]).
pub fn note_letter(midi: i32, style: AccidentalStyle) -> usize {
    spelling(midi.rem_euclid(12) as usize, style).0 as usize
}

/// Even staff steps that need a ledger line for a note at `step`.
///
/// Ledger lines fill in the missing staff lines between the note and the staff:
/// below the bottom line they run at -2 (C4), -4 (A3), … down to the note; above
/// the top line at 10 (A5), 12 (C6), … up to the note. A note in a space just
/// past the edge (odd step) still only draws ledgers on the even slots it reaches.
pub fn ledger_steps(step: i32) -> Vec<i32> {
    let mut v = Vec::new();
    if step < 0 {
        let mut e = -2;
        while e >= step {
            v.push(e);
            e -= 2;
        }
    } else if step > 8 {
        let mut e = 10;
        while e <= step {
            v.push(e);
            e += 2;
        }
    }
    v
}

// ---------------------------------------------------------------------------
// egui drawing
// ---------------------------------------------------------------------------

/// Draw the five staff lines across the note region.
pub fn draw_staff_lines(painter: &Painter, geom: &StaffGeom, color: Color32) {
    let stroke = Stroke::new((geom.gap * 0.09).max(1.0), color);
    for i in 0..5 {
        let y = geom.line_y(i);
        painter.line_segment([pos2(geom.staff_left, y), pos2(geom.notes_right, y)], stroke);
    }
}

/// Font size (px) of the clef glyph, as a multiple of the line-gap. The same
/// size is used for every clef: Noto Music designs each clef glyph to sit
/// correctly on a common staff at a shared em, so one scale gives the right
/// relative proportions (the treble glyph's ink spans ~6.4 gaps at 3.7, the
/// shorter bass/C glyphs less, exactly as engraved on paper).
const CLEF_FONT_GAP: f32 = 3.7;

/// Draw a real clef glyph (Noto Music, Musical Symbols block) registered to its
/// staff line: the treble spiral around G4, the bass dots around F3, or the
/// tenor C-clef centred on C4.
pub fn draw_clef(painter: &Painter, geom: &StaffGeom, clef: Clef, color: Color32) {
    let fs = geom.gap * CLEF_FONT_GAP;
    let spec = clef.glyph();
    // Anchor the galley centre `center_above_ref * fs` above the reference line so
    // the glyph's registered feature lands on that line (see `ClefGlyph`).
    let ref_y = geom.step_y(spec.ref_step);
    let pos = pos2(geom.clef_x, ref_y - fs * spec.center_above_ref);
    painter.text(
        pos,
        Align2::CENTER_CENTER,
        spec.glyph,
        crate::ui::theme::music_font(fs),
        color,
    );
}

/// Draw the key signature — the run of sharps or flats — right after the clef,
/// each at its engraved staff step ([`key_sig_steps`]). Returns the x just past
/// the last accidental, where the note region may begin; for C major (no
/// accidentals) it returns `geom.notes_left` unchanged.
pub fn draw_key_signature(
    painter: &Painter,
    geom: &StaffGeom,
    clef: Clef,
    key: KeySignature,
    color: Color32,
) -> f32 {
    let steps = key_sig_steps(clef, key);
    if steps.is_empty() {
        return geom.notes_left;
    }
    let gap = geom.gap;
    let sign = key.accidental().glyph();
    let advance = gap * 1.15;
    // Start just clear of the clef glyph's ink.
    let x0 = geom.clef_x + gap * 2.4;
    for (i, &step) in steps.iter().enumerate() {
        let x = x0 + i as f32 * advance;
        painter.text(
            pos2(x, geom.step_y(step)),
            Align2::CENTER_CENTER,
            sign,
            crate::ui::theme::music_font(gap * 2.6),
            color,
        );
    }
    x0 + steps.len() as f32 * advance
}

/// Continuous vertical position of a fractional MIDI pitch on the staff — used
/// by the live pitch trail. Interpolates between the two nearest semitone slots
/// (the staff is diatonic, so the mapping is piecewise-linear, not uniform).
pub fn midi_to_y(geom: &StaffGeom, style: AccidentalStyle, clef: Clef, midi_f: f32) -> f32 {
    let m0 = midi_f.floor() as i32;
    let frac = midi_f - m0 as f32;
    let y0 = geom.step_y(note_staff_step(m0, style, clef).0);
    let y1 = geom.step_y(note_staff_step(m0 + 1, style, clef).0);
    y0 + (y1 - y0) * frac
}

/// Draw one note (ledger lines, notehead, stem, accidental) at horizontal
/// position `x`. Returns the notehead centre so the caller can attach labels.
///
/// `emphasize` draws it larger with a coloured ring — used for the note that is
/// currently sounding.
#[allow(clippy::too_many_arguments)]
pub fn draw_note(
    painter: &Painter,
    geom: &StaffGeom,
    x: f32,
    midi: i32,
    style: AccidentalStyle,
    clef: Clef,
    key: KeySignature,
    head_color: Color32,
    staff_color: Color32,
    emphasize: bool,
) -> Pos2 {
    let (step, acc) = note_staff_step(midi, style, clef);
    let y = geom.step_y(step);
    let gap = geom.gap;

    // Ledger lines through/around the note, matching the staff-line weight.
    // Defensive: only within the painter's rect (a stray out-of-range note must
    // never paint a runaway stack); with the panel's pitch gate this stays small.
    let ledger_stroke = Stroke::new((gap * 0.09).max(1.0), staff_color);
    let ledger_half = gap * 0.92;
    let clip = painter.clip_rect();
    for e in ledger_steps(step) {
        let ly = geom.step_y(e);
        if ly < clip.top() - gap || ly > clip.bottom() + gap {
            continue;
        }
        painter.line_segment(
            [pos2(x - ledger_half, ly), pos2(x + ledger_half, ly)],
            ledger_stroke,
        );
    }

    // Notehead: a filled ellipse, tilted like an engraved note.
    let scale = if emphasize { 1.15 } else { 1.0 };
    let rx = gap * 0.66 * scale;
    let ry = gap * 0.5 * scale;
    filled_ellipse(painter, pos2(x, y), rx, ry, -0.32, head_color);

    // Stem: quarter-note look. Notes below the middle line (B4=step 4) stem up
    // on the right; on/above it they stem down on the left.
    let stem_stroke = Stroke::new((gap * 0.13).max(1.2), head_color);
    let stem_len = gap * 3.1;
    if step < 4 {
        let sx = x + rx * 0.88;
        painter.line_segment([pos2(sx, y), pos2(sx, y - stem_len)], stem_stroke);
    } else {
        let sx = x - rx * 0.88;
        painter.line_segment([pos2(sx, y), pos2(sx, y + stem_len)], stem_stroke);
    }

    // Emphasis ring around the currently-sounding note.
    if emphasize {
        painter.circle_stroke(
            pos2(x, y),
            gap * 1.05,
            Stroke::new((gap * 0.11).max(1.2), head_color),
        );
    }

    // Accidental glyph to the left of the head. Under a key signature the note's
    // own accidental is drawn only when it deviates from the signature: in-key
    // notes carry nothing (the signature already implies them), a note contradicting
    // the signature gets a ♮, and a chromatic note outside the key gets its ♯/♭.
    let letter = note_letter(midi, style);
    if let Some(glyph_acc) = key.note_glyph(letter, acc) {
        painter.text(
            pos2(x - rx - gap * 0.5, y),
            Align2::CENTER_CENTER,
            glyph_acc.glyph(),
            crate::ui::theme::music_font(gap * 2.6),
            head_color,
        );
    }

    pos2(x, y)
}

/// Fill a tilted ellipse (egui has no ellipse primitive) as a convex polygon.
fn filled_ellipse(painter: &Painter, center: Pos2, rx: f32, ry: f32, angle: f32, color: Color32) {
    const N: usize = 28;
    let (sa, ca) = angle.sin_cos();
    let pts: Vec<Pos2> = (0..N)
        .map(|i| {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            let (ex, ey) = (rx * a.cos(), ry * a.sin());
            // rotate by `angle`
            pos2(center.x + ex * ca - ey * sa, center.y + ex * sa + ey * ca)
        })
        .collect();
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staff_steps_match_treble_clef() {
        let sharp = AccidentalStyle::Sharps;
        let g = Clef::Treble;
        // Bottom line E4, G4 line, top line F5.
        assert_eq!(note_staff_step(64, sharp, g).0, 0); // E4 bottom line
        assert_eq!(note_staff_step(67, sharp, g).0, 2); // G4 (line 1)
        assert_eq!(note_staff_step(71, sharp, g).0, 4); // B4 middle line
        assert_eq!(note_staff_step(77, sharp, g).0, 8); // F5 top line
        // Middle C and the open violin strings below the staff.
        assert_eq!(note_staff_step(60, sharp, g).0, -2); // C4 (1 ledger below)
        assert_eq!(note_staff_step(55, sharp, g).0, -5); // G3 (open G)
        assert_eq!(note_staff_step(69, sharp, g).0, 3); // A4
        assert_eq!(note_staff_step(76, sharp, g).0, 7); // E5 (open E is E5=76)
    }

    #[test]
    fn staff_steps_match_bass_and_tenor_clefs() {
        let sharp = AccidentalStyle::Sharps;
        // Bass (F clef): bottom line G2, F3 on line 3 (step 6), top line A3.
        assert_eq!(note_staff_step(43, sharp, Clef::Bass).0, 0); // G2 bottom line
        assert_eq!(note_staff_step(50, sharp, Clef::Bass).0, 4); // D3 middle line
        assert_eq!(note_staff_step(53, sharp, Clef::Bass).0, 6); // F3 (the F-clef line)
        assert_eq!(note_staff_step(57, sharp, Clef::Bass).0, 8); // A3 top line
        assert_eq!(note_staff_step(60, sharp, Clef::Bass).0, 10); // C4 (1 ledger above)
        // Tenor (C clef on the 4th line): bottom line D3, C4 on line 3 (step 6).
        assert_eq!(note_staff_step(50, sharp, Clef::Tenor).0, 0); // D3 bottom line
        assert_eq!(note_staff_step(57, sharp, Clef::Tenor).0, 4); // A3 middle line
        assert_eq!(note_staff_step(60, sharp, Clef::Tenor).0, 6); // C4 (the tenor line)
        assert_eq!(note_staff_step(64, sharp, Clef::Tenor).0, 8); // E4 top line
    }

    #[test]
    fn accidentals_follow_spelling() {
        let g = Clef::Treble;
        assert_eq!(
            note_staff_step(66, AccidentalStyle::Sharps, g).1,
            Accidental::Sharp
        ); // F#4
        // Same pitch, flat spelling → Gb: different letter slot, flat sign.
        let (sharp_step, _) = note_staff_step(66, AccidentalStyle::Sharps, g); // F#4 → F slot
        let (flat_step, flat_acc) = note_staff_step(66, AccidentalStyle::Flats, g); // Gb4 → G slot
        assert_eq!(flat_acc, Accidental::Flat);
        assert_eq!(flat_step, sharp_step + 1); // G sits one slot above F
    }

    #[test]
    fn key_signature_glyph_positions() {
        // Treble: the canonical engraved heights for a full seven-accidental run.
        // Sharps F C G D A E B → steps 8 5 9 6 3 7 4; flats B E A D G C F → 4 7 3 6 2 5 1.
        assert_eq!(
            key_sig_steps(Clef::Treble, KeySignature { fifths: 7 }),
            vec![8, 5, 9, 6, 3, 7, 4]
        );
        assert_eq!(
            key_sig_steps(Clef::Treble, KeySignature { fifths: -7 }),
            vec![4, 7, 3, 6, 2, 5, 1]
        );
        // Bass is the treble layout one line (two steps) lower.
        assert_eq!(
            key_sig_steps(Clef::Bass, KeySignature { fifths: 7 }),
            vec![6, 3, 7, 4, 1, 5, 2]
        );
        // A partial signature is just the first N of the run (D major = first 2 sharps).
        assert_eq!(
            key_sig_steps(Clef::Treble, KeySignature { fifths: 2 }),
            vec![8, 5]
        );
        // C major draws nothing.
        assert!(key_sig_steps(Clef::Treble, KeySignature { fifths: 0 }).is_empty());
    }

    #[test]
    fn key_signature_absorbs_note_accidentals() {
        // In G major the played F♯5 (MIDI 78) sits on the F slot with no glyph —
        // the signature already carries it; a played F natural (77) gets a ♮.
        let g = KeySignature { fifths: 1 };
        let sharps = AccidentalStyle::Sharps;
        assert_eq!(note_letter(78, sharps), 3); // F
        assert_eq!(g.note_glyph(note_letter(78, sharps), Accidental::Sharp), None);
        assert_eq!(
            g.note_glyph(note_letter(77, sharps), Accidental::Natural),
            Some(Accidental::Natural)
        );
    }

    #[test]
    fn ledger_lines_below_and_above() {
        assert_eq!(ledger_steps(-2), vec![-2]); // C4: one line below
        assert_eq!(ledger_steps(-5), vec![-2, -4]); // G3: hangs below 2nd ledger
        assert_eq!(ledger_steps(9), Vec::<i32>::new()); // G5: space above, no ledger
        assert_eq!(ledger_steps(10), vec![10]); // A5: one line above
        assert_eq!(ledger_steps(2), Vec::<i32>::new()); // on the staff: none
    }

    /// Dev aid: rasterise the staff + clef + a spread of notes to a PPM in the
    /// scratchpad so the vector clef can be eyeballed. Ignored by default.
    /// Run with: `cargo test -p fretboard render_staff_preview -- --ignored`
    #[test]
    #[ignore]
    fn render_staff_preview() {
        let (w, h) = (1000usize, 340usize);
        let mut img = vec![[24u8, 27, 31]; w * h];

        let gap = 18.0;
        let geom = StaffGeom {
            gap,
            bottom_y: 210.0,
            staff_left: 24.0,
            clef_x: 66.0,
            notes_left: 150.0,
            notes_right: w as f32 - 30.0,
        };
        let line_col = [120u8, 128, 140];

        // Staff lines.
        for i in 0..5 {
            let y = geom.line_y(i);
            draw_line_img(
                &mut img,
                w,
                h,
                (geom.staff_left, y),
                (geom.notes_right, y),
                1.4,
                line_col,
            );
        }
        // (Clef is a font glyph now — not rendered by this bitmap preview, which
        // only checks staff-line + notehead + ledger geometry.)
        // A spread of notes across the violin range to check placement + ledgers.
        let midis = [55, 60, 62, 64, 67, 69, 71, 74, 76, 79, 83, 88];
        for (k, &m) in midis.iter().enumerate() {
            let x = geom.notes_left + 40.0 + k as f32 * 66.0;
            let (step, _acc) = note_staff_step(m, AccidentalStyle::Sharps, Clef::Treble);
            let y = geom.step_y(step);
            for e in ledger_steps(step) {
                let ly = geom.step_y(e);
                draw_line_img(
                    &mut img,
                    w,
                    h,
                    (x - gap * 0.92, ly),
                    (x + gap * 0.92, ly),
                    1.4,
                    line_col,
                );
            }
            draw_ellipse_img(&mut img, w, h, (x, y), gap * 0.66, gap * 0.5, [232, 200, 120]);
        }

        write_ppm("staff_preview.ppm", w, h, &img);
    }

    // --- tiny software rasteriser for the preview test ---
    fn put(img: &mut [[u8; 3]], w: usize, h: usize, x: i32, y: i32, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            img[y as usize * w + x as usize] = c;
        }
    }
    fn draw_disk_img(img: &mut [[u8; 3]], w: usize, h: usize, c: (f32, f32), r: f32, col: [u8; 3]) {
        let r2 = r * r;
        let (cx, cy) = c;
        for dy in -(r as i32 + 1)..=(r as i32 + 1) {
            for dx in -(r as i32 + 1)..=(r as i32 + 1) {
                if (dx * dx + dy * dy) as f32 <= r2 {
                    put(img, w, h, cx as i32 + dx, cy as i32 + dy, col);
                }
            }
        }
    }
    fn draw_ellipse_img(
        img: &mut [[u8; 3]],
        w: usize,
        h: usize,
        c: (f32, f32),
        rx: f32,
        ry: f32,
        col: [u8; 3],
    ) {
        let (cx, cy) = c;
        for dy in -(ry as i32 + 1)..=(ry as i32 + 1) {
            for dx in -(rx as i32 + 1)..=(rx as i32 + 1) {
                let nx = dx as f32 / rx;
                let ny = dy as f32 / ry;
                if nx * nx + ny * ny <= 1.0 {
                    put(img, w, h, cx as i32 + dx, cy as i32 + dy, col);
                }
            }
        }
    }
    fn draw_line_img(
        img: &mut [[u8; 3]],
        w: usize,
        h: usize,
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
        col: [u8; 3],
    ) {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let steps = (len * 1.5) as usize + 1;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let x = a.0 + dx * t;
            let y = a.1 + dy * t;
            draw_disk_img(img, w, h, (x, y), width * 0.5, col);
        }
    }
    fn write_ppm(name: &str, w: usize, h: usize, img: &[[u8; 3]]) {
        use std::io::Write;
        let dir = std::env::var("STAFF_PREVIEW_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "P6\n{w} {h}\n255\n").unwrap();
        let mut bytes = Vec::with_capacity(w * h * 3);
        for px in img {
            bytes.extend_from_slice(px);
        }
        f.write_all(&bytes).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
