//! Melody-line pitch: SWIPE′'s salience curve decoded by continuity, with its octave still
//! cross-checked against pYIN. This is the "marry the two sources" the pitch survey
//! recommends first, and the architecture the violin plan's Phase 1.3 called the latency
//! endgame.
//!
//! # The two stages, and which one is load-bearing
//!
//! 1. [`SalienceDecoder`] — the bank frame's whole salience curve, run through the shared
//!    online Viterbi ([`super::trellis`]). This is where the played note is decided, and it
//!    is the only stage with a model of *time*.
//! 2. The **repair layer** — [`snap_to_anchor_octave`], [`LEAP_CONFIRM_FRAMES`] and
//!    [`OctaveGate`]. Three mechanisms built to patch a per-frame argmax that had no such
//!    model. Stage 1 now supplies it by construction, so these are expected to be
//!    redundant, and the plan retires them **in one piece** — after the D and E strings are
//!    recorded, not before. The evidence today is two strings; retiring a layer on that
//!    would be this plan's own recurring mistake.
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
//! extra weighted candidate — and it was removed, in two steps, for two different
//! reasons. Both are worth keeping, because the arrangement is a tempting one.
//!
//! Off an onset it **did nothing**: the bank's candidate was capped at `BANK_WEIGHT ×
//! strength ≤ 0.5` while YIN's own candidate measures `p = 1.000`, so the bank lost
//! every frame at any signal strength, and the octave transition cost (~18 nats)
//! buried it besides. Feeding the HMM a bank reading 10 ms ahead of the window changed
//! the output by exactly zero. The bank's speed can only survive if the bank *is* the
//! pitch — which is what this module does.
//!
//! *On* an onset it did far worse than nothing, and this is the part that had gone
//! unnoticed: the attack path also dropped the trellis, so the frame was decided by
//! emissions alone, where an `ATTACK_BANK_WEIGHT` of 2.0 beat YIN's 1.000 outright.
//! pYIN simply **echoed the bank's octave** — measured: window on a clean A4, bank
//! saying A5, tracker reporting A5. So the anchor this module cross-examines was the
//! bank's own opinion coming back around, at exactly the moment the bank is least
//! reliable (the attack transient), and the HMM's continuity then held that octave
//! for the frames after it. [`MelodyTracker::pin_octave`]'s three layers are built to
//! weigh two *independent* witnesses; a mirror cannot be a witness. If you are ever
//! tempted to re-fuse, the thing to check first is not the weight — it is whether the
//! anchor is still independent evidence.

use super::octave_gate::OctaveGate;
use super::swipe::SalienceFrame;
use super::trellis::{
    Emissions,
    PitchGrid,
    PitchState,
    PitchTrellis,
};

/// Frame length assumed for the very first frame of a phrase, when there is no predecessor
/// to measure against (`ResonatorSettings::update_ms`'s default).
///
/// It is never used for anything that matters: a first frame is decided by its emissions
/// alone — the trellis has no path to weigh it against — so the kernel this builds is
/// discarded unread. Every subsequent frame measures its own `dt` off the audio clock.
const NOMINAL_FRAME_DT: f32 = 0.016;

/// How many nats of evidence one unit of SWIPE′ salience is worth — the exchange rate
/// between the observation and the transition costs in [`super::trellis`].
///
/// **This could not be inherited from pYIN, and assuming it could is a mistake worth
/// recording.** The trellis's rates carry over between channels because they describe how a
/// player's pitch moves in *time*, which no detector changes. The emission scale is the
/// opposite: pYIN's candidates are near-delta functions (a candidate measures `p = 1.000`
/// against an emission floor of 1e-9 — a contrast of **20.7 nats**), so a leap's ~9.9 nats
/// is pocket change to it. SWIPE′'s salience is a broad, low-contrast curve where the right
/// note beats junk by 0.017. Feeding *that* to a kernel calibrated against deltas priced
/// every note change out of reach: measured, the octave went 120 ms → 248 ms and a whole
/// tone 24 ms → 120 ms. The rates were right and the exchange rate was nonsense.
///
/// It is bounded from both sides, and both bounds are measured rather than argued:
///
/// - **Too low** and real notes never arrive — the leap's 9.9 nats outlasts the latency
///   budget (`segmenter::end_to_end_latency_probe`).
/// - **Too high** and the 4–6% ties buy their way through in a frame or two, which is the
///   phantom octave coming back (`swipe::real_violin_g_probe`).
///
/// The value is the sweep's, not a guess — see [`beta_sweep::salience_beta_sweep`], which prints
/// both edges of the trade across the whole corpus.
const SALIENCE_BETA: f32 = 40.0;

