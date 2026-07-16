//! Note segmentation: turning the melody line's continuous pitch into written notes.
//!
//! The melody line (`dsp::melody`) says what pitch is sounding *right now*. This says
//! where one note ends and the next begins — glitch rejection, dropout grace, the
//! cents average a written note keeps. Those are musical decisions specified in
//! **seconds**, which is the whole reason this module exists.
//!
//! # Why this is not in the panel
//!
//! It was, and it was driven by `ui.input(|i| i.time)` — one call per UI frame. Three
//! things were wrong with that, in increasing order of how much they matter:
//!
//! 1. **A dropped frame changed a musical decision.** `MIN_NOTE_SECONDS` and
//!    `RELEASE_SECONDS` were sampled at whatever rate the compositor happened to be
//!    delivering, so a stutter could commit a note early, or fail to.
//! 2. **Note durations were frame-quantised.** A note's length was measured to the
//!    nearest ~16 ms tick of the *renderer*, which is not a musical quantity.
//! 3. **It was the last one left.** The octave decision had already moved into
//!    `dsp::melody` (Phase 1.8) for exactly this reason; segmentation was the
//!    remaining place where the UI clock reached into the music.
//!
//! The rule it now obeys — see `docs/note_detection.md` §4 — is that **anything whose
//! behaviour is specified in seconds must be driven by an audio clock**. Here that is
//! the resonator bank's publish cadence, timestamped off the **sample count**, which
//! also makes a note's duration sample-accurate rather than frame-quantised.
//!
//! # The asymmetry between a gap and a wrong note
//!
//! `pitch` arriving as `None` means silence **or** a slip the engine rejected, and the
//! two are deliberately indistinguishable here. A gap is harmless: [`RELEASE_SECONDS`]
//! absorbs it and the held note survives intact. A wrong-*octave* frame would be
//! destructive: it reads as a pitch change, commits the held note early, opens a bogus
//! one and restarts its timer — so with slips recurring nothing ever reaches
//! [`MIN_NOTE_SECONDS`] and the line writes **nothing at all**. That is not
//! hypothetical; it is how this was reported live. Upstream turns slips into gaps
//! precisely because this end treats gaps kindly.

use std::collections::VecDeque;

use crate::audio::types::{
    NoteLine,
    StaffNote,
};

/// A held note shorter than this (seconds) is discarded as a glitch, not written.
const MIN_NOTE_SECONDS: f64 = 0.06;
/// Grace period of silence before the held note is committed and cleared, so a brief
/// pitch dropout mid-note does not chop it in two. Also what absorbs a rejected slip
/// — see the module docs.
const RELEASE_SECONDS: f64 = 0.14;
/// Smoothing for the live cents of the held note (EMA weight, per bank frame).
///
/// Counted in **bank frames** (~16 ms), which is what fixes its timescale in seconds.
/// Driven per UI frame this silently tracked the frame rate: the same constant
/// averaged over ~4 s at 15 fps and ~1 s at 60 fps.
const CENTS_EMA: f32 = 0.25;
/// How many finished notes to keep. Only those that fit on screen are drawn; the rest
/// have already scrolled off, but a few extra are kept for when the panel is widened.
const HISTORY_CAP: usize = 96;

/// The note currently sounding, still accumulating. Timestamps are seconds on the
/// **audio** clock (see the module docs), not the UI's.
struct HeldNote {
    midi:  i32,
    cents: f32,
    onset: f64,
    last:  f64,
}

/// Segments the melody line into written notes.
///
/// Contract: [`update`] must be driven at the resonator bank's publish cadence, with
/// `now` taken from the **sample count** — one call per bank frame, exactly like
/// [`crate::audio::dsp::melody::MelodyTracker`], and for the same reason.
///
/// [`update`]: NoteSegmenter::update
#[derive(Default)]
pub(crate) struct NoteSegmenter {
    /// Finished notes, oldest → newest.
    history:        VecDeque<StaffNote>,
    /// The note sounding right now.
    current:        Option<HeldNote>,
    /// Last onset counter seen; a change means a fresh attack, used to split a
    /// re-bowed repeat of the same pitch into a new note.
    last_onset_seq: u64,
}

