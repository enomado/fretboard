use crate::core_types::pitch::{Interval, PCNote, PNote};

/// types for convinience

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub struct Octave(pub u8);

impl Octave {
    pub fn name(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug, Clone, Copy)]
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
}

/// absolute
#[derive(Debug, Clone, Copy)]
pub struct ANote {
    pub note: Note,
    pub ass: Accidental,
    pub octave: Octave,
}

impl ANote {
    pub fn to_pitch(&self) -> PNote {
        let note = self.octave.0 as i32 * 12 + self.simple().0 as i32;
        PNote::new(note as u8).unwrap()
    }

    pub fn from_pitch(pitch: PNote) -> ANote {
        let (octave, note) = pitch.to_note();
        let (note, ass) = note.to_note();

        ANote {
            note,
            ass,
            octave: octave,
        }
    }

    pub fn add_interval(&self, semitones: Interval) -> ANote {
        let pitch = self.to_pitch();
        ANote::from_pitch(pitch.add(semitones))
    }

    pub fn new(n: Note, octave: Octave) -> Self {
        Self {
            note: n,
            ass: Accidental::Natural,
            octave: octave,
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
