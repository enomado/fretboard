use std::fmt::format;

use crate::core_types::note::{ANote, Accidental, Note, Octave};

/// относительная нота. без октавы
#[derive(Debug, Clone)]
pub struct PCNote(pub u8);

impl PCNote {
    fn pitch_class(note: Note, ass: Accidental) -> PCNote {
        let f = match (note, ass) {
            (Note::C, Accidental::Natural) => 0,
            (Note::C, Accidental::Sharp) | (Note::D, Accidental::Flat) => 1,
            (Note::D, Accidental::Natural) => 2,
            (Note::D, Accidental::Sharp) | (Note::E, Accidental::Flat) => 3,
            (Note::E, Accidental::Natural) => 4,
            (Note::F, Accidental::Natural) => 5,
            (Note::F, Accidental::Sharp) | (Note::G, Accidental::Flat) => 6,
            (Note::G, Accidental::Natural) => 7,
            (Note::G, Accidental::Sharp) | (Note::A, Accidental::Flat) => 8,
            (Note::A, Accidental::Natural) => 9,
            (Note::A, Accidental::Sharp) | (Note::B, Accidental::Flat) => 10,
            (Note::B, Accidental::Natural) => 11,
            // Ass::Natural on enharmonic weird cases covered above
            _ => panic!("Unsupported accidental combination"),
        };

        PCNote(f)
    }

    pub fn to_note(&self) -> (Note, Accidental) {
        let pc = self.0;

        let (note, ass) = match pc {
            0 => (Note::C, Accidental::Natural),
            1 => (Note::C, Accidental::Sharp),
            2 => (Note::D, Accidental::Natural),
            3 => (Note::D, Accidental::Sharp),
            4 => (Note::E, Accidental::Natural),
            5 => (Note::F, Accidental::Natural),
            6 => (Note::F, Accidental::Sharp),
            7 => (Note::G, Accidental::Natural),
            8 => (Note::G, Accidental::Sharp),
            9 => (Note::A, Accidental::Natural),
            10 => (Note::A, Accidental::Sharp),
            11 => (Note::B, Accidental::Natural),
            _ => unreachable!(),
        };

        (note, ass)
    }

    pub fn from_note(note: Note, ass: Accidental) -> Self {
        Self::pitch_class(note, ass)
    }

    pub fn add(&self, i: &Interval) -> PCNote {
        let brr = (self.0 as i32 + i.0) % 12;
        PCNote(brr as u8)
    }
}

// Pitch.  абсолютная нота, с октавой
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PNote(u8);

impl PNote {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 127;

    pub fn new(v: u8) -> Option<Self> {
        if (Self::MIN..=Self::MAX).contains(&v) {
            Some(Self(v))
        } else {
            None
        }
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }

    /// Прибавить n полутонов к текущей ноте.
    /// Если выходит за диапазон 0..=127, обрезаем к границе.
    pub fn add(&self, semitones: Interval) -> PNote {
        let value = self.0 as i32 + semitones.0;
        let clamped = value.clamp(0, 127) as u8;
        PNote(clamped)
    }

    pub fn to_pc(&self) -> (Octave, PCNote) {
        let octave = (self.0 / 12) as u8;
        let pc = (self.0 % 12 + 12) % 12; // нормализация
        (Octave(octave), PCNote(pc))
    }

    pub fn to_anote(&self) -> ANote {
        ANote::from_pitch(&self)
    }
}

#[derive(Debug, Clone)]
pub struct Interval(pub i32);
