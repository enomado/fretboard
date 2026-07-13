use crate::core_types::pitch::{
    Interval,
    PCNote,
    PNote,
};

/// types for convinience

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accidental {
    Natural,
    Flat,
    Sharp,
}

impl Accidental {
    pub fn name(&self) -> &'static str {
        match self {
            Accidental::Flat => "b",
            Accidental::Sharp => "#",
            Accidental::Natural => "",
        }
    }

    /// The proper music glyph for the accidental (♯/♭/♮), for display.
    pub fn glyph(&self) -> &'static str {
        match self {
            Accidental::Flat => "\u{266D}",    // ♭
            Accidental::Sharp => "\u{266F}",   // ♯
            Accidental::Natural => "\u{266E}", // ♮
        }
    }
}

/// How black-key pitch classes are spelled when we render a note name: as
/// sharps (C#, D#, …) or as flats (Db, Eb, …). This is a single global display
/// preference threaded through every note-label producer (the snails, the note
/// bars/waterfalls, the resonator bank, the tuner readout, the fretboard, the
/// scale-finder wheels, the drone) so the whole app agrees on one spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AccidentalStyle {
    Sharps,
    Flats,
}

impl Default for AccidentalStyle {
    // Sharps: the spelling the analysis labels (note bars, resonator, tuner)
    // already used before the toggle existed, so old configs read unchanged.
    fn default() -> Self {
        AccidentalStyle::Sharps
    }
}

impl AccidentalStyle {
    /// The name of pitch class `pc` (0 = C, wrapping mod 12) in this spelling.
    pub fn pitch_class_name(self, pc: usize) -> &'static str {
        const SHARPS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        const FLATS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
        let table = match self {
            AccidentalStyle::Sharps => &SHARPS,
            AccidentalStyle::Flats => &FLATS,
        };
        table[pc % 12]
    }

    /// A full name with octave for a MIDI note, e.g. `"C#4"` / `"Db4"`.
    pub fn midi_name(self, midi: i32) -> String {
        let pc = midi.rem_euclid(12) as usize;
        let octave = midi / 12 - 1;
        format!("{}{}", self.pitch_class_name(pc), octave)
    }
}

/// A key signature: the run of sharps or flats sitting right after the clef that
/// applies for the whole staff. Modelled by its position on the circle of fifths
/// — `fifths` counts perfect fifths from C major: positive = that many sharps
/// (G = +1, D = +2, …), negative = that many flats (F = −1, B♭ = −2, …), 0 = C
/// major (no accidentals). The magnitude directly *is* the number of accidentals
/// drawn, and their identity/order is fixed by convention (see the order tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySignature {
    /// Circle-of-fifths distance from C, in `-7..=7`.
    pub fifths: i32,
}

impl Default for KeySignature {
    /// C major — no sharps, no flats.
    fn default() -> Self {
        KeySignature { fifths: 0 }
    }
}

impl KeySignature {
    /// Natural-letter indices (0 = C … 6 = B) that take a sharp, in the order
    /// sharps are added to a signature: F C G D A E B.
    const SHARP_ORDER: [usize; 7] = [3, 0, 4, 1, 5, 2, 6];
    /// Letters that take a flat, in order: B E A D G C F (the sharp order reversed).
    const FLAT_ORDER: [usize; 7] = [6, 2, 5, 1, 4, 0, 3];

    /// Number of accidentals in the signature (always ≥ 0).
    pub fn count(self) -> usize {
        self.fifths.unsigned_abs() as usize
    }

    /// Whether the signature's accidentals are sharps or flats. Meaningless for C
    /// major (returns `Sharp` by convention); callers gate on `count() == 0` first.
    pub fn accidental(self) -> Accidental {
        if self.fifths < 0 {
            Accidental::Flat
        } else {
            Accidental::Sharp
        }
    }

    /// The spelling style the key implies: flat keys spell the black notes as
    /// flats, sharp keys as sharps. `None` for C major, where there are no
    /// accidentals to spell and the caller keeps its own global preference.
    pub fn style(self) -> Option<AccidentalStyle> {
        match self.fifths {
            0 => None,
            n if n < 0 => Some(AccidentalStyle::Flats),
            _ => Some(AccidentalStyle::Sharps),
        }
    }

