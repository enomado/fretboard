use itertools::Itertools;

use crate::core_types::{
    note::ANote,
    parse::parse_notes,
    pitch::{Interval, PCNote, PNote},
};

#[derive(Debug, Clone)]
pub struct Tuning {
    v: Vec<PNote>,
}

impl Tuning {
    // E2–A2–D3–G3–B3–E4 .
    pub fn standart() -> Self {
        let (_, mut v) = parse_notes("E2–A2–D3–G3–B3–E4").unwrap();

        v.reverse();
        let v = v.iter().map(|s| s.to_pitch()).collect_vec();

        Self { v }
    }

    pub fn minor_thirds(mut from: PNote) -> Self {
        let mut v = vec![];

        v.push(from);

        // начинаем с последней
        for s in 1..6 {
            from = from.add(Interval(3));
            v.push(from);
        }

        v.reverse();

        Self { v }
    }

    pub fn note(&self, index: GString) -> PNote {
        self.v.get(index.0 - 1).unwrap().clone()
    }

    pub fn string_count(&self) -> usize {
        self.v.len()
    }
}

/// from 1
#[derive(Debug, Clone, Copy)]
pub struct GString(pub usize);

/// from 1
#[derive(Debug, Clone, Copy)]
pub struct Fret(pub usize);
impl Fret {
    pub fn semitones(&self) -> Interval {
        Interval(self.0 as i32)
    }
}

impl GString {
    pub fn name(&self) -> String {
        self.0.to_string()
    }
}
