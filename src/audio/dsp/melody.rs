//! Melody-line pitch: the resonator bank's fast fine pitch with its octave pinned
//! by pYIN. This is the "marry the two sources" the pitch survey recommends first,
//! and the architecture the violin plan's Phase 1.3 called the latency endgame.
//!
//! # Why the bank leads and YIN only pins the octave
//!
//! The two sources fail in opposite directions, and each one covers the other's
//! failure exactly:
//!
//! - The **bank** (`dsp::resonator`) is a per-sample IIR filterbank, so it follows a
//!   note change in **8–29 ms** (measured, `resonator::tests::bank_latency_probe`).
//!   Its weakness is the *octave*: on a bowed string the 2nd harmonic drifts louder
//!   and quieter than the fundamental, so a per-frame harmonic score flip-flops
//!   between f0 and 2·f0 ("octave wandering", seen live in Phase 1.3).
//! - **pYIN** (`dsp::pyin`) is octave-robust but pinned to its analysis window: it
//!   cannot leave a note while the window still holds *any* trace of it, which is
//!   **128 ms** at the default 6144-sample window (measured,
//!   `pyin::tests::latency_probe` — latency tracks window length exactly). During a
//!   *sustain*, though — which is where the wandering shows — it is rock steady.
//!
//! So: take timing and fine pitch from the bank, and borrow exactly one bit from
//! pYIN — which octave. The perceptual threshold for pitch feedback is ~30–40 ms
//! (see `docs/pitch_detection_survey.md`); the bank fits inside it, YIN never can.
//!
//! # Why this is not fusion inside the HMM
//!
//! Phase 1.5 tried the other arrangement — feed the bank into the pYIN HMM as an
//! extra weighted candidate — and it provably contributes nothing: the bank's
//! candidate is capped at `BANK_WEIGHT × strength ≤ 0.5` while YIN's own candidate
//! measures `p = 1.000`, so the bank loses every frame at any signal strength, and
//! the octave transition cost (~18 nats) buries it besides. Measured: feeding the
//! HMM a bank reading 10 ms ahead of the window changes the output by exactly zero.
//! The bank's speed can only survive if the bank *is* the pitch.

/// pYIN voiced probability below which its octave opinion is not trusted. Under it
/// the bank keeps its own octave — a guess from an unvoiced frame is worse than the
/// bank's harmonic scoring, which at least looked at the spectrum.
const YIN_OCTAVE_CONFIDENCE: f32 = 0.5;

/// How close the anchor must land to the snapped pitch, in semitones, for the snap
/// to be believed.
///
/// This is the guard that makes the snap safe when the anchor is stale *on a
/// different note*, and it is not optional. The naive snap moves the bank to
/// whichever octave sits nearest the anchor — but for ~128 ms after a leap the anchor
/// is still on the **previous** note. Snapping a fresh E5 (76) toward a stale A4 (69)
/// computes `round((69 − 76) / 12) = −1` and drags the note to E4 (64): a whole
/// octave wrong, caused entirely by trusting a stale anchor. So the snap only applies
/// when the two agree on the *pitch class*; a bigger disagreement means the anchor is
/// talking about a different note, and the bank — which is ~100 ms fresher — wins.
const OCTAVE_AGREE_SEMITONES: f32 = 1.5;

/// Consecutive bank frames of disputed octave after which the **bank** is believed
/// and the snap gives up.
///
/// [`OCTAVE_AGREE_SEMITONES`] cannot help here: an octave *leap* and an octave
/// *slip* are the same pitch class, so no single frame can tell "the player jumped to
/// A5" from "the bank crowned A4's 2nd harmonic". What separates them is **time**:
///
/// - **Slip/wandering** — the bank flip-flops between f0 and 2·f0 as the 2nd harmonic
///   drifts louder and quieter, so its disagreement is *intermittent*: any frame
///   where the bank is right resets the count, and it never runs up.
/// - **Real leap** — the bank is right and the anchor is stale, so the disagreement
///   is *unbroken* until the anchor catches up ~128 ms later.
///
/// So this threshold has to sit above the wandering timescale (1–2 frames) and below
/// the anchor's catch-up (~128 ms). At the bank's ~16 ms publish cadence, 4 frames
/// ≈ 64 ms lands between them with room on both sides. Cost of the compromise: an
/// octave leap shows the old octave for ~64 ms before it is believed — a quarter of
/// what trusting the anchor outright costs, and it decays to nothing as the anchor
/// catches up.
const LEAP_CONFIRM_FRAMES: u32 = 4;

