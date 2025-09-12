use itertools::Itertools;

use crate::types::{ANote, Interval, PCNote};

#[derive(Debug, Clone)]
pub struct Scale {
    pub root: PCNote,             // корневая нота
    pub intervals: Vec<Interval>, // интервалы в полутонах от корня
}

impl Scale {
    pub fn new(root: PCNote, intervals: &[u8]) -> Self {
        Self {
            root,
            intervals: intervals.iter().map(|s| Interval(*s as i32)).collect_vec(),
        }
    }

    pub fn notes(&self) -> Vec<PCNote> {
        let root_val = &self.root;
        self.intervals.iter().map(|i| root_val.add(i)).collect()
    }

    pub fn major(root: PCNote) -> Self {
        Self::new(root, &[0, 2, 4, 5, 7, 9, 11])
    }

    pub fn minor(root: PCNote) -> Self {
        Self::new(root, &[0, 2, 3, 5, 7, 8, 10])
    }

    pub fn dorian(root: PCNote) -> Self {
        Self::new(root, &[0, 2, 3, 5, 7, 9, 10])
    }

    pub fn phrygian(root: PCNote) -> Self {
        Self::new(root, &[0, 1, 3, 5, 7, 8, 10])
    }

    pub fn lydian(root: PCNote) -> Self {
        Self::new(root, &[0, 2, 4, 6, 7, 9, 11])
    }

    pub fn mixolydian(root: PCNote) -> Self {
        Self::new(root, &[0, 2, 4, 5, 7, 9, 10])
    }

    pub fn locrian(root: PCNote) -> Self {
        Self::new(root, &[0, 1, 3, 5, 6, 8, 10])
    }
}
