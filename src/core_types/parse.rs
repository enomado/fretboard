use nom::branch::alt;
use nom::bytes::complete::{
    tag,
    take_while1,
};
use nom::character::complete::{
    char,
    one_of,
};
use nom::combinator::{
    map,
    opt,
};
use nom::multi::separated_list1;
use nom::{
    IResult,
    Parser,
};

use crate::core_types::note::{
    ANote,
    Accidental,
    Note,
    Octave,
};

/// Парсинг одной буквы A–G
fn parse_note(input: &str) -> IResult<&str, Note> {
    map(one_of("ABCDEFG"), |c| {
        match c {
            'A' => Note::A,
            'B' => Note::B,
            'C' => Note::C,
            'D' => Note::D,
            'E' => Note::E,
            'F' => Note::F,
            'G' => Note::G,
            _ => unreachable!(),
        }
    })
    .parse(input)
}

/// Парсинг диез/бемоль/натурал
fn parse_ass(input: &str) -> IResult<&str, Accidental> {
    map(opt(alt((char('#'), char('b')))), |opt_c| {
        match opt_c {
            Some('#') => Accidental::Sharp,
            Some('b') => Accidental::Flat,
            None => Accidental::Natural,
            _ => unreachable!(),
        }
    })
    .parse(input)
}

/// Парсинг октавы (цифра или несколько)
fn parse_octave(input: &str) -> IResult<&str, Octave> {
    map(take_while1(|c: char| c.is_ascii_digit()), |s: &str| {
        Octave(s.parse::<u8>().unwrap())
    })
    .parse(input)
}

/// Парсинг одной ноты целиком
pub fn parse_anote(input: &str) -> IResult<&str, ANote> {
    map((parse_note, parse_ass, parse_octave), |(note, ass, octave)| {
        ANote { note, ass, octave }
    })
    .parse(input)
}

/// Парсинг списка нот, разделённых длинным тире
pub fn parse_notes(input: &str) -> IResult<&str, Vec<ANote>> {
    separated_list1(tag("–"), parse_anote).parse(input)
}

#[test]
fn brr() {
    let input = "E2–A2–D3–G3–B3–E4";
    let (_, notes) = parse_notes(input).unwrap();
    println!("{:?}", notes);
}