/// Pin `bank_midi` to the octave of `anchor_midi`, when the two agree on the pitch
/// class. Returns the bank's pitch unchanged when the anchor is talking about some
/// other note (i.e. it is stale mid-leap — see [`OCTAVE_AGREE_SEMITONES`]).
///
/// The fractional part always comes from the bank: the snap moves the pitch by whole
/// octaves only, so the bank's fine pitch and cents survive it exactly.
fn snap_to_anchor_octave(bank_midi: f32, anchor_midi: f32) -> f32 {
    let octaves = ((anchor_midi - bank_midi) / 12.0).round();
    let snapped = bank_midi + 12.0 * octaves;
    if (anchor_midi - snapped).abs() <= OCTAVE_AGREE_SEMITONES {
        snapped
    } else {
        bank_midi
    }
}

/// Marries the bank's fast pitch to pYIN's octave for one melody line.
///
/// Stateful, because telling an octave *leap* from an octave *slip* is a question
/// about time and cannot be answered from a single frame — see
/// [`LEAP_CONFIRM_FRAMES`].
///
/// Contract: [`update`] must be driven at the **bank's** publish cadence (one call
/// per bank frame), since [`LEAP_CONFIRM_FRAMES`] is counted in those frames. Calling
/// it again from the slower pYIN path would double-count the dispute and believe a
/// leap early; that path re-stamps the last computed value instead.
///
/// [`update`]: MelodyTracker::update
#[derive(Default)]
pub(crate) struct MelodyTracker {
    /// Consecutive bank frames whose octave the anchor disputed. Reset by any frame
    /// the two agree on, which is what makes it read *unbroken* disagreement (a real
    /// leap) rather than *intermittent* disagreement (the bank wandering).
    octave_dispute: u32,
}

