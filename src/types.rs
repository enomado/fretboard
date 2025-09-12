use std::fmt::format;

#[derive(Debug, Clone, Copy)]
pub enum Ass {
    Flat,
    Sharp,
    Natural,
}

impl Ass {
    pub fn name(&self) -> &'static str {
        match self {
            Ass::Flat => "b",
            Ass::Sharp => "#",
            Ass::Natural => "",
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
    pub ass: Ass,
    pub octave: Octave,
}

/// относительная нота. без октавы
#[derive(Debug, Clone)]
pub struct PCNote(pub u8);

impl PCNote {
    fn pitch_class(note: Note, ass: Ass) -> PCNote {
        let f = match (note, ass) {
            (Note::C, Ass::Natural) => 0,
            (Note::C, Ass::Sharp) | (Note::D, Ass::Flat) => 1,
            (Note::D, Ass::Natural) => 2,
            (Note::D, Ass::Sharp) | (Note::E, Ass::Flat) => 3,
            (Note::E, Ass::Natural) => 4,
            (Note::F, Ass::Natural) => 5,
            (Note::F, Ass::Sharp) | (Note::G, Ass::Flat) => 6,
            (Note::G, Ass::Natural) => 7,
            (Note::G, Ass::Sharp) | (Note::A, Ass::Flat) => 8,
            (Note::A, Ass::Natural) => 9,
            (Note::A, Ass::Sharp) | (Note::B, Ass::Flat) => 10,
            (Note::B, Ass::Natural) => 11,
            // Ass::Natural on enharmonic weird cases covered above
            _ => panic!("Unsupported accidental combination"),
        };

        PCNote(f)
    }

    pub fn to_note(&self) -> (Note, Ass) {
        let pc = self.0;

        let (note, ass) = match pc {
            0 => (Note::C, Ass::Natural),
            1 => (Note::C, Ass::Sharp),
            2 => (Note::D, Ass::Natural),
            3 => (Note::D, Ass::Sharp),
            4 => (Note::E, Ass::Natural),
            5 => (Note::F, Ass::Natural),
            6 => (Note::F, Ass::Sharp),
            7 => (Note::G, Ass::Natural),
            8 => (Note::G, Ass::Sharp),
            9 => (Note::A, Ass::Natural),
            10 => (Note::A, Ass::Sharp),
            11 => (Note::B, Ass::Natural),
            _ => unreachable!(),
        };

        (note, ass)
    }

    pub fn from_note(note: Note, ass: Ass) -> Self {
        Self::pitch_class(note, ass)
    }

    pub fn add(&self, i: &Interval) -> PCNote {
        let brr = (self.0 as i32 + i.0) % 12;
        PCNote(brr as u8)
    }
}

impl ANote {
    pub fn to_midi(&self) -> PNote {
        let note = self.octave.0 as i32 * 12 + self.simple().0 as i32;
        PNote::new(note as u8).unwrap()
    }

    pub fn from_midi(midi: PNote) -> ANote {
        let (octave, note) = midi.to_note();
        let (note, ass) = note.to_note();

        ANote {
            note,
            ass,
            octave: octave,
        }
    }

    pub fn add_interval(&self, semitones: Interval) -> ANote {
        let midi = self.to_midi();
        ANote::from_midi(midi.add(semitones))
    }

    pub fn new(n: Note, octave: u8) -> Self {
        Self {
            note: n,
            ass: Ass::Natural,
            octave: Octave(octave),
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

    pub fn to_note(&self) -> (Octave, PCNote) {
        let octave = (self.0 / 12) as u8;
        let pc = (self.0 % 12 + 12) % 12; // нормализация
        (Octave(octave), PCNote(pc))
    }
}

#[derive(Debug, Clone)]
pub struct Interval(pub i32);
