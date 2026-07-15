//! The continuity model: an online Viterbi over a pitch grid, shared by both channels.
//!
//! A per-frame pitch decision is a coin toss whenever two candidates come within a few
//! percent of each other, and no threshold can fix that — the tie is in the evidence, not
//! in the rule reading it. What breaks a tie is the one thing a single frame does not
//! have: **what came before it**. That is this module. Give it a likelihood over pitch
//! each frame and it decodes the most likely *path*, where holding a note is cheap and
//! moving is not, so a lone outlier cannot move the line but sustained evidence can.
//!
//! # Why this is one trellis and not two
//!
//! [`super::pyin`] has had an honest online Viterbi since Phase 1.4 — but on the **128 ms
//! window**, where the octave never broke in the first place ("rock steady" on a sustain,
//! by its own measurement). The fast channel that actually draws the staff and the pitch
//! roll — the resonator bank at ~16 ms — had no probabilistic model **at all**, and that
//! is exactly where the ties live. The model was on the wrong channel.
//!
//! So the trellis is lifted out of `pyin` rather than reimplemented beside it: the two
//! channels differ only in *what they observe* (`pyin` integrates YIN's threshold prior
//! into a handful of candidates; the fast channel hands over SWIPE′'s whole salience
//! curve), never in *how pitch moves over time*. A player's fingers do not know which
//! detector is watching. Two copies of that model would be two things to keep in sync and
//! one more place for a tuning constant to drift.
//!
//! # Rates, not per-frame probabilities — and why that is not a flourish
//!
//! Every constant here is a **rate per second**, discretized against the frame's actual
//! elapsed time. The original kernel's constants were per *frame*, which is only
//! well-defined while every consumer shares one cadence — and they do not: `pyin` runs at
//! `core::ANALYSIS_INTERVAL` (40 ms), while the bank publishes every
//! `ResonatorSettings::update_ms`, a **user-facing slider spanning 8..80 ms** (it sits next
//! to "History" in the waterfall's controls, `app::controls`). A per-frame `SELF_STAY` of
//! 0.8 means "80% chance of holding for 8 ms" at one end of that slider and "80% chance of
//! holding for 80 ms" at the other: a 10× swing in the detector's smoothing, driven by a
//! display knob.
//!
//! That is the **fourth** time a display setting has reached into this detector — after the
//! silence gate sharing the UI meter's smoothing (1.9), `gamma` (a waterfall-contrast
//! slider) deciding the octave (1.11), and the waterfall's C0..C8 extent choosing the
//! candidates (`35db82e`). The rule those three wrote is that a detector's timescale is a
//! property of the **task**, never of the picture — so this one is closed by construction
//! rather than by a comment asking the next person not to touch it. See
//! `memory/display_settings_must_not_steer_the_detector.md`.
//!
//! The discretization is a jump process: events (glides, leaps, voicing switches) arrive as
//! independent Poisson processes, so the chance of one landing in a frame of length `dt` is
//! `1 − exp(−rate·dt)`. Aggregate behaviour is then **dt-invariant**: halve the frame
//! length and you get twice as many frames, each half as likely to move — the expected
//! number of leaps per second, and the diffusion per second, come out (almost) identical.
//! Almost, because a discrete-time chain allows at most one event per frame, so a long
//! frame under-counts: measured at ×1.21 across the slider's whole span, against the ×10.0
//! the per-frame constants swing by. Both numbers come out of
//! [`tests::the_rates_are_dt_invariant_where_the_per_frame_constants_were_not`], which
//! measures the old kernel on the same sweep rather than taking the new one's word for it.

/// Which bin of a [`PitchGrid`] the trellis is in.
///
/// A newtype because this module traffics in **two** index domains that are trivially
/// confusable and catastrophic to mix: a state here is an index into *this* grid, which
/// starts at the app's `pitch::TRACKED_MIN_MIDI` (C1), while the salience curve feeding the
/// fast channel is indexed on the *bank's output grid*, which starts an octave lower at C0
/// because that is what the waterfall draws. Confusing the two is not a crash — it is a
/// silent transposition, which is the exact class of bug this whole phase exists to kill.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PitchState(pub(crate) usize);

