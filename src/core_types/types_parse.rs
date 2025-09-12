// чтобы не было циркулярных зависимостей

use crate::core_types::{parse::parse_anote, pitch::ANote};

impl ANote {
    pub fn parse(input: &str) -> Self {
        let (p, note) = parse_anote(input).unwrap();
        note
    }
}
