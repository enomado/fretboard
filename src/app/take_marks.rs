//! Note marks — the player's own truth about a take, and the file it lives in.
//!
//! ## The whole point: the pitch is *declared*, never read off the screen
//!
//! A human marking notes on top of a drawn pitch line **agrees with it**. Not through
//! laziness — through physics and fatigue. On the open G the second harmonic is ten
//! times brighter than the fundamental, so a marker reading "by eye" honestly writes
//! G4; and over two hundred notes of a trill nobody recalls each one independently,
//! they concur with the line wherever it looks plausible — which is exactly where the
//! detector is confidently wrong. A corpus built that way is a mirror of what the
//! detector already thinks, and it scores the detector high **precisely where we are
//! blind**. A mirror is not evidence.
//!
//! So the truth is split in two, and only one half is marked on screen:
//!
//! 1. **What** was played is declared *before playing* — the detector is not in the
//!    room. That is [`Declaration`], and the user was already writing it: it is the
//!    take's name (`g_flageolet_f5`).
//! 2. **When** it was played is marked afterwards, against the recording. The time
//!    axis is legitimate — the player is the authority on when a note started, and no
//!    amount of looking at a waterfall can talk them out of it.
//!
//! The pitch never passes through an eye looking at the detector's output. This is our
//! equivalent of what MDB-stem-synth gets for free by resynthesising its audio *from*
//! the ground truth: there, the truth cannot be argued with because the sound was made
//! from it.
//!
//! ## What is deliberately **not** modelled yet
//!
//! `main_app`'s `TimeMark` (`crates/apps/welllog/src/ui/time/time_mark.rs`) carries two
//! more axes — `MarkProvenance{Human, Machine}` and `MarkStatus{Approved, Pending,
//! Rejected}` — and they are right for welllog, where the question is "can this be
//! believed". Here there is no second source: this app cannot write the detector's
//! decisions down at all (that invariant is what makes a take replayable against a
//! *newer* detector), so every mark is human by construction. A `provenance` field with
//! one possible value would not enforce the invariant — it would decorate it.
//!
//! ⚠ When machine guesses do arrive, the trap is already known and written down:
//! `main_app` folds "human truth" and "approved guess" into one `Approved` status, and
//! for us that is poison — an approved guess is the detector nodded at, i.e. a mirror
//! with a stamp. `provenance` must then survive approve and never be rewritten to
//! `Human`, and the score must count only what a human declared.
//! See `memory/kickstart_recording_and_annotation.md`.
//!
//! ## What *is* modelled, and could not be recovered later
//!
//! [`MarkedAgainst`] — whether the detector's line was on screen when the mark was
//! made. That fact exists only at the moment of marking and is gone forever
//! afterwards, and it is the real contamination channel here (the user's stated
//! workflow is «размечаю поверх того что задетектилось»). Recording it costs one field
//! and keeps the question answerable: score the clean marks, score all of them, and
//! see whether looking at the line moved the number.

use std::fs::File;
use std::io::{
    BufRead,
    BufReader,
    Write,
};
use std::ops::Range;
use std::path::{
    Path,
    PathBuf,
};

use serde::{
    Deserialize,
    Serialize,
};

use crate::core_types::note::ANote;
use crate::core_types::parse::parse_anote;

/// What the player says they are about to play — the source of every mark's pitch.
///
/// Parsed from a note name ("F5", "G3", "A#4"), because that is how a violinist says
/// what they meant, and it is what the take is already named after.
///
/// Its own type, holding the *parsed* note rather than the text: a `String` here would
/// be re-parsed at every mark and could differ per mark within one take, which is the
/// opposite of a declaration. Once this exists, the note is settled.
#[derive(Clone, Copy, Debug)]
pub struct Declaration {
    note: ANote,
}

impl Declaration {
    /// Parse a declaration, or `None` if it is not a note name yet.
    ///
    /// `None` is not a crutch here: it is the ordinary state of a text field being
    /// typed into ("F" is not yet a note), and the caller shows it as "not armed"
    /// rather than papering over it.
    pub fn parse(text: &str) -> Option<Self> {
        // Reuse the crate's note parser (`ANote::parse` is its panicking twin — fine
        // for literals in code, not for a field a user is typing into).
        let (rest, note) = parse_anote(text.trim()).ok()?;
        // A trailing tail means the text is not *a note* — "F5x" must not quietly
        // declare F5. `parse_anote` is happy to stop early, so this is the check that
        // makes the declaration exact.
        rest.is_empty().then_some(Self { note })
    }

    /// The declared pitch as a MIDI number — what every mark made under it records.
    pub fn midi(self) -> u8 {
        self.note.to_pitch().as_u8()
    }

