// чтобы не было циркулярных зависимостей

use crate::core_types::{note::ANote, parse::parse_anote};

impl ANote {
    pub fn parse(input: &str) -> Self {
        let (p, note) = parse_anote(input).unwrap();
        note
    }
}