/// SWIPE′'s salience curve, decoded into a played note by continuity rather than by argmax.
///
/// # What this fixes, and what it cannot
///
/// Phase 1.11 measured the remainder on a real violin and found it is **not** a confident
/// error — it is a tie:
///
/// ```text
/// frame 1.26s : junk 0.4006   G3 0.3833     ← 4.5% apart
/// frame 2.09s : junk 0.2990   G3 0.2819     ← 6% apart
/// ```
///
/// The right note loses to noise by 4–6%. No threshold fixes that, because there is no
/// threshold to set: the tie is in the evidence. What breaks it is time — a lone excursion
/// costs the trellis a leap (~9.9 nats) and buys back 4–6% of emission (~0.04 nats), so it
/// loses by a factor of ~225 and the path holds. It would take *seconds* of sustained junk
/// to pay that off, which is exactly the distinction wanted.
///
/// What it cannot do is invent a candidate the salience never proposed. This decodes the
/// curve; it does not improve it.
///
/// # The trap: "an octave jump is an error"
///
/// It is not, and a rule saying so would be wrong on this app's own test corpus —
/// `testdata/g_open_real_octave.wav` is the user deliberately *playing* an octave. The
/// difference between a phantom octave and a real one was never the interval: it is whether
/// the evidence **persists**. A real leap's old note goes quiet, so its emission collapses
/// and the new note pays off the leap's 9.9 nats within a frame or three (measured:
/// `trellis::leap_latency_probe`, 48 ms at this cadence). A phantom's true note never
/// leaves, so it never pays. That is a property of the path, and it needs no rule at all.
pub(crate) struct SalienceDecoder {
    /// The continuity model, and `None` until a frame says which grid to build it on.
    ///
    /// The grid cannot be known up front: the bank publishes the reassigned path on a fixed
    /// `OUTPUT_BINS_PER_SEMITONE` and the rollback path on the user's own resolution, so
    /// only a frame knows. Rebuilt — not merely reset — when that grid changes under it,
    /// because a trellis whose states meant one pitch cannot go on meaning another.
    trellis: Option<PitchTrellis>,
    /// Audio-clock timestamp of the last frame, for the `dt` the kernel is cut to.
    last_t:  Option<f64>,
    /// Salience→nats exchange rate. A field rather than a constant only so
    /// [`beta_sweep::salience_beta_sweep`] can measure the trade it makes; production uses
    /// [`SALIENCE_BETA`].
    beta:    f32,
}

impl Default for SalienceDecoder {
    fn default() -> Self {
        Self {
            trellis: None,
            last_t:  None,
            beta:    SALIENCE_BETA,
        }
    }
}

impl SalienceDecoder {
    /// A decoder at a chosen exchange rate — for the sweep that sets [`SALIENCE_BETA`].
    #[cfg(test)]
    fn with_beta(beta: f32) -> Self {
        Self {
            beta,
            ..Default::default()
        }
    }

    /// Forget the path. Silence ends a phrase, and the next note must be judged on its own
    /// evidence rather than against a note that has already stopped.
    fn reset(&mut self) {
        if let Some(trellis) = self.trellis.as_mut() {
            trellis.reset();
        }
        self.last_t = None;
    }

