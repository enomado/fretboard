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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Octave(pub u8);

impl Octave {
    pub fn name(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