    /// The key-signature alteration of a natural letter (0 = C … 6 = B):
    /// `Sharp`/`Flat` if that letter is in the signature, else `Natural`.
    pub fn alteration(self, letter: usize) -> Accidental {
        let order = if self.fifths < 0 {
            Self::FLAT_ORDER
        } else {
            Self::SHARP_ORDER
        };
        // Only the first `count` letters of the order are altered.
        if order[..self.count().min(7)].contains(&letter) {
            self.accidental()
        } else {
            Accidental::Natural
        }
    }

    /// The accidental glyph to draw *at the notehead* for a note spelled with
    /// `letter` and sounding-accidental `acc`, under this signature. `None` when
    /// the signature already implies it (in-key notes carry no glyph). `Some` is
    /// returned when the note deviates from what the signature says for its letter
    /// — `Some(Natural)` is a ♮ cancelling a signature accidental, `Some(Sharp)` /
    /// `Some(Flat)` marks a chromatic note outside the key.
    pub fn note_glyph(self, letter: usize, acc: Accidental) -> Option<Accidental> {
        (acc != self.alteration(letter)).then_some(acc)
    }

    /// The signature's accidentals as `(letter, accidental)` in draw order.
    pub fn accidentals(self) -> impl Iterator<Item = (usize, Accidental)> {
        let order = if self.fifths < 0 {
            Self::FLAT_ORDER
        } else {
            Self::SHARP_ORDER
        };
        let acc = self.accidental();
        (0..self.count().min(7)).map(move |i| (order[i], acc))
    }
}

