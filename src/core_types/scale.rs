use std::collections::HashSet;

use itertools::Itertools;

use crate::core_types::pitch::{Interval, PCNote};

use super::pitch::PNote;

#[derive(Debug, Clone)]
pub struct Scale {
    pub root: PCNote,             // корневая нота
    pub intervals: Vec<Interval>, // интервалы в полутонах от корня

    pcs_set: Vec<PCNote>, // для быстрого contains
}

impl Scale {
    pub fn new(root: PCNote, intervals: &[u8]) -> Self {
        let intervals = intervals.iter().map(|s| Interval(*s as i32)).collect_vec();

        let pcs_set = intervals
            .iter()
            .map(|interval| root.add(interval))
            .collect();

        Self {
            root,
            intervals: intervals,
            pcs_set,
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

    pub fn is_root(&self, note: PCNote) -> bool {
        self.root == note
    }

    pub fn contains(&self, pc_note: PCNote) -> bool {
        self.pcs_set.contains(&pc_note)
    }

    pub fn degree(&self, note: PCNote) -> Option<Degree> {
        self.pcs_set
            .iter()
            .position(|&my_note| note == my_note)
            .map(|idx| Degree(idx as u8 + 1)) // ступени обычно нумеруются с 1
    }
}

// ступень лада
pub struct Degree(pub u8);