impl NoteSegmenter {
    /// Feed one bank frame and return the line as it now stands.
    ///
    /// `pitch` is the melody line's fractional MIDI, or `None` for silence **or** a
    /// rejected slip (see the module docs — the two are the same here on purpose).
    /// `onset_seq` is the engine's monotonic attack counter. `now` is the audio clock
    /// in seconds, derived from the sample count.
    pub(crate) fn update(&mut self, pitch: Option<f32>, onset_seq: u64, now: f64) -> NoteLine {
        match pitch {
            Some(midi_f) => {
                // Nearest semitone is the note; the fractional part is how far off
                // equal temperament it was played.
                let midi = midi_f.round();
                let cents = (midi_f - midi) * 100.0;
                let midi = midi as i32;

                // A fresh attack (onset counter moved) forces a new note even at the
                // same pitch, so a re-bowed repeat isn't merged into the held note.
                // Consumed only here, in the voiced branch: an onset during the
                // attack's brief unvoiced flicker stays pending until the pitch
                // returns, so it still splits.
                let onset = onset_seq != self.last_onset_seq;
                self.last_onset_seq = onset_seq;
                let same = self.current.as_ref().is_some_and(|h| h.midi == midi);
                if same && !onset {
                    // Same note still sounding: extend it and smooth the cents.
                    let h = self.current.as_mut().unwrap();
                    h.cents = h.cents * (1.0 - CENTS_EMA) + cents * CENTS_EMA;
                    h.last = now;
                } else {
                    // Different pitch, a re-attack of the same pitch, or the first
                    // note: finish the old, begin new.
                    self.end_current();
                    self.current = Some(HeldNote {
                        midi,
                        cents,
                        onset: now,
                        last: now,
                    });
                }
            }
            None => {
                // Commit the held note once the silence outlasts the grace period.
                let expired = self
                    .current
                    .as_ref()
                    .is_some_and(|h| now - h.last > RELEASE_SECONDS);
                if expired {
                    self.end_current();
                }
            }
        }
        self.line()
    }

