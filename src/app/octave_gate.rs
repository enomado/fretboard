//! Lone-octave-slip rejection for the live pitch panels.
//!
//! The fused pYIN pitch on `TunerReading::frequency_hz` is octave-*stable*, not
//! octave-*perfect*. Two single-frame excursions survive the tracker: the resonator
//! bank fused into it occasionally crowns an overtone (a lone **+12** spike — the
//! "peaks at C6/A5" artefact), and the windowed front end can still drop to a
//! sub-bass ghost (the ~37 Hz "D0") for a frame. Every panel that reads the melody
//! line has to drop those, so the rule lives here once instead of per panel.
//!
//! The rule is **interval**-based, not duration-based: a sample `OCTAVE_REJECT`
//! semitones or more off the local median is a slip. Gating on the interval is what
//! lets even the fastest trill through (its interval is small) while dropping octave
//! jumps — a duration filter cannot tell a 2-frame trill note from a 2-frame spike.
//! The median window is short on purpose: a *sustained* octave leap re-establishes
//! the median within ~3 frames and is then accepted, so real leaps are never folded
//! away permanently.
//!
//! # Why a rejected frame must read as silence, not as a pitch
//!
//! Downstream, a `None` frame is *harmless* — the staff's release grace period
//! absorbs it and the held note survives; the roll draws a one-frame gap. An
//! accepted *wrong-octave* frame is *destructive* — it terminates the held note and
//! restarts note segmentation. That asymmetry is the whole reason this gate exists.

use std::collections::VecDeque;

/// Reject a sample this many semitones or more off the local median.
///
/// This must sit just under the **octave** it is named for, because the intervals it
/// has to let through are *melodic*, not just trills and vibrato. At the old value of
/// 7.0 this rejected a perfect fifth — which on a violin is every open-string
/// crossing (G3–D4–A4–E5 are tuned in fifths), the single most common leap there is.
/// A fifth only got through at all because the tracker happens to report 75.95 rather
/// than 76.0, i.e. by a rounding accident.
///
/// At 11.0 every interval up to a major seventh passes untouched and a ±12 slip is
/// still caught. A *sustained* octave leap also still tracks: it re-establishes the
/// median within ~3 frames (see [`MEDIAN_WINDOW`]).
const OCTAVE_REJECT: f32 = 11.0;
/// Window (frames) of recent raw pitch whose median is the rejection reference.
/// Short on purpose — see the module docs: a lone spike never moves the median
/// (rejected), but a sustained leap re-establishes it within ~3 frames (accepted).
const MEDIAN_WINDOW: usize = 5;

/// Stateful octave-slip rejector for one melody line; one per panel.
///
/// Contract: feed **every** voiced frame's raw fractional MIDI to [`accept`], in
/// order, and call [`reset`] on every silent frame. Skipping frames corrupts the
/// median and skipping `reset` judges the next phrase against the previous one's
/// octave.
///
/// [`accept`]: OctaveGate::accept
/// [`reset`]: OctaveGate::reset
pub(super) struct OctaveGate {
    /// Recent *raw* (pre-rejection) voiced pitches, oldest → newest. Holds the raw
    /// values, not the accepted ones: a median of already-filtered samples could
    /// never follow a genuine sustained leap, and the gate would latch to the first
    /// octave it saw.
    raw_recent: VecDeque<f32>,
}

impl Default for OctaveGate {
    fn default() -> Self {
        Self {
            raw_recent: VecDeque::with_capacity(MEDIAN_WINDOW),
        }
    }
}

impl OctaveGate {
    /// Judge one voiced frame's fractional MIDI against the local median. Returns
    /// the pitch when it is a real note, `None` when it is an octave slip (which the
    /// caller must treat exactly as it treats silence).
    ///
    /// The sample being judged is itself part of the median window — that is what
    /// lets a sustained leap take the median over within ~3 frames.
    pub(super) fn accept(&mut self, midi_f: f32) -> Option<f32> {
        self.raw_recent.push_back(midi_f);
        while self.raw_recent.len() > MEDIAN_WINDOW {
            self.raw_recent.pop_front();
        }
        // Below 3 samples there is no meaningful median yet: accept, so the first
        // notes of a phrase are never swallowed while the window fills.
        let spike = self.raw_recent.len() >= 3 && (midi_f - median(&self.raw_recent)).abs() >= OCTAVE_REJECT;
        (!spike).then_some(midi_f)
    }

    /// Silence ends the phrase: forget the median so the next note is judged on its
    /// own rather than against the previous phrase's octave.
    pub(super) fn reset(&mut self) {
        self.raw_recent.clear();
    }
}

/// Median of a small non-empty window — the robust reference for octave-spike
/// rejection (one or two outliers can't move it, unlike a mean).
fn median(values: &VecDeque<f32>) -> f32 {
    let mut v: Vec<f32> = values.iter().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lone +12 slip is rejected, while a trill-sized interval right after it is
    /// kept — the interval-based rule's whole point.
    #[test]
    fn octave_spike_rejected_but_trill_kept() {
        let mut g = OctaveGate::default();
        for _ in 0..5 {
            assert!(g.accept(69.0).is_some()); // establish A4
        }
        assert!(g.accept(81.0).is_none(), "lone +12 slip should be rejected");
        assert!(g.accept(71.0).is_some(), "trill neighbour should pass");
    }

    /// The sub-bass ghost (~37 Hz ≈ MIDI 26) under a held A4 is rejected. This is
    /// the slip the staff's old `MIDI_MIN = 48` floor used to hide and that lowering
    /// the floor to 24 (for the bass clefs) let through.
    #[test]
    fn sub_bass_ghost_rejected() {
        let mut g = OctaveGate::default();
        for _ in 0..5 {
            g.accept(69.0);
        }
        assert!(g.accept(26.0).is_none(), "sub-bass ghost should be rejected");
    }

    /// A *sustained* octave leap is accepted once the short median follows it, so
    /// real leaps aren't permanently folded away.
    #[test]
    fn sustained_octave_leap_accepted() {
        let mut g = OctaveGate::default();
        for _ in 0..5 {
            g.accept(69.0); // A4
        }
        let mut last = None;
        for _ in 0..5 {
            last = g.accept(81.0); // hold A5
        }
        assert!(
            last.is_some(),
            "a held octave leap is accepted once the median catches up"
        );
    }

    /// The first samples of a phrase pass while the window is still filling.
    #[test]
    fn first_samples_pass() {
        let mut g = OctaveGate::default();
        assert!(g.accept(69.0).is_some());
        assert!(g.accept(69.0).is_some());
    }

    /// After silence the gate judges the new phrase on its own octave, not the old
    /// one — without the reset, a phrase an octave down would be wholly rejected.
    #[test]
    fn reset_forgets_the_previous_phrase() {
        let mut g = OctaveGate::default();
        for _ in 0..5 {
            g.accept(69.0); // A4 phrase
        }
        g.reset();
        assert!(
            g.accept(57.0).is_some(),
            "a new phrase an octave down must not be rejected"
        );
    }
}