    pub fn name(self) -> String {
        self.note.name()
    }
}

/// Was the detector's line on screen when the mark was made?
///
/// See the module docs: this is the one fact about a mark that cannot be recovered
/// afterwards, and it is the real contamination channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MarkedAgainst {
    /// Only the spectral heat was on screen. The heat makes no pitch decision — it is
    /// the physical column the bank heard — so it can witness against the detector
    /// without agreeing with it. This is the default while marking, and the mark that
    /// counts.
    Evidence,
    /// The detector's line was on screen too. The mark's *timing* may be agreeing with
    /// it; the pitch still cannot, because the pitch came from the declaration.
    TheLineToo,
}

/// One note: what was played, and when.
///
/// The interval is the spine (as in `main_app`'s `TimeMark`) and everything else hangs
/// off it. `midi` is not "the pitch we saw" — it is the pitch the player declared
/// before the detector had an opinion.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NoteMark {
    /// When the note sounded, in **take seconds** — 0.0 is the take's first sample.
    ///
    /// Take seconds rather than wall time or sample index: it is the frame the player
    /// reasons in («слип на третьей секунде»), it is what the roll's ruler shows, and
    /// it survives the take being re-analysed by a newer detector at a different
    /// cadence. Always `start < end`; the constructor is what guarantees it.
    pub interval: Range<f64>,
    /// The declared pitch. See [`Declaration`].
    pub midi:     u8,
    /// See [`MarkedAgainst`].
    pub against:  MarkedAgainst,
}

impl NoteMark {
    /// A mark between two clicks, in either order.
    ///
    /// The order is normalised here rather than trusted from the gesture: a user is
    /// perfectly entitled to mark a note's end first and its start second, and an
    /// inverted `Range` would then be silently empty — the mark would vanish from every
    /// query without ever being rejected.
    pub fn new(a: f64, b: f64, declared: Declaration, against: MarkedAgainst) -> Self {
        Self {
            interval: a.min(b)..a.max(b),
            midi: declared.midi(),
            against,
        }
    }

    pub fn seconds(&self) -> f64 {
        self.interval.end - self.interval.start
    }
}

/// Where a take's marks live: `<take>.marks.jsonl`, beside the WAV.
///
/// Beside the audio, and in the source tree's `testdata/`, because the marks *are* part
/// of the corpus — they are committed with the take they describe, and a take without
/// its truth is not evidence, just a sound file. One line per mark (JSONL, as
/// `main_app` persists its annotations) so a diff shows the note that changed rather
/// than the whole file.
pub fn marks_path(take: &Path) -> PathBuf {
    take.with_extension("marks.jsonl")
}