    /// Move the held note into history if it lasted long enough to count.
    fn end_current(&mut self) {
        let Some(h) = self.current.take() else {
            return;
        };
        if h.last - h.onset >= MIN_NOTE_SECONDS {
            self.history.push_back(StaffNote {
                midi:  h.midi,
                cents: h.cents,
            });
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
    }

    /// The publishable snapshot: what the panel draws.
    fn line(&self) -> NoteLine {
        NoteLine {
            history: self.history.iter().copied().collect(),
            current: self.current.as_ref().map(|h| {
                StaffNote {
                    midi:  h.midi,
                    cents: h.cents,
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gated pitch this frame, as fractional MIDI — what `melody_pitch` carries.
    fn play(midi: i32, cents: f32) -> Option<f32> {
        Some(midi as f32 + cents / 100.0)
    }

    /// No onset this frame — the counter the engine last reported, unchanged.
    const NO_ONSET: u64 = 0;

    /// Cents survive a round trip through fractional MIDI to within a hundredth of a
    /// cent — `midi_f` is an `f32`, so exact equality is not on offer and not wanted.
    const CENTS_EPS: f32 = 0.01;

    /// The note sounding now, `(midi, cents)`.
    fn current(line: &NoteLine) -> Option<(i32, f32)> {
        line.current.map(|n| (n.midi, n.cents))
    }

    /// REGRESSION: the written line must show a note change promptly, end to end.
    ///
    /// Drives the real melody path — real `ResonatorAnalyzer` + real `PitchTracker`
    /// for the octave anchor → real `dsp::melody` snap → real `OctaveGate` → this
    /// segmenter — at the cadences the engine actually uses, and measures the ms from
    /// the true note change until the line *shows* the new note. This is the number
    /// the user perceives, and its absence is exactly how the melody came to be driven
    /// from pYIN alone (128 ms ordinary / 328 ms on an octave) without anyone
    /// noticing.
    #[test]
    fn end_to_end_latency_probe() {
        use std::f32::consts::TAU;

        use crate::audio::dsp::melody::MelodyTracker;
        use crate::audio::dsp::pyin::PitchTracker;
        use crate::audio::dsp::resonator::ResonatorAnalyzer;
        use crate::core_types::note::AccidentalStyle;

        fn violin_tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
            let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
            (0..len)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    partials
                        .iter()
                        .enumerate()
                        .map(|(h, a)| a * (TAU * frequency_hz * (h + 1) as f32 * t).sin())
                        .sum::<f32>()
                        / 3.0
                })
                .collect()
        }

        /// Drive the whole stack and return ms until the line shows `to_hz`.
        fn probe(from_hz: f32, to_hz: f32) -> f32 {
            let sr = 48_000.0f32;
            let window_size = 6144usize;
            let analysis_hop = 1920usize; // ANALYSIS_INTERVAL = 40 ms
            let bank_publish_ms = 16.0f32; // ResonatorSettings::update_ms
            let hold = (sr * 0.6) as usize;
            let mut sig = violin_tone(from_hz, sr, hold);
            sig.extend(violin_tone(to_hz, sr, hold));
            let change_ms = hold as f32 / sr * 1000.0;
            let target_midi = (69.0 + 12.0 * (to_hz / 440.0).log2()).round() as i32;

            let mut tracker = PitchTracker::new();
            let mut bank = ResonatorAnalyzer::new(sr);
            let mut melody = MelodyTracker::default();
            let mut segmenter = NoteSegmenter::default();
            // Prime the bank with everything up to the first publish, as the live bank
            // (fed continuously from the audio callback) would already be.
            bank.process_samples(&sig[..window_size], true);

            let mut anchor: Option<(f32, f32)> = None;
            let mut next_analysis = window_size;
            let mut bank_fed = window_size;
            let mut next_bank_publish_ms = 0.0f32;

            // Step at the BANK's cadence, which is what drives both the melody tracker
            // and this segmenter in the engine — the UI's frame rate no longer has a
            // vote in any of it, which is the point of the module.
            let step_ms = 1.0f32;
            let mut t_ms = window_size as f32 / sr * 1000.0;
            while t_ms < 1150.0 {
                let now_samples = ((t_ms * sr / 1000.0) as usize).min(sig.len());
                // The bank consumes audio continuously and republishes every ~16 ms.
                if now_samples > bank_fed {
                    bank.process_samples(&sig[bank_fed..now_samples], true);
                    bank_fed = now_samples;
                }
                // The pYIN path rebuilds the octave anchor every 40 ms.
                if now_samples >= next_analysis && next_analysis + window_size <= sig.len() {
                    let win = &sig[next_analysis - window_size..next_analysis];
                    anchor = tracker
                        .process(win, sr)
                        .map(|(f, c)| (69.0 + 12.0 * (f / 440.0).log2(), c));
                    next_analysis += analysis_hop;
                }
                if t_ms >= next_bank_publish_ms {
                    // `now` is the SAMPLE clock, exactly as the engine derives it — and now
                    // also what the melody's Viterbi measures its frame length off.
                    let now = now_samples as f64 / sr as f64;
                    let snapshot = bank.snapshot(true, AccidentalStyle::Sharps);
                    let melody_pitch = melody.update(
                        snapshot.salience.as_ref(),
                        anchor,
                        now,
                        crate::audio::types::PitchFrontend::ResonatorBank,
                    );
                    let line = segmenter.update(melody_pitch.map(|(m, _)| m), NO_ONSET, now);
                    if current(&line).map(|(m, _)| m) == Some(target_midi) {
                        return t_ms - change_ms;
                    }
                    next_bank_publish_ms = t_ms + bank_publish_ms;
                }
                t_ms += step_ms;
            }
            f32::INFINITY
        }

        // Two budgets, because one interval is genuinely harder than the rest.
        //
        // Any leap that CHANGES PITCH CLASS is caught by `melody`'s agreement guard on
        // the first frame, so it costs only the bank (8–29 ms) + the bank's 16 ms
        // publish tick. pYIN alone measured 128 ms for these.
        const BUDGET_MS: f32 = 60.0;
        // An OCTAVE leap is the one interval no single frame can tell from the bank
        // slipping an octave (A4 and A5 are the same pitch class), so it additionally
        // pays `melody::LEAP_CONFIRM_FRAMES` × the bank's 16 ms cadence before the leap
        // is believed. That is the deliberate price of not re-breaking the octave
        // wandering the snap exists to fix — it was 328 ms when the anchor was trusted
        // outright.
        //
        // 130 ms since Phase 1.11, and the arithmetic is worth knowing because it points
        // at the next fix rather than excusing this one: the measured 120 ms is the bank's
        // 56 ms (see `resonator::bank_latency_probe` — the octave is the interval whose
        // valleys the old note feeds longest) plus `LEAP_CONFIRM_FRAMES` × 16 ms = 64 ms.
        //
        // That second term is now **suspect**. It exists for exactly one job: telling a
        // real octave leap from the *bank slipping an octave*, which no single frame can
        // do because A4 and A5 share a pitch class. SWIPE′ does not slip: 0 of 612 frames
        // on a real bowed open G (`swipe::real_violin_g_probe`). So more than half of this
        // budget is a guard against a bug that no longer happens, and deleting it should
        // bring the octave back to ~72 ms — better than it has ever been.
        //
        // It is NOT deleted yet, on purpose. The evidence is one string: the open G, which
        // is where the phantom lived. Retiring the guard for the whole range on the
        // strength of that would be this plan's own recurring mistake — a number verified
        // in one condition, restated as a property. It needs recordings of the other
        // strings first.
        const OCTAVE_BUDGET_MS: f32 = 130.0;
        // The LOW register pays too, and Phase 1.11 (`dsp::swipe`) is why.
        //
        // SWIPE′ decides a note by asking whether a harmonic series *explains* the
        // column — including the energy in that candidate's valleys. While the old note
        // is still ringing in the bank, its partials sit in the new candidate's valleys
        // and vote against it. Until the bank decays they are not wrong: the old note
        // really is still sounding. This is the same mechanism that took the phantom
        // octave from 57% of frames to 0% on a real bowed G, so the cost is the price of
        // the fix, not a defect in it.
        //
        // Why the register and not the interval: A4->E5 and G3->D4 are the *same* fifth,
        // so the kernel geometry is identical — but they measure 13 ms and 80 ms in the
        // bank. The bank is constant-Q, so its ring-down is a roughly fixed number of
        // *cycles*, which is longer in seconds the lower you go. G3 is 196 Hz.
        //
        // 130 ms is the measurement (104 ms) plus headroom, and it is knowingly past the
        // ~40 ms perceptual threshold. That is the accepted trade, recorded in the plan
        // (Phase 1.11): the scorer this replaced answered in 8-29 ms and was wrong on
        // 57% of the frames of a real open G. A fast wrong note is not a note.
        //
        // This is a budget, not a target. If it is ever *reached*, something regressed;
        // the way to make it smaller is the bank's ring-down (a time/frequency trade in
        // `heuristic_alpha`), not a weaker kernel.
        const LOW_REGISTER_BUDGET_MS: f32 = 130.0;
        println!("\n=== melody end-to-end latency (what the user sees) ===");
        let cases = [
            ("A4->B4  (2nd)  ", 440.0f32, 493.88f32, BUDGET_MS),
            ("A4->D5  (4th)  ", 440.0, 587.33, BUDGET_MS),
            ("A4->E5  (5th)  ", 440.0, 659.25, BUDGET_MS),
            ("G3->D4  (violin G->D)", 196.0, 293.66, LOW_REGISTER_BUDGET_MS),
            ("A4->A5  (8ve)  ", 440.0, 880.0, OCTAVE_BUDGET_MS),
        ];
        // Measure everything before asserting anything: a probe that dies on its first
        // failure hides the *shape* of a regression, and the shape is the diagnosis.
        // (Exactly how Phase 1.11's two slow cases were told apart from each other.)
        let measured: Vec<(&str, f32, f32)> = cases
            .iter()
            .map(|&(name, from, to, budget)| (name, probe(from, to), budget))
            .collect();
        for (name, ms, budget) in &measured {
            println!("{name} -> {ms:>7.1} ms  (budget {budget:.0})");
        }
        for (name, ms, budget) in &measured {
            assert!(
                ms <= budget,
                "{name}: the line took {ms:.1} ms to show the note, budget {budget:.0} ms"
            );
        }
    }

    /// A one-frame blip shorter than `MIN_NOTE_SECONDS` is shown live but never
    /// committed to the written line.
    #[test]
    fn short_blip_is_not_written() {
        let mut s = NoteSegmenter::default();
        let line = s.update(play(69, 3.0), NO_ONSET, 0.0);
        assert_eq!(current(&line).unwrap().0, 69); // shown live immediately
        let line = s.update(None, NO_ONSET, RELEASE_SECONDS + 0.01); // past the grace period
        assert!(line.current.is_none());
        assert!(line.history.is_empty()); // held 0 s → discarded
    }

    /// A sustained note commits once released, and its cents are the smoothed EMA.
    #[test]
    fn sustained_note_commits_on_release() {
        let mut s = NoteSegmenter::default();
        s.update(play(69, 2.0), NO_ONSET, 0.0);
        s.update(play(69, 4.0), NO_ONSET, 0.10); // same pitch: extend + smooth cents
        let line = s.update(None, NO_ONSET, 0.10 + RELEASE_SECONDS + 0.01);
        assert!(line.current.is_none());
        assert_eq!(line.history.len(), 1);
        assert_eq!(line.history[0].midi, 69);
        // EMA: 2*(1-0.25) + 4*0.25 = 2.5
        assert!((line.history[0].cents - 2.5).abs() < CENTS_EPS);
    }

    /// Moving to a new pitch finishes the previous note and starts the new one.
    #[test]
    fn pitch_change_writes_previous() {
        let mut s = NoteSegmenter::default();
        s.update(play(69, 0.0), NO_ONSET, 0.0);
        s.update(play(69, 0.0), NO_ONSET, 0.10);
        let line = s.update(play(71, 0.0), NO_ONSET, 0.12); // A4 → B4
        assert_eq!(line.history.len(), 1);
        assert_eq!(line.history[0].midi, 69);
        assert_eq!(current(&line).unwrap().0, 71);
    }

    /// A dropout shorter than the grace period does not chop the held note.
    #[test]
    fn brief_dropout_keeps_current() {
        let mut s = NoteSegmenter::default();
        s.update(play(62, 0.0), NO_ONSET, 0.0);
        let line = s.update(None, NO_ONSET, 0.05); // < RELEASE_SECONDS → keep sounding
        assert_eq!(current(&line).unwrap().0, 62);
        let line = s.update(play(62, 0.0), NO_ONSET, 0.06); // pitch returns, same note
        assert_eq!(current(&line).unwrap().0, 62);
        assert!(line.history.is_empty());
    }

    /// REGRESSION: a dropped frame mid-note must not tear the held note apart.
    ///
    /// This is the segmenter's half of the octave-slip contract — see the module docs
    /// for why the engine hands a rejected slip over as `None` rather than as a pitch.
    #[test]
    fn a_dropped_frame_does_not_tear_the_held_note() {
        let mut s = NoteSegmenter::default();
        for i in 0..5 {
            s.update(play(69, 0.0), NO_ONSET, i as f64 * 0.02); // hold A4
        }
        // The engine rejected a frame (a slip, or a momentary dropout).
        let line = s.update(None, NO_ONSET, 0.10);
        assert_eq!(
            current(&line).unwrap().0,
            69,
            "a dropped frame must not disturb the held note"
        );
        assert!(
            line.history.is_empty(),
            "a dropped frame must not commit the held note"
        );

        // The note carries on and, once released, is written as exactly ONE note —
        // not the two fragments a slip used to produce.
        s.update(play(69, 0.0), NO_ONSET, 0.12);
        let line = s.update(None, NO_ONSET, 0.12 + RELEASE_SECONDS + 0.01);
        assert_eq!(
            line.history.len(),
            1,
            "the held note should survive the gap intact"
        );
        assert_eq!(line.history[0].midi, 69);
    }

    /// A leap to a new octave is tracked — the line must not go deaf after a big
    /// interval. (Whether the leap is real is settled upstream in `dsp::melody`; by
    /// the time it reaches here it is simply a new pitch.)
    #[test]
    fn octave_leap_is_still_written() {
        let mut s = NoteSegmenter::default();
        for i in 0..5 {
            s.update(play(69, 0.0), NO_ONSET, i as f64 * 0.02); // A4
        }
        let mut line = NoteLine::default();
        for i in 0..6 {
            line = s.update(play(81, 0.0), NO_ONSET, 0.10 + i as f64 * 0.02); // hold A5
        }
        assert_eq!(
            current(&line).unwrap().0,
            81,
            "a held octave leap must be tracked"
        );
    }

    /// A re-attack on the *same* pitch (onset counter advances) splits into a new note
    /// instead of extending the held one — the repeated-note capability.
    #[test]
    fn re_attack_splits_same_pitch() {
        let mut s = NoteSegmenter::default();
        s.update(play(69, 0.0), 1, 0.0); // first stroke starts
        let line = s.update(play(69, 0.0), 1, 0.10); // same onset → extend
        assert!(line.history.is_empty());
        let line = s.update(play(69, 0.0), 2, 0.12); // NEW onset, same pitch → split
        assert_eq!(line.history.len(), 1, "the first stroke should be committed");
        assert_eq!(line.history[0].midi, 69);
        assert_eq!(current(&line).unwrap().0, 69); // the second stroke is now current
    }

    /// REGRESSION: note duration is measured on the audio clock, so a note held for
    /// just over `MIN_NOTE_SECONDS` is written no matter how the caller is ticking.
    ///
    /// The point of the move: driven per UI frame this depended on the frame rate,
    /// because `now` was whatever the renderer last said. Here the same two timestamps
    /// decide it, whatever cadence the engine calls at.
    #[test]
    fn duration_is_measured_on_the_supplied_clock() {
        // Held exactly over the threshold → written.
        let mut s = NoteSegmenter::default();
        s.update(play(69, 0.0), NO_ONSET, 0.0);
        s.update(play(69, 0.0), NO_ONSET, MIN_NOTE_SECONDS);
        let line = s.update(None, NO_ONSET, MIN_NOTE_SECONDS + RELEASE_SECONDS + 0.01);
        assert_eq!(line.history.len(), 1, "a note at the threshold is written");

        // Held a hair under it → discarded, on the same number of calls.
        let mut s = NoteSegmenter::default();
        s.update(play(69, 0.0), NO_ONSET, 0.0);
        s.update(play(69, 0.0), NO_ONSET, MIN_NOTE_SECONDS - 0.001);
        let line = s.update(None, NO_ONSET, MIN_NOTE_SECONDS + RELEASE_SECONDS + 0.01);
        assert!(
            line.history.is_empty(),
            "a note under the threshold is a glitch, whatever the call count"
        );
    }
}
