// чтобы не было циркулярных зависимостей

use crate::core_types::note::ANote;
use crate::core_types::parse::parse_anote;

impl ANote {
    pub fn parse(input: &str) -> Self {
        let (_, note) = parse_anote(input).unwrap();
        note
    }
}