    /// Decode one bank frame into `(fractional_midi, strength)`.
    ///
    /// Contract: `now_seconds` is the **audio** clock (off the sample count), and this is
    /// driven once per bank frame. Both are load-bearing — see [`super::trellis`] for why a
    /// frame length measured off the wall, or off a slider, would put a display knob back in
    /// charge of the detector.
    fn decode(&mut self, frame: &SalienceFrame, now_seconds: f64) -> Option<(f32, f32)> {
        // The trellis decodes the app's pitch domain at the frame's own resolution:
        // resampling the curve onto some other grid would blur the very peaks the decision is
        // made from.
        //
        // The domain comes from `tracked_bins`, which is C1..C8 **clamped to what this bank
        // actually reaches** — and taking it from the constants instead was a crash, not a
        // detail. `ResonatorSettings::max_midi` is a slider (a direct CPU dial), so the curve
        // is only as long as the user's bank: at the C8 default the last state lands on the
        // curve's last bin, i.e. it fitted by *exactly nothing*, and a bank ceiling of C6 had
        // this reading 192 bins past the end. That panics on the audio thread, which kills
        // the bank worker, which empties the staff and the pitch roll — the report was
        // "вообще нот нет".
        let bins = frame.tracked_bins()?;
        let (lo, hi) = (frame.midi_of_bin(*bins.start()), frame.midi_of_bin(*bins.end()));
        if hi <= lo {
            return None; // a bank narrower than one bin of the domain: nothing to decode
        }
        let grid = PitchGrid::new(lo, hi, frame.bins_per_semitone());
        let dt = self
            .last_t
            .map(|last| (now_seconds - last) as f32)
            .filter(|dt| *dt > 0.0)
            .unwrap_or(NOMINAL_FRAME_DT);
        self.last_t = Some(now_seconds);

        let stale = self
            .trellis
            .as_ref()
            .is_none_or(|t| t.grid().n_pitch() != grid.n_pitch());
        if stale {
            self.trellis = Some(PitchTrellis::new(grid, dt));
        }
        let trellis = self.trellis.as_mut().unwrap();

        // Curve bin ↔ trellis state. Both grids are log-linear in MIDI at the same
        // resolution, so they differ by a constant offset — the octave the waterfall draws
        // below the app's pitch domain. This is the one arithmetic step where a slip is a
        // silent transposition rather than a crash, which is why `PitchState` is a newtype
        // and why the offset is the *same* `tracked_bins` the grid above came from, rather
        // than a second derivation that has to agree with it.
        let offset = *bins.start();

        // Emissions: a Gibbs link from salience to likelihood, `exp(β·(s − s_max))`.
        //
        // A salience is a *score*, not a probability, so something has to convert one into
        // the other, and the choice is not free — it sets the exchange rate between "how
        // much better does this pitch explain the spectrum" and "how much does moving there
        // cost". Get it wrong and the trellis is either deaf to real notes or blind to junk.
        //
        // Exponential, because it is the one link that uses only what SWIPE′'s scale
        // actually means. That scale is **not** the thesis's: it is normalized against a
        // 480-resonator C0..C8 column where Camacho used a narrow FFT band, so an absolute
        // salience here is an invented number and only *comparisons* are assertable (design
        // §1.4). `exp(β·s)` is shift-invariant — add a constant to the whole curve and every
        // emission ratio is unchanged — so only differences ever reach the trellis, which is
        // exactly that rule made structural. Subtracting `s_max` also pins the per-frame
        // scale at 1, so a hard bow and a soft one produce the same emissions.
        //
        // The peak's own breadth is why the link must be *sharpening* rather than linear:
        // SWIPE′'s h=1 lobe alone spans 71 bins at this grid, so the curve is a broad hump
        // and its raw values put the right note only a few percent above its neighbours.
        let mut emissions = Emissions::zeroed(trellis.grid());
        let mut max_salience = f32::NEG_INFINITY;
        for state in 0..trellis.grid().n_pitch() {
            max_salience = max_salience.max(frame.salience_at(offset + state));
        }
        if max_salience <= 0.0 {
            // Nothing in C1..C8 looks like a harmonic series at all.
            return None;
        }
        for state in 0..trellis.grid().n_pitch() {
            let salience = frame.salience_at(offset + state);
            emissions.set_state(PitchState(state), ((salience - max_salience) * self.beta).exp());
        }
        // Voicing is the *level gate's* job, upstream (`core::MELODY_LEVEL_GATE`) — SWIPE′'s
        // absolute scale is not a confidence and must never be thresholded as one (design
        // §1.4). So this channel never votes itself unvoiced; the unvoiced state stays a
        // route the trellis offers and this caller declines.
        emissions.set_unvoiced(0.0);

        let state = trellis.step(&emissions, dt)?;
        // The decoded bin names the harmonic series; the partials say exactly where it is.
        // Re-read at the bin that *won* — which on an overruled frame is not the argmax's.
        Some(frame.resolve(offset + state.0))
    }
}

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
/// ≈ 64 ms landed between them with room on both sides.
///
/// **It was 4 until the Viterbi landed, and the reason it can now be 1 is written in the
/// paragraph above.** The upper bound never moved; the *lower* one did. This guard is sized
/// against the wandering timescale — but wandering is exactly what
/// [`SalienceDecoder`] now removes by construction, since a flip-flopping octave is a lone
/// excursion and loses to continuity by ~225×. Measured on the real violin: the bank's own
/// octave slips are 0.0% of frames on both open-G takes (`beta_sweep::salience_beta_sweep`).
/// A guard against a thing that no longer happens should sit at its floor.
///
/// The floor is 1 rather than 0 because at 0 this stops being a threshold at all: the bank
/// would win *every* dispute on its first frame, which makes [`snap_to_anchor_octave`]
/// return `bank_midi` unconditionally — dead code, taking [`YIN_OCTAVE_CONFIDENCE`] and this
/// module's whole dependency on the slow anchor with it. That is three of the four pieces
/// the plan retires **together, after the D and E strings are recorded** — on the evidence
/// of two strings it would be this plan's own recurring mistake, a number verified in one
/// condition restated as a property. At 1, an intermittent slip still resets the count and
/// the snap still corrects it, so the layer stays whole and honest until that gate opens.
///
/// Cost: an octave leap shows the old octave for ~16 ms before it is believed, down from
/// ~64 ms — which is what brings the end-to-end octave back under budget now that the
/// Viterbi holds the note too (`segmenter::end_to_end_latency_probe`: 152 ms → 104 ms).
const LEAP_CONFIRM_FRAMES: u32 = 1;

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