/// Major keys around the circle of fifths (flat side → sharp side) with their
/// display names, for the key picker. Enharmonic spellings at the rim (C♭/B,
/// G♭/F♯, D♭/C♯) are given by whichever needs fewer accidentals here.
pub const CIRCLE_OF_FIFTHS: [(i32, &str); 15] = [
    (-7, "C\u{266D}"),
    (-6, "G\u{266D}"),
    (-5, "D\u{266D}"),
    (-4, "A\u{266D}"),
    (-3, "E\u{266D}"),
    (-2, "B\u{266D}"),
    (-1, "F"),
    (0, "C"),
    (1, "G"),
    (2, "D"),
    (3, "A"),
    (4, "E"),
    (5, "B"),
    (6, "F\u{266F}"),
    (7, "C\u{266F}"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Octave(pub u8);

impl Octave {
    pub fn name(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Note {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
}

impl Note {
    pub fn name(&self) -> &'static str {
        match self {
            Note::A => "A",
            Note::B => "B",
            Note::C => "C",
            Note::D => "D",
            Note::E => "E",
            Note::F => "F",
            Note::G => "G",
        }
    }

    pub fn to_pc(self) -> PCNote {
        PCNote::from_natural(self)
    }
}

/// absolute
#[derive(Debug, Clone, Copy)]
pub struct ANote {
    pub note:   Note,
    pub ass:    Accidental,
    pub octave: Octave,
}

impl ANote {
    pub fn to_pitch(&self) -> PNote {
        let note = (self.octave.0 as i32 + 1) * 12 + self.simple().0 as i32;
        PNote::new(note as u8).unwrap()
    }

    pub fn from_pitch(pitch: &PNote) -> ANote {
        let (octave, note) = pitch.to_pc();
        let (note, ass) = note.to_note();

        ANote { note, ass, octave }
    }

    pub fn add_interval(&self, semitones: Interval) -> ANote {
        let pitch = self.to_pitch();
        ANote::from_pitch(&pitch.add(semitones))
    }

    pub fn new(n: Note, octave: Octave) -> Self {
        Self {
            note: n,
            ass: Accidental::Natural,
            octave,
        }
    }

    pub fn name(&self) -> String {
        let n = self.note.name();
        let a = self.ass.name();
        let o = self.octave.name();

        format!("{}{}{}", n, a, o)
    }

    /// Name re-spelled in the given accidental style, computed from the note's
    /// pitch (ignoring however `self.ass` happened to be spelled). Used by the
    /// display surfaces that honour the global sharps/flats toggle.
    pub fn name_styled(&self, style: AccidentalStyle) -> String {
        style.midi_name(self.to_pitch().as_u8() as i32)
    }

    fn simple(&self) -> PCNote {
        PCNote::from_note(self.note, self.ass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::pitch::Interval;

    #[test]
    fn low_octave_notes_convert_to_expected_pitches() {
        assert_eq!(ANote::parse("C0").to_pitch().as_u8(), 12);
        assert_eq!(ANote::parse("A0").to_pitch().as_u8(), 21);
        assert_eq!(ANote::parse("B0").to_pitch().as_u8(), 23);
        assert_eq!(ANote::parse("C1").to_pitch().as_u8(), 24);
    }

    #[test]
    fn low_octave_pitches_round_trip_to_note_names() {
        for note in ["C0", "C#0", "A0", "B0", "C1"] {
            let pitch = ANote::parse(note).to_pitch();

            assert_eq!(pitch.to_anote().name(), note);
        }
    }

    #[test]
    fn adding_interval_crosses_from_b0_to_c1() {
        let note = ANote::parse("B0").add_interval(Interval(1));

        assert_eq!(note.name(), "C1");
    }

    // letter indices used throughout the key-signature tests: C=0 D=1 E=2 F=3
    // G=4 A=5 B=6.
    #[test]
    fn key_signature_counts_and_style() {
        assert_eq!(KeySignature { fifths: 0 }.count(), 0);
        assert_eq!(KeySignature { fifths: 0 }.style(), None); // C major defers to global
        assert_eq!(KeySignature { fifths: 2 }.count(), 2); // D major: 2 sharps
        assert_eq!(KeySignature { fifths: 2 }.style(), Some(AccidentalStyle::Sharps));
        assert_eq!(KeySignature { fifths: -3 }.count(), 3); // E♭ major: 3 flats
        assert_eq!(KeySignature { fifths: -3 }.style(), Some(AccidentalStyle::Flats));
    }

    #[test]
    fn key_signature_alters_the_right_letters() {
        // G major = 1 sharp on F.
        let g = KeySignature { fifths: 1 };
        assert_eq!(g.alteration(3), Accidental::Sharp); // F
        assert_eq!(g.alteration(0), Accidental::Natural); // C untouched
        // D major = F♯ and C♯.
        let d = KeySignature { fifths: 2 };
        assert_eq!(d.alteration(3), Accidental::Sharp); // F
        assert_eq!(d.alteration(0), Accidental::Sharp); // C
        assert_eq!(d.alteration(4), Accidental::Natural); // G untouched
        // F major = 1 flat on B.
        let f = KeySignature { fifths: -1 };
        assert_eq!(f.alteration(6), Accidental::Flat); // B
        assert_eq!(f.alteration(2), Accidental::Natural); // E untouched
        // The accidental list is in the canonical order and length.
        assert_eq!(
            d.accidentals().collect::<Vec<_>>(),
            vec![(3, Accidental::Sharp), (0, Accidental::Sharp),]
        );
    }

    #[test]
    fn note_glyph_absorbs_in_key_and_cancels_out_of_key() {
        let g = KeySignature { fifths: 1 }; // G major, F♯
        // F♯ is in the key → no glyph at the note (the signature implies it).
        assert_eq!(g.note_glyph(3, Accidental::Sharp), None);
        // F natural contradicts the signature → a ♮ is drawn.
        assert_eq!(g.note_glyph(3, Accidental::Natural), Some(Accidental::Natural));
        // A chromatic G♯ (letter G, not in the key) → its ♯ is drawn.
        assert_eq!(g.note_glyph(4, Accidental::Sharp), Some(Accidental::Sharp));
        // C major: every natural is bare, every alteration shows its own sign.
        let c = KeySignature { fifths: 0 };
        assert_eq!(c.note_glyph(3, Accidental::Natural), None);
        assert_eq!(c.note_glyph(3, Accidental::Sharp), Some(Accidental::Sharp));
    }
}