/// What the trellis's states mean: a log-frequency grid over a MIDI span.
///
/// Deliberately *not* fixed to pYIN's 10-cent grid. The fast channel observes the bank's
/// own output grid, and resampling a salience curve onto a second grid just to satisfy a
/// constant would blur the very peaks the decision is made from.
#[derive(Clone, Debug)]
pub(crate) struct PitchGrid {
    min_midi:          f32,
    bins_per_semitone: f32,
    n_pitch:           usize,
}

impl PitchGrid {
    /// The grid spanning `min_midi..max_midi` at `bins_per_semitone`.
    ///
    /// Contract: the span must be non-empty and the resolution positive — both are
    /// compile-time-ish facts at every call site (module constants), so a bad grid is a
    /// programming error, not an input to handle.
    pub(crate) fn new(min_midi: f32, max_midi: f32, bins_per_semitone: f32) -> Self {
        assert!(max_midi > min_midi, "empty pitch grid {min_midi}..{max_midi}");
        assert!(
            bins_per_semitone > 0.0,
            "grid resolution {bins_per_semitone} must be positive"
        );
        Self {
            min_midi,
            bins_per_semitone,
            n_pitch: ((max_midi - min_midi) * bins_per_semitone) as usize,
        }
    }

    pub(crate) fn n_pitch(&self) -> usize {
        self.n_pitch
    }

    /// Fractional bin of a MIDI pitch. May fall outside `0..n_pitch` — the caller decides
    /// what an off-grid observation means, because that answer differs by channel.
    pub(crate) fn bin_of_midi(&self, midi: f32) -> f32 {
        (midi - self.min_midi) * self.bins_per_semitone
    }

    /// The MIDI pitch at a state's bin *centre*.
    pub(crate) fn midi_of_state(&self, state: PitchState) -> f32 {
        self.min_midi + state.0 as f32 / self.bins_per_semitone
    }

    /// How far a transition may reach, in bins: ±1 octave. Beyond that a move must go
    /// through the unvoiced state — which for a channel that never goes unvoiced (the
    /// bank's, gated on level upstream) means a leap wider than an octave takes two frames
    /// rather than one. That is not a limitation worth widening: an interval that big
    /// between two adjacent frames is not a violin.
    fn trans_window(&self) -> usize {
        (12.0 * self.bins_per_semitone) as usize
    }
}

// --- Transition rates --------------------------------------------------------
//
// All three are *derived* from the kernel pYIN shipped, not invented here: that kernel's
// per-frame numbers were tuned and measured at the 40 ms `core::ANALYSIS_INTERVAL`, so
// reading them as Poisson rates at that cadence carries the tuning over intact rather than
// restarting it. `kernel_reproduces_pyins_numbers_at_its_own_cadence` pins the round trip,
// and it is what makes this a *generalization* of the old kernel rather than a new one
// wearing its name.

/// Rate at which the pitch *does something* — glides or leaps — rather than holding.
///
/// From `SELF_STAY = 0.8` per 40 ms frame: an 80% chance of no event in 40 ms is a Poisson
/// rate of `−ln(0.8)/0.040 ≈ 5.58` events/s.
///
/// The mass that does **not** move must dominate, and that requirement survives the change
/// of units: a voiced state holds its ground against the (nearly free) unvoiced self-loop
/// only while staying is cheaper than moving. Raise this far enough and the tracker never
/// commits to anything.
const EVENT_RATE_HZ: f32 = 5.578_6;

/// Of the events above, the share that is a **leap** rather than a glide — pYIN's
/// `LEAP_MASS = 0.02` against `1 − SELF_STAY − LEAP_MASS = 0.18` of glide, i.e. 10%.
///
/// A leap is spread *uniformly* over the whole ±octave window rather than left to the
/// Gaussian's tail, and that is load-bearing: on a Gaussian of 70 cents a fifth sits 10σ
/// out at `exp(−50)`, so before this mass existed the only thing keeping a leap's cost
/// finite was a numerical floor. The tracker could then not follow a *legato* leap (which
/// never goes unvoiced, so the unvoiced escape hatch is emission-blocked) until the old
/// note had left the analysis window entirely — measured at +120 ms on every octave.
///
/// Uniform mass makes a leap of *any* interval cost the same bounded ~9 nats. That single
/// number is what decides both halves of what this phase is for: 9 nats is far more than a
/// 4–6% emission tie can pay (it would need to hold for seconds), and far less than a
/// genuinely re-fingered note can pay (its evidence swings by nats *per frame*). The
/// difference between a phantom octave and a real one was never the interval — it is
/// whether the evidence *persists*, and that is a property of the path, not of a rule.
const LEAP_SHARE: f32 = 0.1;