/// Marries the bank's fast pitch to pYIN's octave for one melody line, and is the
/// **only** place the melody line's octave is decided.
///
/// Three layers act on the octave here, in this order, each covering the previous
/// one's blind spot:
/// 1. the **bank's** own harmonic scoring (`analysis_math::resonator_fundamental`)
///    picks a fundamental over a louder overtone — upstream, its best single-frame
///    guess;
/// 2. the **snap** pins that to pYIN's octave when pYIN is confident and agrees on
///    the pitch class, with [`LEAP_CONFIRM_FRAMES`] to keep a stale anchor from
///    dragging a real leap;
/// 3. the [`OctaveGate`] rejects what is left — a lone slip on a frame where pYIN
///    had *no* opinion (unvoiced, or below [`YIN_OCTAVE_CONFIDENCE`]), which is
///    exactly when layer 2 stands down.
///
/// Stateful, because telling an octave *leap* from an octave *slip* is a question
/// about time and cannot be answered from a single frame — see
/// [`LEAP_CONFIRM_FRAMES`].
///
/// Contract: [`update`] must be driven at the **bank's** publish cadence (one call
/// per bank frame). Both the dispute counter and the gate's median are counted in
/// those frames, so this is what fixes their timescale in *seconds*. The gate used to
/// live in the panels and be driven per UI frame, which made a DSP filter's window
/// depend on the frame rate — a dropped frame quietly changed its behaviour. Calling
/// this again from the slower pYIN path would likewise double-count; that path
/// re-stamps the last computed value instead.
///
/// [`update`]: MelodyTracker::update
#[derive(Default)]
pub(crate) struct MelodyTracker {
    /// Layer 0: the salience curve decoded by continuity. Everything below it is the
    /// **repair layer** — three mechanisms built to patch a per-frame argmax that had no
    /// model of time, which is what this decoder now supplies by construction.
    ///
    /// The repair layer is therefore expected to be redundant, and the plan retires it in
    /// one piece (`snap_to_anchor_octave`, [`LEAP_CONFIRM_FRAMES`], [`OctaveGate`], and this
    /// module's whole dependency on the slow pYIN anchor). It has **not** been retired yet,
    /// deliberately: the evidence is two strings, G and A. Removing a layer on the strength
    /// of a number measured in one condition is the exact mistake this plan has already made
    /// twice — the D and E takes are the gate, and they are not recorded.
    decoder:        SalienceDecoder,
    /// Consecutive bank frames whose octave the anchor disputed. Reset by any frame
    /// the two agree on, which is what makes it read *unbroken* disagreement (a real
    /// leap) rather than *intermittent* disagreement (the bank wandering).
    octave_dispute: u32,
    /// Last-resort slip rejection, after the snap has had its say. See
    /// [`OctaveGate`].
    gate:           OctaveGate,
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
    /// class, and has not been disputed long enough to look stale. `None` for silence
    /// **or for a rejected slip** — the two are deliberately the same to the caller,
    /// because downstream a missing frame is harmless (the staff's release grace
    /// carries the held note through it) while a wrong-octave frame tears the note in
    /// two.
    ///
    /// Contract: the caller still owns the silence gate. The bank's column is
    /// normalized, so it reports *some* fundamental even for room noise — absolute
    /// input level is the real silence gate, not the presence of a value here.
    pub(crate) fn update(
        &mut self,
        frame: Option<&SalienceFrame>,
        anchor: Option<(f32, f32)>,
        now_seconds: f64,
    ) -> Option<(f32, f32)> {
        let Some(frame) = frame else {
            self.decoder.reset();
            return self.pin_and_gate(None, anchor);
        };
        let bank = self.decoder.decode(frame, now_seconds);
        self.pin_and_gate(bank, anchor)
    }

    /// The **repair layer**: layers 2 and 3, on an already-decoded note.
    ///
    /// Split from [`Self::update`] so it can be driven with a pitch directly — which is how
    /// every one of its regression tests below is written, and they are the record of what
    /// these three mechanisms are *for*. When the D and E takes land and the layer is
    /// retired, this function and those tests go together, in one piece.
    fn pin_and_gate(&mut self, bank: Option<(f32, f32)>, anchor: Option<(f32, f32)>) -> Option<(f32, f32)> {
        let Some((bank_midi, strength)) = bank else {
            // Silence ends the phrase: the next note's octave is judged fresh rather
            // than against a dispute, or a median, left over from the previous one.
            self.octave_dispute = 0;
            self.gate.reset();
            return None;
        };
        let (midi, leap_confirmed) = self.pin_octave(bank_midi, anchor);
        if leap_confirmed {
            // Layer 2 has just concluded, from an unbroken run of dispute, that this
            // is a real octave leap. Layer 3 must not re-litigate it: the gate's
            // median is still sitting on the *old* octave, so left alone it would
            // reject the leap for another few frames out of pure inertia — a second
            // conservatism tax on a decision already paid for. A confirmed leap is a
            // new phrase as far as the gate is concerned.
            self.gate.reset();
        }
        // Layer 3: whatever the snap could not settle. A no-op when the anchor was
        // confident — the series is already clean, so nothing sits off the median.
        self.gate.accept(midi).map(|midi| (midi, strength))
    }

