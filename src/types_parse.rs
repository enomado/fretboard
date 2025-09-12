// чтобы не было циркулярных зависимостей
use crate::{parse::parse_anote, types::ANote};

impl ANote {
    pub fn parse(input: &str) -> Self {
        let (p, note) = parse_anote(input).unwrap();
        note
    }
}