/// Rate of switching between voiced and unvoiced. From `VOICING_SWITCH = 0.02` per 40 ms:
/// `−ln(1 − 0.02)/0.040 ≈ 0.505`/s. Small — notes persist — but a real gap can switch.
const VOICING_SWITCH_RATE_HZ: f32 = 0.505_08;

/// Gaussian width (cents) of a **glide** event's displacement: vibrato, portamento, the
/// pitch moving under a finger.
///
/// This one is a property of the *event*, not of the frame, so — unlike the rates above —
/// it does **not** scale with `dt`, and that is deliberate rather than an oversight. The
/// aggregate is what has to stay dt-invariant, and it already is: with glide events
/// arriving at `λ_g` per second and each displacing by `σ`, the variance accumulated per
/// second is `λ_g·σ²` no matter how the frames are cut. Scaling `σ` with `dt` *as well*
/// would make the diffusion depend on the frame length quadratically — reintroducing the
/// exact bug this module exists to close, in a subtler place.
const GLIDE_SIGMA_CENTS: f32 = 70.0;

/// Emission floor, so no state ever carries a log-probability of −∞ (which would poison the
/// renormalization for every frame after it).
const EMIT_EPS: f32 = 1e-9;

/// One frame's likelihood over the states — what the trellis observes.
///
/// Linear scale, **not** required to be normalized: the Viterbi's argmax is invariant to a
/// per-frame constant factor, so a caller that has an unnormalized score (a salience curve,
/// say) may hand it over as-is. What a caller must **not** do is let the scale drift *between*
/// frames for reasons unrelated to the evidence, since that would silently re-weight the
/// emission against the (fixed) transition costs.
#[derive(Clone, Debug)]
pub(crate) struct Emissions {
    /// Likelihood per pitch state; length `grid.n_pitch()`.
    pitch:    Vec<f32>,
    /// Likelihood that this frame is not a pitch at all.
    unvoiced: f32,
}

impl Emissions {
    /// All-zero emissions (floored to [`EMIT_EPS`] on use) for `grid`.
    pub(crate) fn zeroed(grid: &PitchGrid) -> Self {
        Self {
            pitch:    vec![0.0; grid.n_pitch()],
            unvoiced: 0.0,
        }
    }

    /// Deposit `mass` at a fractional bin, split over the nearest bin and half to each
    /// neighbour — a candidate is a point estimate with grid-scale uncertainty, and a
    /// single-bin spike would make the decode brittle to which side of a boundary it
    /// landed. Off-grid bins are silently dropped: they are pitches this app does not
    /// claim to hear.
    pub(crate) fn add_at_bin(&mut self, bin: f32, mass: f32) {
        let center = bin.round() as isize;
        for offset in -1..=1isize {
            let b = center + offset;
            if b < 0 || b >= self.pitch.len() as isize {
                continue;
            }
            let weight = if offset == 0 { 1.0 } else { 0.5 };
            self.pitch[b as usize] += mass * weight;
        }
    }

    /// Set the likelihood of a state outright — for a caller whose evidence *is* a value
    /// per bin (a salience curve) rather than a list of candidates.
    pub(crate) fn set_state(&mut self, state: PitchState, likelihood: f32) {
        self.pitch[state.0] = likelihood;
    }

    pub(crate) fn set_unvoiced(&mut self, likelihood: f32) {
        self.unvoiced = likelihood;
    }
}