impl MelodyTracker {
    /// The played note for the melody panels (staff, pitch roll).
    ///
    /// `bank` is the resonator bank's fast fundamental `(fractional_midi, strength)`,
    /// or `None` when the bank is parked/quiet. `anchor` is pYIN's octave opinion
    /// `(fractional_midi, voiced_probability)`, or `None` when there is no reading
    /// yet.
    ///
    /// Returns `(fractional_midi, strength)` carrying the bank's timing and fine
    /// pitch, with pYIN's octave applied when pYIN is confident, agrees on the pitch
    /// class, and has not been disputed long enough to look stale.
    ///
    /// Contract: the caller still owns the silence gate. The bank's column is
    /// normalized, so it reports *some* fundamental even for room noise — absolute
    /// input level is the real silence gate, not the presence of a value here.
    pub(crate) fn update(
        &mut self,
        bank: Option<(f32, f32)>,
        anchor: Option<(f32, f32)>,
    ) -> Option<(f32, f32)> {
        let Some((bank_midi, strength)) = bank else {
            // Silence ends the phrase: the next note's octave is judged fresh rather
            // than against a dispute left over from the previous one.
            self.octave_dispute = 0;
            return None;
        };

        // No anchor, or pYIN is not voiced enough for its octave to be worth taking.
        let confident = anchor.filter(|(_, clarity)| *clarity >= YIN_OCTAVE_CONFIDENCE);
        let Some((anchor_midi, _)) = confident else {
            self.octave_dispute = 0;
            return Some((bank_midi, strength));
        };

        let snapped = snap_to_anchor_octave(bank_midi, anchor_midi);
        if (snapped - bank_midi).abs() < 0.5 {
            // The anchor agrees with the bank's own octave (or declined to move it):
            // nothing is in dispute.
            self.octave_dispute = 0;
            return Some((snapped, strength));
        }

        // The anchor wants to move the bank a whole octave. Believe it only while the
        // dispute is short — an unbroken run means the bank is holding a new octave
        // the anchor has not caught up to yet, i.e. a real leap.
        self.octave_dispute += 1;
        if self.octave_dispute > LEAP_CONFIRM_FRAMES {
            Some((bank_midi, strength))
        } else {
            Some((snapped, strength))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bank's fine pitch (and therefore its cents) survives a snap untouched —
    /// the snap only ever moves whole octaves.
    #[test]
    fn snap_preserves_fine_pitch() {
        let mut m = MelodyTracker::default();
        // Bank hears A4 17 cents sharp but an octave up; anchor says A4.
        let (midi, _) = m.update(Some((81.17, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!((midi - 69.17).abs() < 1e-4, "snapped to {midi}, expected 69.17");
    }

    /// The bank's octave wandering — the failure Phase 1.3 saw live — is corrected
    /// when pYIN is confident and agrees on the pitch class.
    #[test]
    fn snap_fixes_bank_octave_wandering() {
        // Bank crowned the 2nd harmonic: A5 instead of A4.
        let mut m = MelodyTracker::default();
        let (midi, _) = m.update(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 69.0).abs() < 1e-4,
            "expected the anchor's octave, got {midi}"
        );
        // Bank fell to the sub-octave: A3 instead of A4.
        let mut m = MelodyTracker::default();
        let (midi, _) = m.update(Some((57.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 69.0).abs() < 1e-4,
            "expected the anchor's octave, got {midi}"
        );
    }

    /// REGRESSION: intermittent wandering must be corrected *indefinitely*.
    ///
    /// The bank flip-flops f0/2·f0 as the 2nd harmonic drifts. Every frame the bank
    /// gets right resets the dispute, so the run never reaches
    /// `LEAP_CONFIRM_FRAMES` and the snap keeps fixing it — for as long as the note
    /// lasts, not just for the first few frames.
    #[test]
    fn intermittent_wandering_is_corrected_indefinitely() {
        let mut m = MelodyTracker::default();
        for i in 0..40 {
            // Alternate: correct A4, then a crowned-overtone A5.
            let bank = if i % 2 == 0 { 69.0 } else { 81.0 };
            let (midi, _) = m.update(Some((bank, 0.9)), Some((69.0, 0.9))).unwrap();
            assert!(
                (midi - 69.0).abs() < 1e-4,
                "frame {i}: wandering should stay corrected, got {midi}"
            );
        }
    }

    /// REGRESSION: a STALE anchor must not drag a fresh note an octave off.
    ///
    /// For ~128 ms after a leap the anchor is still on the previous note. Without the
    /// pitch-class guard, a fresh E5 (76) snapped toward a stale A4 (69) lands on
    /// E4 (64) — the naive snap's octave blunder. The bank is ~100 ms fresher, so it
    /// must win outright and immediately: the pitch classes differ, so no waiting.
    #[test]
    fn stale_anchor_does_not_drag_a_fresh_leap() {
        let mut m = MelodyTracker::default();
        let (midi, _) = m.update(Some((76.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 76.0).abs() < 1e-4,
            "a stale anchor a fifth away must not move the note; got {midi}"
        );
    }

    /// REGRESSION: a sustained octave LEAP must be believed once the dispute is
    /// unbroken past `LEAP_CONFIRM_FRAMES`.
    ///
    /// This is the case the pitch-class guard provably cannot catch — A4 and A5 are
    /// the same pitch class — and it is what made an octave leap take 261 ms end to
    /// end when the anchor was trusted outright.
    #[test]
    fn sustained_octave_leap_is_believed() {
        let mut m = MelodyTracker::default();
        // Settled on A4: bank and anchor agree.
        for _ in 0..4 {
            m.update(Some((69.0, 0.9)), Some((69.0, 0.9)));
        }
        // Player leaps to A5. The anchor stays on A4 for ~128 ms.
        let mut midi = 0.0;
        for _ in 0..=LEAP_CONFIRM_FRAMES {
            midi = m.update(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap().0;
        }
        assert!(
            (midi - 81.0).abs() < 1e-4,
            "an unbroken octave dispute is a real leap; got {midi}"
        );
    }

    /// …and the leap is held once believed, rather than flapping back on the next
    /// frame while the anchor is still catching up.
    #[test]
    fn believed_leap_stays_believed() {
        let mut m = MelodyTracker::default();
        for _ in 0..=LEAP_CONFIRM_FRAMES {
            m.update(Some((81.0, 0.9)), Some((69.0, 0.9)));
        }
        for i in 0..8 {
            let (midi, _) = m.update(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
            assert!(
                (midi - 81.0).abs() < 1e-4,
                "frame {i}: leap flapped back to {midi}"
            );
        }
    }

    /// Once the anchor catches up to the leap, the snap is back on duty: the bank's
    /// octave is pinned again, so wandering *after* a leap is still corrected.
    #[test]
    fn snap_resumes_after_the_anchor_catches_up() {
        let mut m = MelodyTracker::default();
        for _ in 0..=LEAP_CONFIRM_FRAMES {
            m.update(Some((81.0, 0.9)), Some((69.0, 0.9))); // leap, anchor stale
        }
        m.update(Some((81.0, 0.9)), Some((81.0, 0.9))); // anchor arrives → agreement
        // Now the bank wanders down an octave; the anchor must fix it at once.
        let (midi, _) = m.update(Some((69.0, 0.9)), Some((81.0, 0.9))).unwrap();
        assert!(
            (midi - 81.0).abs() < 1e-4,
            "snap should be live again, got {midi}"
        );
    }

    /// Silence resets the dispute, so the next phrase is judged on its own.
    #[test]
    fn silence_resets_the_dispute() {
        let mut m = MelodyTracker::default();
        for _ in 0..=LEAP_CONFIRM_FRAMES {
            m.update(Some((81.0, 0.9)), Some((69.0, 0.9))); // build an unbroken dispute
        }
        m.update(None, Some((69.0, 0.9)));
        // A fresh phrase whose bank octave is wrong must be corrected immediately,
        // not inherit the previous phrase's dispute and be believed.
        let (midi, _) = m.update(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 69.0).abs() < 1e-4,
            "dispute leaked across silence, got {midi}"
        );
    }

    /// An unconfident anchor has no opinion: the bank keeps its own octave rather
    /// than being snapped to a guess made on an unvoiced frame.
    #[test]
    fn unconfident_anchor_is_ignored() {
        let mut m = MelodyTracker::default();
        let (midi, _) = m.update(Some((81.0, 0.9)), Some((69.0, 0.1))).unwrap();
        assert!(
            (midi - 81.0).abs() < 1e-4,
            "expected the bank's octave, got {midi}"
        );
    }

    /// No bank → no melody note. The bank *is* the melody line; pYIN alone is the
    /// slow path this module exists to get off of.
    #[test]
    fn no_bank_no_note() {
        let mut m = MelodyTracker::default();
        assert!(m.update(None, Some((69.0, 0.9))).is_none());
    }

    /// With no anchor at all (no reading yet) the bank passes through.
    #[test]
    fn bank_passes_through_without_an_anchor() {
        let mut m = MelodyTracker::default();
        let (midi, strength) = m.update(Some((69.5, 0.7)), None).unwrap();
        assert!((midi - 69.5).abs() < 1e-4);
        assert!((strength - 0.7).abs() < 1e-4);
    }
}