    /// Layers 1–2: the bank's own octave, pinned to the anchor's when that is worth
    /// believing. Split out so [`update`] reads as the three layers it is.
    ///
    /// Returns the pitch, and whether *this* frame is the one that first believed a
    /// leap — the transition, not the state, so the caller resets the gate once
    /// rather than holding it open for the whole stale-anchor window.
    ///
    /// [`update`]: MelodyTracker::update
    fn pin_octave(&mut self, bank_midi: f32, anchor: Option<(f32, f32)>) -> (f32, bool) {
        // No anchor, or pYIN is not voiced enough for its octave to be worth taking.
        let confident = anchor.filter(|(_, clarity)| *clarity >= YIN_OCTAVE_CONFIDENCE);
        let Some((anchor_midi, _)) = confident else {
            self.octave_dispute = 0;
            return (bank_midi, false);
        };

        let snapped = snap_to_anchor_octave(bank_midi, anchor_midi);
        if (snapped - bank_midi).abs() < 0.5 {
            // The anchor agrees with the bank's own octave (or declined to move it):
            // nothing is in dispute.
            self.octave_dispute = 0;
            return (snapped, false);
        }

        // The anchor wants to move the bank a whole octave. Believe it only while the
        // dispute is short — an unbroken run means the bank is holding a new octave
        // the anchor has not caught up to yet, i.e. a real leap.
        self.octave_dispute += 1;
        if self.octave_dispute > LEAP_CONFIRM_FRAMES {
            (bank_midi, self.octave_dispute == LEAP_CONFIRM_FRAMES + 1)
        } else {
            (snapped, false)
        }
    }
}

/// The sweep that sets [`SALIENCE_BETA`], on the real violin.
///
/// [`SALIENCE_BETA`] is a single number that trades two properties against each other, and
/// neither of them can be reasoned out — they are both facts about a bowed string. So the
/// sweep drives the **whole corpus** through the production decoder at each candidate rate
/// and prints both edges at once:
///
/// - the phantom octave and the low-register junk, which want β *small* (continuity wins);
/// - the note-change latency, which wants β *large* (evidence wins).
///
/// Built on this app's own recurring lesson: a constant verified in one condition is not a
/// property. So the value is read off the corpus at both edges rather than argued for — see
/// `testdata/README.md` for what each take does and does not prove.
#[cfg(test)]
mod beta_sweep {
    use super::*;
    use crate::audio::dsp::resonator::ResonatorAnalyzer;
    use crate::core_types::note::AccidentalStyle;

    /// The candidate exchange rates. Spans three orders of magnitude because the honest
    /// prior on this number was "nobody knows" — pYIN's implied rate (its emissions are
    /// deltas) is effectively infinite, and the linear link is β = 1.
    const CANDIDATES: [f32; 7] = [1.0, 5.0, 10.0, 20.0, 40.0, 80.0, 200.0];

