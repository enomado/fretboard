use crate::core_types::note::{
    ANote,
    Accidental,
    Note,
    Octave,
};

/// pitch class
/// относительная нота. без октавы
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]

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

    pub fn from_natural(note: Note) -> Self {
        Self::pitch_class(note, Accidental::Natural)
    }

    pub fn add(&self, i: &Interval) -> PCNote {
        let brr = (self.0 as i32 + i.0) % 12;
        PCNote(brr as u8)
    }
}

// Pitch.  абсолютная нота, с октавой
//
// Сериализуется как голый `u8` (`try_from`/`into`), а на десериализации
// проходит через `PNote::new` — выход за 0..=127 = ошибка парсинга, а не
// тихо-битый инвариант. Для fail-soft RON-загрузки это означает откат к
// дефолтам, что строго лучше протащенного мусорного питча.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct PNote(u8);

impl From<PNote> for u8 {
    fn from(note: PNote) -> u8 {
        note.0
    }
}

impl TryFrom<u8> for PNote {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        PNote::new(value).ok_or("MIDI pitch out of range 0..=127")
    }
}

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
        let octave = (self.0 / 12).saturating_sub(1);
        let pc = (self.0 % 12 + 12) % 12; // нормализация
        (Octave(octave), PCNote(pc))
    }

    pub fn to_anote(&self) -> ANote {
        ANote::from_pitch(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Interval(pub i32);

/// Частота, Гц.
///
/// `serde(transparent)` — на диске (RON настроек) и в worker-протоколе (postcard) это
/// тот же голый `f32`, что был до появления типа.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Hz(pub f32);

impl Hz {
    /// ISO 16: A4 = 440 Гц. Эталон по умолчанию и опора там, где MIDI — внутренняя
    /// координата, а не высота для человека (сетка pYIN, корпус бенчмарка).
    pub const A4_STANDARD: Hz = Hz(440.0);

    /// Дробная MIDI-высота этой частоты при камертоне `a4`:
    /// `m = 69 + 12·log2(f / a4)` — 69 = A4, каждые 12 единиц = октава (×2 по частоте).
    ///
    /// Единственная копия формулы в репозитории; обратная — [`Midi::to_hz`]. Частота
    /// ≤ 0 даёт `-inf`/NaN — отсеивать тишину вызывающему, как и раньше.
    pub fn to_midi(self, a4: Hz) -> Midi {
        Midi(69.0 + 12.0 * (self.0 / a4.0).log2())
    }
}

/// Дробная MIDI-высота: 69.0 = A4, 69.5 = A4 + 50¢. Целая нота — [`PNote`].
///
/// Шкала логарифмическая и зависит от камертона: одно и то же `Midi` звучит на разной
/// частоте при A4 = 440 и 442, поэтому конверсии в обе стороны принимают его явно.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Midi(pub f32);

impl Midi {
    /// Частота этой высоты при камертоне `a4`. Формулу держит крейт `resonators` (им же
    /// строится банк), здесь её не повторяем.
    pub fn to_hz(self, a4: Hz) -> Hz {
        Hz(resonators::midi_to_hz(self.0, a4.0))
    }

    /// Ближайшая целая нота и отклонение от неё в центах (−50..=50): как написать
    /// сыгранную высоту и насколько мимо равномерной темперации она сыграна.
    ///
    /// Предусловие: высота уже внутри 0..=127 (её дал детектор, у которого сетка
    /// кончается на C8) — снаружи и на NaN паника, а не тихо прижатая нота.
    pub fn nearest_note(self) -> (PNote, f32) {
        let nearest = self.0.round();
        let cents = (self.0 - nearest) * 100.0;
        assert!(
            (PNote::MIN as f32..=PNote::MAX as f32).contains(&nearest),
            "pitch {self:?} is outside MIDI 0..=127"
        );
        (PNote::new(nearest as u8).unwrap(), cents)
    }
}

impl From<PNote> for Midi {
    fn from(note: PNote) -> Midi {
        Midi(note.0 as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Hz,
        Midi,
        PNote,
    };

    /// Круговой путь на 442 Гц, а не на 440: при 440 перепутанная опора (жёсткое 440
    /// внутри одной из конверсий) давала бы тот же ответ и тест бы её не видел.
    #[test]
    fn midi_round_trip_is_identity_at_a442() {
        let a4 = Hz(442.0);
        assert_eq!(a4.to_midi(a4), Midi(69.0));
        for m in [21.0f32, 55.0, 69.0, 69.5, 88.25, 108.0] {
            let back = Midi(m).to_hz(a4).to_midi(a4);
            assert!((back.0 - m).abs() < 1e-4, "{m} → {back:?}");
        }
        // Опора действительно участвует: A4 при 442 — это не 69 при 440.
        let at_440 = a4.to_midi(Hz::A4_STANDARD);
        assert!((at_440.0 - 69.0786).abs() < 1e-3, "{at_440:?}");
    }

    #[test]
    fn integer_note_is_its_own_midi() {
        assert_eq!(Midi::from(PNote::new(60).unwrap()), Midi(60.0));
    }

    /// Центы — со знаком: высота выше ноты даёт `+`, ниже — `−`, а сама нота — ближайшая,
    /// а не нижняя (69.6 — это A#4 на −40¢, а не A4 на +60¢).
    #[test]
    fn nearest_note_rounds_and_keeps_the_sign_of_cents() {
        let (note, cents) = Midi(69.25).nearest_note();
        assert_eq!(note, PNote::new(69).unwrap());
        assert!((cents - 25.0).abs() < 1e-3, "{cents}");
        let (note, cents) = Midi(69.6).nearest_note();
        assert_eq!(note, PNote::new(70).unwrap());
        assert!((cents + 40.0).abs() < 1e-3, "{cents}");
    }

    #[test]
    #[should_panic(expected = "outside MIDI")]
    fn nearest_note_refuses_nan() {
        let _ = Midi(f32::NAN).nearest_note();
    }
}