/// Read a take's marks. A take with no marks file yet has no marks — not an error.
///
/// A malformed *line* is skipped rather than fatal: the alternative is that one bad
/// line costs the user every mark in the file, and these are hand-made evidence that
/// cannot be regenerated by re-running anything.
pub fn load_marks(take: &Path) -> Vec<NoteMark> {
    let path = marks_path(take);
    let Ok(file) = File::open(&path) else {
        return Vec::new(); // no marks yet — the ordinary state of a fresh take
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// Write a take's marks, replacing whatever was there.
///
/// Rewritten whole rather than appended to, even though the format is append-shaped:
/// marks get deleted, and an append-only log would need tombstones and a compaction to
/// express that — a lot of machinery for a file that holds tens of lines and is edited
/// by hand-clicks. Sorted by time so the file reads like the take does and a diff stays
/// legible.
pub fn save_marks(take: &Path, marks: &[NoteMark]) -> Result<(), String> {
    let path = marks_path(take);
    let mut sorted: Vec<&NoteMark> = marks.iter().collect();
    sorted.sort_by(|a, b| a.interval.start.total_cmp(&b.interval.start));

    let mut body = String::new();
    for mark in sorted {
        let line = serde_json::to_string(mark).map_err(|e| e.to_string())?;
        body.push_str(&line);
        body.push('\n');
    }
    // An empty mark list writes an empty file rather than removing it: "this take has
    // been looked at and has no notes in it" and "nobody has marked this take" are
    // different claims, and the file is the only thing that can tell them apart.
    let mut file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declaration is the whole defence against the mirror, so what it accepts is
    /// load-bearing: it must parse what a violinist writes and refuse what merely looks
    /// like it.
    #[test]
    fn a_declaration_is_a_note_name_or_nothing() {
        assert_eq!(Declaration::parse("F5").unwrap().midi(), 77);
        assert_eq!(Declaration::parse("G3").unwrap().midi(), 55); // the open G
        assert_eq!(Declaration::parse("A4").unwrap().midi(), 69);
        assert_eq!(Declaration::parse("  A4  ").unwrap().midi(), 69);
        assert_eq!(Declaration::parse("A#4").unwrap().midi(), 70);
        assert_eq!(Declaration::parse("Bb4").unwrap().midi(), 70);

        // Half-typed: the ordinary state of a text field, and not armed.
        assert!(Declaration::parse("").is_none());
        assert!(Declaration::parse("F").is_none());
        // A tail must not be swallowed: `parse_anote` stops early and would happily
        // report F5, quietly declaring a note the user did not write.
        assert!(Declaration::parse("F5x").is_none());
        assert!(Declaration::parse("H4").is_none());
    }

    /// REGRESSION: a declaration cannot panic the app, however the field is typed into.
    ///
    /// `parse_octave` used `"999".parse::<u8>().unwrap()` under a `take_while1` that
    /// eats any run of digits — a live panic reachable by holding down a number key,
    /// and this field is the first thing ever to route user text into that parser.
    #[test]
    fn a_declaration_field_cannot_panic_whatever_is_typed() {
        for text in ["G999", "C99999999999999999999", "F256", "A-1", "5", "#", "Gb"] {
            let _ = Declaration::parse(text); // must return, not unwind
        }
        assert!(Declaration::parse("G999").is_none());
        assert!(Declaration::parse("F256").is_none(), "256 is not a u8 octave");
    }

    /// Clicks come in the order the user makes them, which is not always forwards.
    #[test]
    fn a_mark_is_normalised_whichever_click_came_first() {
        let f5 = Declaration::parse("F5").unwrap();
        let forwards = NoteMark::new(1.0, 2.5, f5, MarkedAgainst::Evidence);
        let backwards = NoteMark::new(2.5, 1.0, f5, MarkedAgainst::Evidence);
        assert_eq!(forwards.interval, backwards.interval);
        assert_eq!(forwards, backwards);
        assert!((forwards.seconds() - 1.5).abs() < 1e-9);
    }

    /// The mark's pitch is the declaration's, not anything on screen.
    #[test]
    fn a_marks_pitch_comes_from_the_declaration() {
        let declared = Declaration::parse("F5").unwrap();
        let mark = NoteMark::new(0.0, 1.0, declared, MarkedAgainst::Evidence);
        assert_eq!(mark.midi, 77, "F5 — whatever the detector drew at that moment");
    }

    /// Marks round-trip through the file, sorted, and an absent file is no marks.
    #[test]
    fn marks_round_trip_through_the_corpus_file() {
        let dir = std::env::temp_dir().join(format!("fretboard_marks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("g_flageolet_f5.wav");
        let _ = std::fs::remove_file(marks_path(&take));

        assert_eq!(
            marks_path(&take).file_name().unwrap(),
            "g_flageolet_f5.marks.jsonl"
        );
        assert!(load_marks(&take).is_empty(), "an unmarked take has no marks");

        let f5 = Declaration::parse("F5").unwrap();
        let g3 = Declaration::parse("G3").unwrap();
        // Written out of order, on purpose: the file is sorted, not the caller.
        let written = vec![
            NoteMark::new(3.0, 4.0, f5, MarkedAgainst::TheLineToo),
            NoteMark::new(1.0, 2.0, g3, MarkedAgainst::Evidence),
        ];
        save_marks(&take, &written).unwrap();

        let read = load_marks(&take);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].interval, 1.0..2.0, "the file reads like the take does");
        assert_eq!(read[0].midi, 55);
        assert_eq!(read[1].against, MarkedAgainst::TheLineToo);

        // Saving nothing means "looked at, no notes" — a different claim from "never
        // marked", and only the file's existence can tell them apart.
        save_marks(&take, &[]).unwrap();
        assert!(marks_path(&take).exists());
        assert!(load_marks(&take).is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One corrupt line costs that line, not the session's evidence.
    #[test]
    fn a_bad_line_does_not_cost_the_other_marks() {
        let dir = std::env::temp_dir().join(format!("fretboard_marks_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("t.wav");
        std::fs::write(
            marks_path(&take),
            "{\"interval\":{\"start\":1.0,\"end\":2.0},\"midi\":55,\"against\":\"Evidence\"}\n\
             not json at all\n\
             \n\
             {\"interval\":{\"start\":3.0,\"end\":4.0},\"midi\":77,\"against\":\"Evidence\"}\n",
        )
        .unwrap();

        let marks = load_marks(&take);
        assert_eq!(marks.len(), 2, "the readable marks must survive the bad line");
        assert_eq!(marks[1].midi, 77);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