    /// Decode one take at one β; returns the MIDI verdict per bank frame (`None` = no
    /// pitch). Drives the production `SalienceDecoder` over the production snapshot, so this
    /// measures the wired engine rather than a re-implementation of it.
    fn decode_take(name: &str, beta: f32) -> Vec<Option<f32>> {
        let path = format!("{}/testdata/{name}.wav", env!("CARGO_MANIFEST_DIR"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let sample_rate = reader.spec().sample_rate as f32;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut analyzer = ResonatorAnalyzer::new(sample_rate);
        let mut decoder = SalienceDecoder::with_beta(beta);
        // The bank's own ~16 ms publish cadence — the rate `MelodyTracker` is contractually
        // driven at, and what the decoder measures its `dt` against.
        let hop = (sample_rate * 0.016) as usize;
        let mut verdicts = Vec::new();
        let mut fed = 0usize;
        while fed + hop <= samples.len() {
            analyzer.process_samples(&samples[fed..fed + hop], true);
            fed += hop;
            let now = fed as f64 / sample_rate as f64;
            // Skip the first second: the bank charges from empty, so the opening frames are
            // junk for a reason that has nothing to do with this decision. (The first cut of
            // the low-register probe caught only those and "disproved" itself.)
            let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);
            let decoded = snapshot
                .salience
                .as_ref()
                .and_then(|frame| decoder.decode(frame, now));
            if now >= 1.0 {
                verdicts.push(decoded.map(|(midi, _)| midi));
            }
        }
        verdicts
    }

    /// The per-frame argmax over the same take — the baseline the Viterbi has to beat.
    fn argmax_take(name: &str) -> Vec<Option<f32>> {
        let path = format!("{}/testdata/{name}.wav", env!("CARGO_MANIFEST_DIR"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let sample_rate = reader.spec().sample_rate as f32;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut analyzer = ResonatorAnalyzer::new(sample_rate);
        let hop = (sample_rate * 0.016) as usize;
        let mut verdicts = Vec::new();
        let mut fed = 0usize;
        while fed + hop <= samples.len() {
            analyzer.process_samples(&samples[fed..fed + hop], true);
            fed += hop;
            let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);
            if fed as f32 / sample_rate >= 1.0 {
                verdicts.push(snapshot.fundamental.map(|(midi, _)| midi));
            }
        }
        verdicts
    }

    /// Percentage of frames landing inside `band` (inclusive, MIDI), and the percentage that
    /// are an octave of something in it.
    fn tally(verdicts: &[Option<f32>], band: std::ops::RangeInclusive<i32>) -> (f32, f32) {
        let total = verdicts.len().max(1) as f32;
        let mut inside = 0u32;
        let mut octave = 0u32;
        for midi in verdicts.iter().flatten() {
            let rounded = midi.round() as i32;
            if band.contains(&rounded) {
                inside += 1;
            } else if band.contains(&(rounded - 12)) || band.contains(&(rounded + 12)) {
                octave += 1;
            }
        }
        (100.0 * inside as f32 / total, 100.0 * octave as f32 / total)
    }

    /// The takes, and the truth of each — **as the player stated it**, never as guessed. The
    /// trills are fingers in first position, so every note from the open string to +7 is real
    /// content (`testdata/README.md`).
    ///
    /// The last column is what makes `g_open_real_octave` a **control** rather than another
    /// take: notes that must **remain**. An in-band tally cannot convict anything there,
    /// because G3 is itself in the band — a decoder frozen on G3 for the whole take scores
    /// "100% in band" while having missed the entire point. Only "how many frames are on the
    /// G4 the player actually bowed" can tell tracking from paralysis, and the sweep's low-β
    /// rows are exactly that failure caught in the act.
    const TAKES: [(&str, i32, i32, &[(&str, i32)]); 5] = [
        ("g_open_slow_strokes", 55, 55, &[]),
        ("g_open_fast_strokes", 55, 55, &[]),
        ("g_open_real_octave", 55, 67, &[("G3", 55), ("G4", 67)]),
        ("g_string_trill", 55, 62, &[]),
        ("a_string_trill", 69, 76, &[]),
    ];

    /// Percentage of frames sitting on exactly `midi`.
    fn on_note(verdicts: &[Option<f32>], midi: i32) -> f32 {
        let total = verdicts.len().max(1) as f32;
        let hits = verdicts
            .iter()
            .flatten()
            .filter(|m| m.round() as i32 == midi)
            .count();
        100.0 * hits as f32 / total
    }

    /// A bowed-string-ish tone: fundamental + partials, since a pure sine has no octave
    /// ambiguity at all and would flatter any scorer.
    fn violin_tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                partials
                    .iter()
                    .enumerate()
                    .map(|(h, a)| a * (std::f32::consts::TAU * frequency_hz * (h + 1) as f32 * t).sin())
                    .sum::<f32>()
                    / 3.0
            })
            .collect()
    }

    /// Milliseconds from a note change until the decoder reports the new note, at `beta`.
    ///
    /// **The other edge of the trade, and the one that convicts a stuck needle.** A decoder
    /// that never moves scores beautifully on a take whose note never changes — which is
    /// three of the five takes above. Only this can tell "it is right" from "it is stuck".
    fn change_latency_ms(from_hz: f32, to_hz: f32, beta: f32) -> f32 {
        let sample_rate = 48_000.0f32;
        let hold = (sample_rate * 0.6) as usize;
        let mut signal = violin_tone(from_hz, sample_rate, hold);
        signal.extend(violin_tone(to_hz, sample_rate, hold));

        let target_midi = 69.0 + 12.0 * (to_hz / 440.0).log2();
        let mut analyzer = ResonatorAnalyzer::new(sample_rate);
        let mut decoder = SalienceDecoder::with_beta(beta);
        let hop = (sample_rate * 0.016) as usize;
        let mut fed = 0usize;
        while fed + hop <= signal.len() {
            analyzer.process_samples(&signal[fed..fed + hop], true);
            fed += hop;
            let now = fed as f64 / sample_rate as f64;
            let snapshot = analyzer.snapshot(true, AccidentalStyle::Sharps);
            let decoded = snapshot
                .salience
                .as_ref()
                .and_then(|frame| decoder.decode(frame, now));
            if fed <= hold {
                continue;
            }
            if let Some((midi, _)) = decoded {
                if (midi - target_midi).abs() < 0.5 {
                    return (fed - hold) as f32 / sample_rate * 1000.0;
                }
            }
        }
        f32::INFINITY
    }

    #[test]
    #[ignore = "needs no testdata — run with --ignored --nocapture"]
    fn salience_beta_latency_sweep() {
        println!("\n=== SALIENCE_BETA vs note-change latency (ms) ===");
        println!("(a stuck needle reads `inf` here and 100% on a take whose note never moves)");
        let cases = [
            ("A4->B4 (2nd) ", 440.0f32, 493.88f32),
            ("A4->E5 (5th) ", 440.0, 659.25),
            ("A4->A5 (8ve) ", 440.0, 880.0),
            ("G3->D4 (low) ", 196.0, 293.66),
        ];
        print!("{:<16}", "beta");
        for (name, _, _) in cases {
            print!("{name:>16}");
        }
        println!();
        for beta in CANDIDATES {
            print!("{beta:<16.1}");
            for (_, from, to) in cases {
                let ms = change_latency_ms(from, to, beta);
                if ms.is_finite() {
                    print!("{ms:>16.0}");
                } else {
                    print!("{:>16}", "never");
                }
            }
            println!();
        }
    }

    #[test]
    #[ignore = "needs testdata/*.wav — run with --ignored --nocapture"]
    fn salience_beta_sweep() {
        println!("\n=== SALIENCE_BETA sweep — % of frames on a real note / an octave off ===");
        for (name, lo, hi, must_remain) in TAKES {
            let band = lo..=hi;
            println!("\n--- {name}  (truth: MIDI {lo}..={hi}) ---");
            let report = |label: String, verdicts: &[Option<f32>]| {
                let (inside, octave) = tally(verdicts, band.clone());
                print!("  {label:<20}: in {inside:>5.1}%   octave-off {octave:>5.1}%");
                for (note_label, midi) in must_remain {
                    print!("   {note_label} {:>5.1}%", on_note(verdicts, *midi));
                }
                println!();
            };
            report("argmax (no Viterbi)".to_string(), &argmax_take(name));
            for beta in CANDIDATES {
                report(format!("beta {beta:.1}"), &decode_take(name, beta));
            }
        }
        println!(
            "\nNOTE: `g_open_real_octave` counts the played G4 as INSIDE (band 55..=67) — a \
             decoder that 'fixes' octaves by shoving everything down reads 100% there AND on \
             the strokes. The strokes' band is 55..=55, so the contrast is what convicts."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::analysis_math::SPIRAL_BINS_PER_SEMITONE;
    use crate::audio::dsp::swipe::SwipeKernel;

    /// REGRESSION, and it was total: **the bank's span is a slider, and the decoder assumed
    /// it was the default.**
    ///
    /// `ResonatorSettings::max_midi` is user-facing — the bank is a per-sample IIR, so its
    /// span is a direct CPU dial "kept user-adjustable for weak devices". The salience curve
    /// is therefore only as long as *that* bank. The trellis meanwhile built its grid from
    /// `TRACKED_MIN_MIDI..TRACKED_MAX_MIDI` unconditionally and read the curve at
    /// `offset + state`.
    ///
    /// At the C8 default the last state lands on the curve's last bin — it fitted by
    /// **exactly nothing**, so every test and every default run passed and the bug was
    /// invisible. On the ceiling this test uses (C6, which is what was actually in the
    /// user's persisted settings) it read 192 bins past the end and panicked **on the audio
    /// thread**. That kills the bank worker, so the staff and the pitch roll just go empty:
    /// no error, no log, nothing to see. The report was "вообще нот нет".
    ///
    /// The lesson is the one this project keeps relearning from the other side: a user knob
    /// reached the detector. Here it did not bend the decision — it deleted it.
    #[test]
    fn a_bank_narrower_than_the_pitch_domain_decodes_instead_of_panicking() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        // C0..C6 — verbatim from ~/.local/share/fretboard/app.ron when this was reported.
        let (min_midi, max_midi) = (12.0f32, 84.0f32);
        let kernel = SwipeKernel::new(bps);

        // A4 with partials, so there is a genuine harmonic series inside the narrow bank.
        let len = ((max_midi - min_midi) * bps) as usize + 1;
        let mut column = vec![0.0f32; len];
        for (index, amplitude) in [1.0f32, 0.8, 0.6, 0.35].into_iter().enumerate() {
            let hz = 440.0 * (index + 1) as f32;
            let midi = 69.0 + 12.0 * (hz / 440.0).log2();
            let bin = ((midi - min_midi) * bps).round() as usize;
            if bin < len {
                column[bin] += amplitude;
            }
        }

        let frame = SalienceFrame::score(&column, min_midi, bps, &kernel).unwrap();
        let mut decoder = SalienceDecoder::default();
        let decoded = decoder.decode(&frame, 0.0);

        let (midi, _) = decoded.expect("a narrow bank must still decode the note inside it");
        assert!(
            (midi - 69.0).abs() < 0.5,
            "decoded {midi} on a C0..C6 bank; A4 is inside it and should be found"
        );
    }

    /// The bank's fine pitch (and therefore its cents) survives a snap untouched —
    /// the snap only ever moves whole octaves.
    #[test]
    fn snap_preserves_fine_pitch() {
        let mut m = MelodyTracker::default();
        // Bank hears A4 17 cents sharp but an octave up; anchor says A4.
        let (midi, _) = m.pin_and_gate(Some((81.17, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!((midi - 69.17).abs() < 1e-4, "snapped to {midi}, expected 69.17");
    }

    /// The bank's octave wandering — the failure Phase 1.3 saw live — is corrected
    /// when pYIN is confident and agrees on the pitch class.
    #[test]
    fn snap_fixes_bank_octave_wandering() {
        // Bank crowned the 2nd harmonic: A5 instead of A4.
        let mut m = MelodyTracker::default();
        let (midi, _) = m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 69.0).abs() < 1e-4,
            "expected the anchor's octave, got {midi}"
        );
        // Bank fell to the sub-octave: A3 instead of A4.
        let mut m = MelodyTracker::default();
        let (midi, _) = m.pin_and_gate(Some((57.0, 0.9)), Some((69.0, 0.9))).unwrap();
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
            let (midi, _) = m.pin_and_gate(Some((bank, 0.9)), Some((69.0, 0.9))).unwrap();
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
        let (midi, _) = m.pin_and_gate(Some((76.0, 0.9)), Some((69.0, 0.9))).unwrap();
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
            m.pin_and_gate(Some((69.0, 0.9)), Some((69.0, 0.9)));
        }
        // Player leaps to A5. The anchor stays on A4 for ~128 ms.
        let mut midi = 0.0;
        for _ in 0..=LEAP_CONFIRM_FRAMES {
            midi = m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap().0;
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
            m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9)));
        }
        for i in 0..8 {
            let (midi, _) = m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
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
            m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))); // leap, anchor stale
        }
        m.pin_and_gate(Some((81.0, 0.9)), Some((81.0, 0.9))); // anchor arrives → agreement
        // Now the bank wanders down an octave; the anchor must fix it at once.
        let (midi, _) = m.pin_and_gate(Some((69.0, 0.9)), Some((81.0, 0.9))).unwrap();
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
            m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))); // build an unbroken dispute
        }
        m.pin_and_gate(None, Some((69.0, 0.9)));
        // A fresh phrase whose bank octave is wrong must be corrected immediately,
        // not inherit the previous phrase's dispute and be believed.
        let (midi, _) = m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.9))).unwrap();
        assert!(
            (midi - 69.0).abs() < 1e-4,
            "dispute leaked across silence, got {midi}"
        );
    }

    /// REGRESSION: with **no** anchor to snap against, a lone slip must still be
    /// rejected outright rather than passed on as a wrong note.
    ///
    /// This is layer 3's whole reason to exist: layer 2 stands down exactly when pYIN
    /// has no opinion, and that is when a bank slip would otherwise reach the panels.
    /// It must come back as `None` — the same as silence — because downstream a gap is
    /// absorbed by the staff's release grace while a wrong-octave frame tears the held
    /// note in two and restarts its timer.
    #[test]
    fn lone_slip_is_a_gap_not_a_wrong_note() {
        let mut m = MelodyTracker::default();
        for _ in 0..5 {
            assert!(m.pin_and_gate(Some((69.0, 0.9)), None).is_some()); // establish A4
        }
        assert!(
            m.pin_and_gate(Some((81.0, 0.9)), None).is_none(),
            "a lone slip must be dropped, not passed on"
        );
        assert!(
            m.pin_and_gate(Some((71.0, 0.9)), None).is_some(),
            "a trill-sized interval must still pass"
        );
    }

    /// An unconfident anchor has no opinion: the bank keeps its own octave rather
    /// than being snapped to a guess made on an unvoiced frame.
    #[test]
    fn unconfident_anchor_is_ignored() {
        let mut m = MelodyTracker::default();
        let (midi, _) = m.pin_and_gate(Some((81.0, 0.9)), Some((69.0, 0.1))).unwrap();
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
        assert!(m.pin_and_gate(None, Some((69.0, 0.9))).is_none());
    }

    /// With no anchor at all (no reading yet) the bank passes through.
    #[test]
    fn bank_passes_through_without_an_anchor() {
        let mut m = MelodyTracker::default();
        let (midi, strength) = m.pin_and_gate(Some((69.5, 0.7)), None).unwrap();
        assert!((midi - 69.5).abs() < 1e-4);
        assert!((strength - 0.7).abs() < 1e-4);
    }
}
