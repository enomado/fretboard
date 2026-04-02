#[cfg(test)]
mod tests {
    use super::core_types::chord::Chord;
    use super::core_types::note::Note;
    use super::core_types::pitch::{Accidental, PCNote};

    #[test]
    fn test_major_chord_notes() {
        let root = PCNote::from_note(Note::C, Accidental::Natural);
        let chord = Chord::major(root);
        let notes = chord.notes();
        assert_eq!(notes.len(), 3);
        // C, E, G
        assert_eq!(notes[0], PCNote::from_note(Note::C, Accidental::Natural));
        assert_eq!(notes[1], PCNote::from_note(Note::E, Accidental::Natural));
        assert_eq!(notes[2], PCNote::from_note(Note::G, Accidental::Natural));
    }

    #[test]
    fn test_chord_degree() {
        let root = PCNote::from_note(Note::C, Accidental::Natural);
        let chord = Chord::major(root);
        assert_eq!(chord.degree(PCNote::from_note(Note::C, Accidental::Natural)), Some(1));
        assert_eq!(chord.degree(PCNote::from_note(Note::E, Accidental::Natural)), Some(3));
        assert_eq!(chord.degree(PCNote::from_note(Note::G, Accidental::Natural)), Some(5));
        assert_eq!(chord.degree(PCNote::from_note(Note::D, Accidental::Natural)), None);
    }

    #[test]
    fn test_seventh_chord_degree() {
        let root = PCNote::from_note(Note::C, Accidental::Natural);
        let chord = Chord::dominant7(root);
        assert_eq!(chord.degree(PCNote::from_note(Note::B, Accidental::Flat)), Some(7));
    }
}