/// A stateful online Viterbi: one per pitch channel, carrying its forward trellis frame to
/// frame.
///
/// Decoded **greedily** — the argmax of the forward trellis each frame, rather than a
/// backward pass over a buffered segment. That is a real approximation and it is chosen on
/// purpose: full Viterbi needs the future, and the whole point of the fast channel is that
/// there isn't any. The greedy path can differ from the optimal one, but only while the
/// evidence is genuinely ambiguous — which is precisely when no answer is available yet at
/// any latency.
#[derive(Debug)]
pub(crate) struct PitchTrellis {
    grid:        PitchGrid,
    /// log-prob of the best path ending in each state, renormalized each frame; length
    /// `n_pitch + 1`, last entry = unvoiced.
    delta:       Vec<f32>,
    /// log of the normalized pitch→pitch kernel, indexed `offset + trans_window`.
    log_kernel:  Vec<f32>,
    /// The frame length `log_kernel` was cut for. A kernel belongs to a `dt`; a different
    /// `dt` gets a different kernel (see [`Self::step`]).
    kernel_dt:   f32,
    initialized: bool,
}

impl PitchTrellis {
    /// A trellis over `grid`, with its kernel built for a nominal frame length of `dt`
    /// seconds. A caller whose cadence is fixed (pYIN's is) never pays to rebuild it; one
    /// whose cadence jitters gets a fresh kernel per frame, which is ~200 transcendentals
    /// against the decode's ~130k operations.
    pub(crate) fn new(grid: PitchGrid, dt: f32) -> Self {
        let log_kernel = build_log_kernel(&grid, dt);
        let delta = vec![0.0; grid.n_pitch() + 1];
        Self {
            grid,
            delta,
            log_kernel,
            kernel_dt: dt,
            initialized: false,
        }
    }

    pub(crate) fn grid(&self) -> &PitchGrid {
        &self.grid
    }

    /// Forget the path. The next frame is decided by its emissions alone, as if it were the
    /// first — which is what a caller wants at a phrase boundary, where continuity with
    /// what came before is not evidence but contamination.
    pub(crate) fn reset(&mut self) {
        self.initialized = false;
    }

    /// Advance one frame over `dt` seconds and read off the current best state; `None` when
    /// the decoded state is unvoiced.
    ///
    /// Contract: `dt` is the time *this frame covers*, from a clock that measures the
    /// **signal** rather than the wall or the renderer. The bank's caller takes it from the
    /// sample count for exactly that reason.
    pub(crate) fn step(&mut self, emissions: &Emissions, dt: f32) -> Option<PitchState> {
        assert_eq!(
            emissions.pitch.len(),
            self.grid.n_pitch(),
            "emissions were built for a different grid than this trellis decodes"
        );
        if dt != self.kernel_dt {
            self.log_kernel = build_log_kernel(&self.grid, dt);
            self.kernel_dt = dt;
        }

        let n_pitch = self.grid.n_pitch();
        let window = self.grid.trans_window();
        let uv = n_pitch;

        let log_emit: Vec<f32> = emissions
            .pitch
            .iter()
            .chain(std::iter::once(&emissions.unvoiced))
            .map(|e| e.max(EMIT_EPS).ln())
            .collect();

        if !self.initialized {
            // Uniform prior over states → the first frame's path is just its emission.
            self.delta.copy_from_slice(&log_emit);
            self.initialized = true;
        } else {
            let voicing_switch = 1.0 - (-VOICING_SWITCH_RATE_HZ * dt).exp();
            let log_stay_voiced = (1.0 - voicing_switch).ln();
            let log_p2uv = voicing_switch.ln();
            let log_uv2uv = (1.0 - voicing_switch).ln();
            // Unvoiced re-enters pitch uniformly — the escape hatch that lets a genuine leap
            // *across a note gap* track at once, without paying the leap's own cost.
            let log_uv2p = (voicing_switch / n_pitch as f32).ln();

            let max_pitch_delta = self.delta[..n_pitch]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);

            let mut next = vec![f32::NEG_INFINITY; n_pitch + 1];
            for j in 0..n_pitch {
                let lo = j.saturating_sub(window);
                let hi = (j + window).min(n_pitch - 1);
                let mut best = f32::NEG_INFINITY;
                for i in lo..=hi {
                    let offset = i as isize - j as isize + window as isize;
                    let v = self.delta[i] + log_stay_voiced + self.log_kernel[offset as usize];
                    if v > best {
                        best = v;
                    }
                }
                // …or arrive from unvoiced.
                let from_uv = self.delta[uv] + log_uv2p;
                if from_uv > best {
                    best = from_uv;
                }
                next[j] = best + log_emit[j];
            }
            // Unvoiced: stay unvoiced, or the best pitch decides to drop out.
            next[uv] = (self.delta[uv] + log_uv2uv).max(max_pitch_delta + log_p2uv) + log_emit[uv];

            self.delta = next;
        }

