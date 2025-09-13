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
    pub fn standart_e() -> Self {
        let (_, mut v) = parse_notes("E2–A2–D3–G3–B3–E4").unwrap();

        v.reverse();
        let v = v.iter().map(|s| s.to_pitch()).collect_vec();

        Self { v }
    }

    pub fn from_cum(root: PNote, intervals: &[u8]) -> Self {
        let intervals = intervals.iter().map(|s| Interval(*s as i32)).collect_vec();
        let mut notes = intervals.iter().map(|i| root.add(*i)).collect_vec();
        notes.reverse();

        Self { v: notes }
    }

    pub fn from_rel(root: PNote, intervals: &[u8]) -> Self {
        let mut notes = vec![root];
        let intervals = intervals.iter().map(|s| Interval(*s as i32)).collect_vec();

        let mut cum = root;
        for i in intervals {
            cum = cum.add(i);
            notes.push(cum);
        }

        notes.reverse();

        Self { v: notes }
    }

    pub fn standard_from(root: PNote) -> Self {
        // интервалы между струнами в полутонах: E-A-D-G-B-E
        // между 6–5: 5, 5–4: 5, 4–3: 5, 3–2: 4, 2–1: 5
        let intervals = [0, 5, 10, 15, 19, 24]; // кумулятивные интервалы от низкой струны
        Self::from_cum(root, &intervals)
    }

    pub fn cello() -> Self {
        let root = ANote::parse("C2").to_pitch();

        let intervals = [0, 7, 14, 21];
        Self::from_cum(root, &intervals)
    }

    pub fn minor_thirds(root: PNote) -> Self {
        Self::from_rel(root, &[3, 3, 3, 3, 3])
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
