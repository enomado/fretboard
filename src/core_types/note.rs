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
        const SHARPS: [&str; 12] =
            ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        const FLATS: [&str; 12] =
            ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
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
}