        // Renormalize (subtract the max) to keep the log-probs bounded over time. Only the
        // differences matter to an argmax, so this is free of consequence and the alternative
        // is a slow drift to −∞.
        let m = self.delta.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if m.is_finite() {
            for d in &mut self.delta {
                *d -= m;
            }
        }

        let best_state = self
            .delta
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        (best_state != uv).then_some(PitchState(best_state))
    }
}

/// The pitch→pitch transition kernel over offsets −W..=W, as a proper distribution summing
/// to 1, for a frame of `dt` seconds.
///
/// Three components, modelling three different things a player does:
///
///   hold   — no event at all. Must dominate: diagonal dominance is what lets a voiced
///            state hold against the (nearly free) unvoiced self-loop, which otherwise
///            out-races every pitch and the tracker never commits to anything.
///   glide  — vibrato/portamento, a Gaussian displacement of [`GLIDE_SIGMA_CENTS`].
///   leap   — a jump to any other pitch, uniform over the window. See [`LEAP_SHARE`] for
///            why uniform rather than the Gaussian's tail.
fn build_log_kernel(grid: &PitchGrid, dt: f32) -> Vec<f32> {
    // The jump-process discretization: the chance that *any* event lands in this frame.
    let hold = (-EVENT_RATE_HZ * dt).exp();
    let event = 1.0 - hold;
    kernel_with_masses(grid, hold, event * (1.0 - LEAP_SHARE), event * LEAP_SHARE)
}

