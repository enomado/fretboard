use crate::core_types::pitch::{Interval, PCNote};

#[derive(Debug, Clone)]
pub struct Chord {
    pub root: PCNote, // корневая нота
    pub intervals: Vec<Interval>, // интервалы от корня

                      // pub name: Option<String>,     // например "Cmaj7", "Dm"
}

impl Chord {
    /// Создать новый аккорд
    pub fn new(root: PCNote, intervals: Vec<Interval>) -> Self {
        Self { root, intervals }
    }

    /// Проверяет, содержит ли аккорд данную ноту
    pub fn degree(&self, note: PCNote) -> Option<usize> {
        self.intervals
            .iter()
            .position(|interval| self.root.add(interval) == note)
            .map(|idx| idx + 1) // ступени нумеруем с 1
    }

    /// Возвращает все ноты аккорда как PCNote
    pub fn notes(&self) -> Vec<PCNote> {
        self.intervals
            .iter()
            .map(|interval| self.root.add(interval))
            .collect()
    }
}

impl Chord {
    pub fn major(root: PCNote) -> Self {
        Self::new(root, vec![Interval(0), Interval(4), Interval(7)])
    }

    pub fn minor(root: PCNote) -> Self {
        Self::new(root, vec![Interval(0), Interval(3), Interval(7)])
    }

    pub fn dominant7(root: PCNote) -> Self {
        Self::new(
            root,
            vec![Interval(0), Interval(4), Interval(7), Interval(10)],
        )
    }

    pub fn major7(root: PCNote) -> Self {
        Self::new(
            root,
            vec![Interval(0), Interval(4), Interval(7), Interval(11)],
        )
    }

    pub fn minor7(root: PCNote) -> Self {
        Self::new(
            root,
            vec![Interval(0), Interval(3), Interval(7), Interval(10)],
        )
    }

    pub fn half_diminished7(root: PCNote) -> Self {
        Self::new(
            root,
            vec![Interval(0), Interval(3), Interval(6), Interval(10)],
        )
    }

    pub fn diminished7(root: PCNote) -> Self {
        Self::new(
            root,
            vec![Interval(0), Interval(3), Interval(6), Interval(9)],
        )
    }
}
