use crate::{
    parse::parse_notes,
    types::{ANote, Interval},
};

#[derive(Debug, Clone)]
pub struct Tuning {
    v: Vec<ANote>,
}

impl Tuning {
    // E2–A2–D3–G3–B3–E4 .
    pub fn standart() -> Self {
        let (_, mut v) = parse_notes("E2–A2–D3–G3–B3–E4").unwrap();

        v.reverse();

        Self { v }
    }

    pub fn minor_thirds(mut from: ANote) -> Self {
        let mut v = vec![];

        v.push(from);

        // начинаем с последней
        for s in 1..6 {
            from = from.add_interval(Interval(3));
            v.push(from);
        }

        v.reverse();

        Self { v }
    }

    pub fn note(&self, index: GString) -> ANote {
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