/// The kernel's shape, given how the three masses are split.
///
/// Split out from [`build_log_kernel`] so a test can build the **per-frame** kernel this
/// module replaced — same shape, masses that ignore `dt` — and measure the two on the same
/// sweep. Without that, "the rates are dt-invariant" is a claim checked only against itself.
fn kernel_with_masses(grid: &PitchGrid, hold: f32, glide_mass: f32, leap_mass: f32) -> Vec<f32> {
    let window = grid.trans_window();
    let w = window as isize;
    let n_offsets = (2 * window) as f32; // every offset except the diagonal

    let cents_per_bin = 100.0 / grid.bins_per_semitone;
    let mut kernel = vec![0.0f32; 2 * window + 1];
    let mut glide_sum = 0.0f32;
    for off in -w..=w {
        if off == 0 {
            continue;
        }
        let cents = off as f32 * cents_per_bin;
        let g = (-0.5 * (cents / GLIDE_SIGMA_CENTS).powi(2)).exp();
        kernel[(off + w) as usize] = g;
        glide_sum += g;
    }
    for (i, k) in kernel.iter_mut().enumerate() {
        if i == window {
            *k = hold;
        } else {
            *k = glide_mass * *k / glide_sum + leap_mass / n_offsets;
        }
    }
    kernel.iter().map(|k| k.ln()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cadence pYIN's per-frame constants were tuned at (`core::ANALYSIS_INTERVAL`), and
    /// therefore the one where the rates here must reproduce them exactly.
    const CALIBRATION_DT: f32 = 0.040;

    /// pYIN's grid, so the calibration tests speak in the numbers the old kernel used.
    fn pyin_grid() -> PitchGrid {
        PitchGrid::new(24.0, 108.0, 10.0)
    }

    /// **The calibration.** This module claims to *generalize* pYIN's kernel rather than
    /// replace it, and that claim is only worth anything if the generalization reproduces
    /// the original where the original was tuned: at the 40 ms `ANALYSIS_INTERVAL`, the
    /// rates must come back out as `SELF_STAY = 0.8`, glide mass `0.18`, `LEAP_MASS = 0.02`.
    ///
    /// If this fails, every number pYIN measured — its latency table, its octave
    /// behaviour — was measured on a kernel that no longer exists.
    #[test]
    fn kernel_reproduces_pyins_numbers_at_its_own_cadence() {
        let grid = pyin_grid();
        let window = grid.trans_window();
        let log_kernel = build_log_kernel(&grid, CALIBRATION_DT);
        let kernel: Vec<f32> = log_kernel.iter().map(|k| k.exp()).collect();

        let self_stay = kernel[window];
        assert!(
            (self_stay - 0.8).abs() < 1e-3,
            "SELF_STAY at 40 ms came out {self_stay:.4}, pYIN's kernel had 0.800"
        );

        // The leap floor: the far edge of the window is pure leap mass, since the Gaussian
        // is `exp(−0.5·(1200/70)²)` there — zero to every float that matters.
        let leap_per_offset = kernel[0];
        let leap_mass = leap_per_offset * (2 * window) as f32;
        assert!(
            (leap_mass - 0.02).abs() < 1e-3,
            "LEAP_MASS at 40 ms came out {leap_mass:.4}, pYIN's kernel had 0.020"
        );

        let total: f32 = kernel.iter().sum();
        assert!(
            (total - 1.0).abs() < 1e-3,
            "kernel is not a distribution: sums to {total}"
        );
        let glide_mass = total - self_stay - leap_mass;
        assert!(
            (glide_mass - 0.18).abs() < 1e-3,
            "glide mass at 40 ms came out {glide_mass:.4}, pYIN's kernel had 0.180"
        );
    }

    /// The `update_ms` slider's full span — the range the detector must not notice.
    const SLIDER_SPAN_SECONDS: [f32; 4] = [0.008, 0.016, 0.040, 0.080];

    /// What one kernel does per **second**: how many leaps it injects, and how much
    /// diffusion. Both are per-frame quantities scaled by the frame rate, which is exactly
    /// the conversion that makes two cadences comparable.
    fn per_second_behaviour(grid: &PitchGrid, log_kernel: &[f32], dt: f32) -> (f32, f32) {
        let window = grid.trans_window();
        let kernel: Vec<f32> = log_kernel.iter().map(|k| k.exp()).collect();
        let frames_per_second = 1.0 / dt;

        // Leap mass reads off the window's far edge, where the Gaussian is exactly 0
        // (`exp(−0.5·(1200/70)²)`), so that bin is pure uniform leap.
        let leap_mass = kernel[0] * (2 * window) as f32;

        // Second moment of the displacement, in cents² — the diffusion injected per frame.
        let cents_per_bin = 100.0 / grid.bins_per_semitone;
        let variance: f32 = kernel
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let cents = (i as isize - window as isize) as f32 * cents_per_bin;
                p * cents * cents
            })
            .sum();
        (leap_mass * frames_per_second, variance * frames_per_second)
    }

    fn spread(v: &[f32]) -> f32 {
        let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min = v.iter().copied().fold(f32::INFINITY, f32::min);
        max / min
    }

    /// **The point of the rewrite, and its non-circular half.**
    ///
    /// The frame length is a display slider (8..80 ms), so the detector's behaviour must not
    /// depend on it. Per *frame* it necessarily does — a longer frame is likelier to contain
    /// an event. What must hold is that the aggregate per **second** does not.
    ///
    /// So both kernels run the same sweep: the rate-based one, and the **per-frame** one it
    /// replaces (pYIN's shipped `SELF_STAY = 0.8`/`0.18`/`LEAP_MASS = 0.02`, applied
    /// regardless of `dt`). A new kernel passing its own test proves nothing until the thing
    /// it replaces is shown to fail it on the same input — the rule
    /// `swipe::the_old_comb_fails_the_same_column` was written under.
    ///
    /// The rate-based kernel is **not** perfectly flat, and the residual is honest rather
    /// than sloppy: a discrete-time chain allows at most one event per frame, so a long
    /// frame under-counts (two leaps in 80 ms look like one). That curvature is bounded and
    /// small; the per-frame kernel's error is neither.
    #[test]
    fn the_rates_are_dt_invariant_where_the_per_frame_constants_were_not() {
        let grid = pyin_grid();

        let mut rate_leaps = Vec::new();
        let mut rate_diffusion = Vec::new();
        let mut fixed_leaps = Vec::new();
        let mut fixed_diffusion = Vec::new();
        for dt in SLIDER_SPAN_SECONDS {
            let (leaps, diffusion) = per_second_behaviour(&grid, &build_log_kernel(&grid, dt), dt);
            rate_leaps.push(leaps);
            rate_diffusion.push(diffusion);

            // The kernel this module replaced: the same shape, with pYIN's per-frame masses
            // nailed on at every cadence.
            let fixed = kernel_with_masses(&grid, 0.8, 0.18, 0.02);
            let (leaps, diffusion) = per_second_behaviour(&grid, &fixed, dt);
            fixed_leaps.push(leaps);
            fixed_diffusion.push(diffusion);
        }

        println!("\n=== behaviour per second across the `update_ms` slider (8..80 ms) ===");
        println!(
            "  rates  : {rate_leaps:?} leaps/s  (×{:.2} across the span)",
            spread(&rate_leaps)
        );
        println!(
            "  frames : {fixed_leaps:?} leaps/s  (×{:.1} across the span)",
            spread(&fixed_leaps)
        );

        // The residual: `(1 − exp(−λ·dt))/dt` sags as `dt` grows. Over a 10× span that is
        // ~20%, i.e. ~0.2 nats out of the leap's ~9.9 — it cannot flip a decision that the
        // 4–6% emission ties (0.04 nats) already lose by two orders of magnitude.
        assert!(
            spread(&rate_leaps) < 1.25,
            "leap rate should barely move across the slider: {rate_leaps:?} leaps/s"
        );
        assert!(
            spread(&rate_diffusion) < 1.25,
            "diffusion should barely move across the slider: {rate_diffusion:?} cents²/s"
        );

        // …and the oracle: the per-frame constants swing by the slider's full 10×, because
        // "per frame" simply means "per whatever the user dragged it to". If this ever stops
        // failing, this test has stopped measuring anything.
        assert!(
            spread(&fixed_leaps) > 9.0,
            "the per-frame kernel was supposed to be the broken one, but its leap rate held \
             at {fixed_leaps:?} — then the premise of this rewrite is wrong"
        );
        assert!(
            spread(&fixed_diffusion) > 9.0,
            "the per-frame kernel's diffusion held at {fixed_diffusion:?} across a 10× span"
        );
    }

    /// A steady note plus one outlier frame: the outlier must not move the decode. This is
    /// the property the whole module exists for, asserted on the trellis itself rather than
    /// through any front end.
    #[test]
    fn a_lone_outlier_does_not_move_the_path() {
        let grid = pyin_grid();
        let a4 = PitchState(grid.bin_of_midi(69.0) as usize);
        let a5 = PitchState(grid.bin_of_midi(81.0) as usize);
        let mut trellis = PitchTrellis::new(grid, CALIBRATION_DT);

        let mut steady = Emissions::zeroed(trellis.grid());
        steady.set_state(a4, 0.9);
        for _ in 0..8 {
            trellis.step(&steady, CALIBRATION_DT);
        }
        // The outlier frame: the octave up momentarily *wins*, but the true pitch is still
        // there as weaker evidence — exactly the 4–6% tie measured on the real violin.
        let mut outlier = Emissions::zeroed(trellis.grid());
        outlier.set_state(a5, 0.40);
        outlier.set_state(a4, 0.38);
        let decoded = trellis.step(&outlier, CALIBRATION_DT).unwrap();
        assert_eq!(decoded, a4, "a 5% tie moved the path an octave");
    }

    /// …but sustained evidence *does* move it, and this is not the same assertion inverted:
    /// it is the control that stops the module from being a note-freezer. A tracker that
    /// only ever passed the test above would pass it by refusing to move at all.
    #[test]
    fn sustained_evidence_moves_the_path() {
        let grid = pyin_grid();
        let a4 = PitchState(grid.bin_of_midi(69.0) as usize);
        let a5 = PitchState(grid.bin_of_midi(81.0) as usize);
        let mut trellis = PitchTrellis::new(grid, CALIBRATION_DT);

        let mut steady = Emissions::zeroed(trellis.grid());
        steady.set_state(a4, 0.9);
        for _ in 0..8 {
            trellis.step(&steady, CALIBRATION_DT);
        }
        // A real octave leap: the old note's evidence *leaves*. That is what separates it
        // from the tie above — not the interval, which is identical.
        let mut leapt = Emissions::zeroed(trellis.grid());
        leapt.set_state(a5, 0.9);
        leapt.set_state(a4, 0.01);
        let mut decoded = None;
        for _ in 0..40 {
            decoded = trellis.step(&leapt, CALIBRATION_DT);
        }
        assert_eq!(decoded.unwrap(), a5, "a sustained octave never tracked");
    }

    /// How *fast* a real leap is followed, in frames — the price of the leap's ~9 nats,
    /// reported rather than asserted because it is a consequence of the rates, not a
    /// tuning target. If this ever runs to tens of frames the fast channel has no business
    /// calling itself fast.
    #[test]
    fn leap_latency_probe() {
        for dt in [0.008f32, 0.016, 0.040] {
            let grid = PitchGrid::new(24.0, 108.0, 8.0);
            let a4 = PitchState(grid.bin_of_midi(69.0) as usize);
            let a5 = PitchState(grid.bin_of_midi(81.0) as usize);
            let mut trellis = PitchTrellis::new(grid, dt);

            let mut steady = Emissions::zeroed(trellis.grid());
            steady.set_state(a4, 0.9);
            for _ in 0..16 {
                trellis.step(&steady, dt);
            }
            let mut leapt = Emissions::zeroed(trellis.grid());
            leapt.set_state(a5, 0.9);
            leapt.set_state(a4, 0.01);
            let mut frames = 0;
            for i in 1..=64 {
                if trellis.step(&leapt, dt) == Some(a5) {
                    frames = i;
                    break;
                }
            }
            println!(
                "dt {:>5.1} ms -> real octave followed after {frames} frames ({:.1} ms)",
                dt * 1000.0,
                frames as f32 * dt * 1000.0
            );
        }
    }

    /// A reset forgets the path: the frame after it is decided by its emissions alone. This
    /// is what a phrase boundary needs — the previous note's continuity is contamination,
    /// not evidence.
    #[test]
    fn reset_forgets_the_path() {
        let grid = pyin_grid();
        let a4 = PitchState(grid.bin_of_midi(69.0) as usize);
        let a5 = PitchState(grid.bin_of_midi(81.0) as usize);
        let mut trellis = PitchTrellis::new(grid, CALIBRATION_DT);

        let mut steady = Emissions::zeroed(trellis.grid());
        steady.set_state(a4, 0.9);
        for _ in 0..8 {
            trellis.step(&steady, CALIBRATION_DT);
        }
        let mut tie = Emissions::zeroed(trellis.grid());
        tie.set_state(a5, 0.40);
        tie.set_state(a4, 0.38);
        // Without a reset the path holds A4 (the test above); with one, the tie's own
        // argmax wins because there is no path left to hold.
        trellis.reset();
        assert_eq!(trellis.step(&tie, CALIBRATION_DT).unwrap(), a5);
    }

    /// Emissions of zero everywhere + unvoiced mass decodes as unvoiced.
    #[test]
    fn unvoiced_decodes_as_none() {
        let grid = pyin_grid();
        let mut trellis = PitchTrellis::new(grid, CALIBRATION_DT);
        let mut silence = Emissions::zeroed(trellis.grid());
        silence.set_unvoiced(1.0);
        assert!(trellis.step(&silence, CALIBRATION_DT).is_none());
    }

    /// The grid's two directions round-trip, so a state means the pitch it says it means.
    #[test]
    fn grid_round_trips() {
        let grid = PitchGrid::new(24.0, 108.0, 8.0);
        assert_eq!(grid.n_pitch(), 84 * 8);
        for midi in [24.0f32, 55.0, 69.0, 107.875] {
            let state = PitchState(grid.bin_of_midi(midi).round() as usize);
            let back = grid.midi_of_state(state);
            assert!((back - midi).abs() < 0.07, "{midi} round-tripped to {back}");
        }
    }
}
