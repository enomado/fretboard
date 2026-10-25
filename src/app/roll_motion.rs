//! The live roll's pitch window as a **moving body**: it has velocity, it is pushed
//! toward a target by a spring, it coasts after a fling, and it never accelerates
//! harder than [`Feel::max_accel`] allows.
//!
//! The window is the pair `(center, span)`, each its own 1-D [`Glide`]. They are
//! independent on purpose: a pan moves the center and leaves a resize in flight alone,
//! and a resize can land while the center is still coasting. Positions are the state —
//! `view_lo`/`view_hi` are derived from them, never stored beside them.
//!
//! **Ported, not invented.** The spring and the coast are the kernels of rust_app's
//! inertia engine (`ss_common_gui::ui::zoom_and_inertia`: `spring_step`,
//! `coast_center_step`, `AxisMotion`) — the one the welllog and main_app plots run on,
//! where both have been tuned live and proven (kani/proptest) not to overshoot from rest
//! and to decay without flipping sign. What is new here is the acceleration cap: that
//! engine follows the hand or a goto, both of which *want* to arrive fast; this one
//! follows a pitch detector, whose target can jump an octave between two frames. A
//! critically damped spring meets such a jump with its full `ω²·d` acceleration, and on
//! screen that is the window being yanked — exactly the jolt this module exists to
//! remove. So the spring proposes, and the cap disposes.

use crate::audio::types::BankRange;
use crate::core_types::pitch::Midi;

/// A distance or extent along the pitch axis, in (fractional) semitones.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub(super) struct Semitones(pub f32);

/// A rate along the pitch axis, semitones per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SemitonesPerSec(pub f32);

/// How one glide is allowed to move toward its target.
#[derive(Clone, Copy, Debug)]
struct Feel {
    /// Spring frequency, 1/s. Settle time ≈ 5/ω — the spring's character near the
    /// target, where the cap is not binding.
    omega:     f32,
    /// The most the glide may accelerate **or brake**, semitones/s². Binding on large
    /// leaps: with it, a leap of `d` takes at least `2·√(d/a)` — a trapezoid of speed,
    /// not a kick.
    max_accel: f32,
}

/// The auto framing opening up for a note that does not fit. Fast — the note the player
/// is watching is off screen until it lands — but still a motion, not a cut: a 12-
/// semitone opening (each edge 6) takes ≈ 0.35 s under the cap.
const GROW: Feel = Feel {
    omega:     9.0,
    max_accel: 200.0,
};
/// The auto framing closing down again. Slow and soft: a slightly oversized view costs
/// nothing, a view that breathes in and out costs the eye its place.
const SHRINK: Feel = Feel {
    omega:     2.5,
    max_accel: 25.0,
};
/// The auto framing following the played range up and down.
const FOLLOW: Feel = Feel {
    omega:     6.0,
    max_accel: 120.0,
};

/// Coast decay after a fling, 1/s — rust_app's `OMEGA_COAST` / `OMEGA_COAST_TOUCH`. A
/// finger's fling runs ≈ 2.2× further than a mouse's at the same speed, which is what
/// Android's native fling does and what a thumb expects (rust_app,
/// `decision-coast-touch-law`).
const COAST_OMEGA_POINTER: f32 = 6.0;
const COAST_OMEGA_TOUCH: f32 = 2.7;
/// Below this a coast snaps to rest, semitones/s — no sub-pixel creep on a still roll.
const COAST_STOP: f32 = 0.2;

/// Arrival: closer than this and slower than [`ARRIVE_VEL`], a glide lands exactly on
/// its target and lets it go.
const ARRIVE_POS: f32 = 0.01;
const ARRIVE_VEL: f32 = 0.05;

/// The longest step the integrator takes. The spring step is exact for any `dt`, but
/// the cap and the braking envelope are applied per step, so a long frame (a phone
/// dropping to 20 fps) is cut into short ones — the motion must not depend on the frame
/// rate, and a cap checked once per 50 ms frame would let a whole frame's worth through.
const MAX_SUBSTEP_S: f32 = 1.0 / 240.0;
/// The longest frame we integrate at all. A UI frame after the app was in the
/// background can claim seconds; the window should resume, not teleport.
const MAX_FRAME_S: f32 = 0.1;

/// The narrowest pitch window a hand may zoom to — still a few labelled rows.
const MANUAL_MIN_SPAN: Semitones = Semitones(4.0);

/// One analytic step of a critically damped spring toward `target` (rust_app
/// `inertia::spring_step_with_decay`, same closed form):
///
/// ```text
///   d(t) = (d₀ + (v₀ + ω·d₀)·t)·e^(−ωt),   v(t) = (b − ω·(d₀ + b·t))·e^(−ωt),   b = v₀ + ω·d₀
/// ```
///
/// From rest it never overshoots and closes monotonically, for any `dt`.
fn spring_step(pos: f32, vel: f32, target: f32, omega: f32, dt: f32) -> (f32, f32) {
    let e = (-omega * dt).exp();
    let d = pos - target;
    let b = vel + omega * d;
    let new_d = (d + b * dt) * e;
    let new_vel = (b - omega * (d + b * dt)) * e;
    (target + new_d, new_vel)
}

/// One coordinate of the window — a position with a velocity, and maybe a target.
///
/// `target == None` means nobody is steering it: it coasts on whatever velocity it has
/// (a fling) and comes to rest. The velocity lives through every mode change, which is
/// what makes a fling handed over to the auto framing (double-tap mid-coast) continuous.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Glide {
    pos:    f32,
    vel:    f32,
    target: Option<f32>,
}

impl Glide {
    fn at(pos: f32) -> Self {
        Self {
            pos,
            vel: 0.0,
            target: None,
        }
    }

    /// Advance by `dt` seconds. `feel` is used while steered; `coast_omega` while not.
    fn step(&mut self, dt: f32, feel: Feel, coast_omega: f32) {
        let dt = dt.clamp(0.0, MAX_FRAME_S);
        let steps = (dt / MAX_SUBSTEP_S).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as usize {
            match self.target {
                Some(target) => self.steer(target, h, feel),
                None => self.coast(h, coast_omega),
            }
        }
    }

    /// One step toward `target`: the spring's proposal, with its acceleration capped
    /// and its speed held inside the braking envelope.
    fn steer(&mut self, target: f32, h: f32, feel: Feel) {
        let (spring_pos, spring_vel) = spring_step(self.pos, self.vel, target, feel.omega, h);
        let wanted = (spring_vel - self.vel) / h;
        let accel = wanted.clamp(-feel.max_accel, feel.max_accel);
        let mut vel = self.vel + accel * h;
        // The braking envelope: never approach faster than `max_accel` can stop on the
        // target. Without it a capped spring arrives too fast to brake and overshoots —
        // the cap would have traded a jolt at the start for a wobble at the end.
        let d = target - self.pos;
        let v_stop = (2.0 * feel.max_accel * d.abs()).sqrt();
        let braked = vel * d > 0.0 && vel.abs() > v_stop;
        if braked {
            vel = v_stop.copysign(vel);
        }
        if accel == wanted && !braked {
            // The cap did not bind: take the exact spring step, not its trapezoid.
            self.pos = spring_pos;
            self.vel = spring_vel;
        } else {
            self.pos += 0.5 * (self.vel + vel) * h;
            self.vel = vel;
        }
        let crossed = (target - self.pos) * d < 0.0;
        let near = (target - self.pos).abs() < ARRIVE_POS && self.vel.abs() < ARRIVE_VEL;
        // Crossing at envelope speed means the last step simply overran the target by
        // under one step's travel: that is arriving, not overshooting.
        if near || (crossed && self.vel.abs() <= feel.max_accel * h * 4.0) {
            self.pos = target;
            self.vel = 0.0;
            self.target = None;
        }
    }

    /// One step of the exponential (viscous) coast, rust_app `coast_center_step`.
    fn coast(&mut self, h: f32, omega: f32) {
        if self.vel.abs() < COAST_STOP {
            self.vel = 0.0;
            return;
        }
        let new_vel = self.vel * (-omega * h).exp();
        self.pos += 0.5 * (self.vel + new_vel) * h;
        self.vel = new_vel;
    }
}

/// Who decides the window right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Framing {
    /// The auto framing aims it at what is being played.
    Auto,
    /// A hand moved it. The auto framing stands down until asked back (double-tap),
    /// otherwise it would yank the window from under the finger the moment it lifts.
    Manual,
}

/// What threw the last pan — it decides how far the fling coasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlingHand {
    Pointer,
    Touch,
}

/// The live roll's pitch window — see the module docs.
#[derive(Clone, Debug)]
pub(super) struct PitchWindow {
    /// Fractional MIDI at the middle of the plot.
    center:  Glide,
    /// Semitones from the bottom edge to the top. Always `> 0`.
    span:    Glide,
    framing: Framing,
    hand:    FlingHand,
    /// A hand moved the center this frame. The next [`Self::tick`] must not also coast
    /// it — the hand already put it where it belongs (rust_app's `deltas` register).
    held:    bool,
}

impl PitchWindow {
    /// A window at rest showing `lo..hi`, auto-framed.
    pub(super) fn new(lo: Midi, hi: Midi) -> Self {
        assert!(lo.0 < hi.0, "empty pitch window {lo:?}..{hi:?}");
        Self {
            center:  Glide::at(0.5 * (lo.0 + hi.0)),
            span:    Glide::at(hi.0 - lo.0),
            framing: Framing::Auto,
            hand:    FlingHand::Pointer,
            held:    false,
        }
    }

    /// The pitch at the plot's bottom edge.
    pub(super) fn lo(&self) -> Midi {
        Midi(self.center.pos - 0.5 * self.span.pos)
    }

    /// The pitch at the plot's top edge.
    pub(super) fn hi(&self) -> Midi {
        Midi(self.center.pos + 0.5 * self.span.pos)
    }

    pub(super) fn span(&self) -> Semitones {
        Semitones(self.span.pos)
    }

    pub(super) fn framing(&self) -> Framing {
        self.framing
    }

    /// Where the auto framing wants the window — the glides head there under the caps.
    /// Ignored while a hand has the window: [`Framing::Manual`] is the user's.
    pub(super) fn aim(&mut self, lo: Midi, hi: Midi) {
        assert!(lo.0 < hi.0, "empty pitch target {lo:?}..{hi:?}");
        if self.framing == Framing::Manual {
            return;
        }
        self.center.target = Some(0.5 * (lo.0 + hi.0));
        self.span.target = Some(hi.0 - lo.0);
    }

    /// A hand pulled the window by `delta` this frame, moving at `vel`. Positional: the
    /// window goes exactly where the hand puts it, and `vel` is kept for the coast
    /// after the hand lets go.
    pub(super) fn drag(&mut self, delta: Semitones, vel: SemitonesPerSec, hand: FlingHand) {
        self.framing = Framing::Manual;
        self.hand = hand;
        self.held = true;
        self.center.target = None;
        self.center.pos += delta.0;
        self.center.vel = vel.0;
    }

    /// A hand zoomed by `factor` (> 1 = in = the span shrinks), keeping `anchor` where it
    /// is on screen. Direct, like the drag: two fingers are a ruler, and the rows must
    /// stay under them.
    pub(super) fn zoom_about(&mut self, factor: f32, anchor: Midi) {
        assert!(factor > 0.0, "zoom factor must be positive: {factor}");
        self.framing = Framing::Manual;
        self.span.target = None;
        self.span.vel = 0.0;
        let scale = factor.recip();
        self.center.pos = anchor.0 + (self.center.pos - anchor.0) * scale;
        self.span.pos *= scale;
    }

    /// Hand the window back to the auto framing. The velocity is kept, so a window
    /// still coasting from a fling turns toward the music without a stop.
    pub(super) fn resume_auto(&mut self) {
        self.framing = Framing::Auto;
    }

    /// Advance the motion by `dt` seconds of wall time.
    ///
    /// `bank` bounds a hand-aimed window only: outside the bank there is no evidence to
    /// look at (`take_roll::clamp_pitch` argues it). The auto framing is aimed at pitches
    /// the bank published, so it is inside by construction and is not clamped — a clamp
    /// there would silently overrule the framing.
    pub(super) fn tick(&mut self, dt: f32, bank: BankRange) {
        let coast = match self.hand {
            FlingHand::Pointer => COAST_OMEGA_POINTER,
            FlingHand::Touch => COAST_OMEGA_TOUCH,
        };
        // Growing and shrinking are different motions — see `GROW`/`SHRINK`. The span
        // never coasts (nothing flings a zoom), so the coast law is irrelevant to it.
        let opening = matches!(self.span.target, Some(target) if target > self.span.pos);
        self.span.step(dt, if opening { GROW } else { SHRINK }, coast);
        if !self.held {
            // While the span is opening for a note the center must keep up with it, or
            // the note the opening is for leaves through the other edge: it borrows the
            // opening's feel.
            self.center.step(dt, if opening { GROW } else { FOLLOW }, coast);
        }
        self.held = false;
        if self.framing == Framing::Manual {
            self.clamp_to(bank);
        }
    }

    /// Keep a hand-aimed window inside the bank and no narrower than
    /// [`MANUAL_MIN_SPAN`]. Slides, never squashes: an edge must not rescale the zoom the
    /// user chose. Hitting an edge stops the coast — a wall, not a spring.
    fn clamp_to(&mut self, bank: BankRange) {
        let (bank_lo, bank_hi) = (Midi::from(bank.lo()).0, Midi::from(bank.hi()).0);
        let whole = (bank_hi - bank_lo).max(MANUAL_MIN_SPAN.0);
        self.span.pos = self.span.pos.clamp(MANUAL_MIN_SPAN.0, whole);
        let half = 0.5 * self.span.pos;
        let (lo, hi) = (bank_lo + half, (bank_hi - half).max(bank_lo + half));
        let clamped = self.center.pos.clamp(lo, hi);
        if clamped != self.center.pos {
            self.center.pos = clamped;
            self.center.vel = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::pitch::PNote;

    const FRAME_S: f32 = 1.0 / 60.0;

    fn bank() -> BankRange {
        BankRange::new(PNote::new(36).unwrap(), PNote::new(96).unwrap())
    }

    /// Run `seconds` of frames of `dt`, recording the largest |acceleration| of the
    /// center and the span seen frame to frame.
    fn run(window: &mut PitchWindow, seconds: f32, dt: f32) -> f32 {
        let mut worst: f32 = 0.0;
        let mut prev = (window.center.vel, window.span.vel);
        for _ in 0..(seconds / dt).round() as usize {
            window.tick(dt, bank());
            let now = (window.center.vel, window.span.vel);
            worst = worst
                .max(((now.0 - prev.0) / dt).abs())
                .max(((now.1 - prev.1) / dt).abs());
            prev = now;
        }
        worst
    }

    /// The reason the module exists: an octave leap of the target does **not** kick the
    /// window. No frame accelerates past the cap — the uncapped spring would have asked
    /// for `ω²·d` ≈ 970 st/s² on the first frame of a 12-semitone grow.
    ///
    /// The bound is a literal, not `GROW.max_accel`: an oracle that reads the constant
    /// under test passes whatever the constant is — a mutant setting the cap to 1e6 went
    /// through it green. 200 st/s² is the decision; changing it is changing this test.
    #[test]
    fn a_leap_is_met_at_capped_acceleration() {
        const DECIDED_CAP: f32 = 200.0;
        let mut window = PitchWindow::new(Midi(62.0), Midi(76.0));
        window.aim(Midi(62.0), Midi(90.0));
        let worst = run(&mut window, 2.0, FRAME_S);
        assert!(
            worst <= DECIDED_CAP * 1.01,
            "accelerated at {worst:.0} st/s², the cap is {DECIDED_CAP}"
        );
        assert!((window.hi().0 - 90.0).abs() < 1e-3, "landed at {:?}", window.hi());
    }

    /// The braking envelope: a capped spring started from rest lands without passing its
    /// target — the cap must not have traded the kick for a wobble.
    #[test]
    fn a_capped_glide_does_not_overshoot() {
        for leap in [0.5f32, 3.0, 12.0, 30.0] {
            let mut window = PitchWindow::new(Midi(60.0), Midi(74.0));
            window.aim(Midi(60.0 + leap), Midi(74.0 + leap));
            let target = 67.0 + leap;
            for _ in 0..240 {
                window.tick(FRAME_S, bank());
                assert!(
                    window.center.pos <= target + 1e-4,
                    "leap {leap}: center {} passed {target}",
                    window.center.pos
                );
            }
            assert_eq!(window.center.pos, target, "leap {leap} never landed");
        }
    }

    /// The motion is the same at 20 fps as at 120 fps — sub-stepping is what keeps the
    /// cap honest on a slow phone.
    #[test]
    fn the_motion_does_not_depend_on_the_frame_rate() {
        let trace = |dt: f32| {
            let mut window = PitchWindow::new(Midi(60.0), Midi(74.0));
            window.aim(Midi(70.0), Midi(90.0));
            run(&mut window, 0.3, dt);
            (window.lo().0, window.hi().0)
        };
        let (fast, slow) = (trace(1.0 / 120.0), trace(1.0 / 20.0));
        assert!(
            (fast.0 - slow.0).abs() < 0.05 && (fast.1 - slow.1).abs() < 0.05,
            "120 fps reached {fast:?}, 20 fps {slow:?}"
        );
    }

    /// A fling coasts on, slows, and stops — and a finger's goes further than a mouse's.
    #[test]
    fn a_fling_coasts_to_rest_further_for_a_finger() {
        let travel = |hand| {
            let mut window = PitchWindow::new(Midi(60.0), Midi(74.0));
            window.drag(Semitones(0.0), SemitonesPerSec(20.0), hand);
            window.tick(FRAME_S, bank());
            let start = window.center.pos;
            run(&mut window, 4.0, FRAME_S);
            assert_eq!(window.center.vel, 0.0, "the coast never came to rest");
            window.center.pos - start
        };
        let (mouse, finger) = (travel(FlingHand::Pointer), travel(FlingHand::Touch));
        assert!(mouse > 1.0, "the fling did not coast: {mouse}");
        assert!(finger > 2.0 * mouse, "finger {finger} vs mouse {mouse}");
    }

    /// Once a hand has the window, the auto framing's aim is not obeyed — and a
    /// double-tap gives the window back, which then turns toward the music.
    #[test]
    fn a_hand_takes_the_window_and_resume_gives_it_back() {
        let mut window = PitchWindow::new(Midi(60.0), Midi(74.0));
        window.drag(Semitones(5.0), SemitonesPerSec(0.0), FlingHand::Touch);
        window.tick(FRAME_S, bank());
        window.aim(Midi(40.0), Midi(54.0));
        run(&mut window, 1.0, FRAME_S);
        assert_eq!(window.lo().0, 65.0, "the auto framing stole a hand-aimed window");

        window.resume_auto();
        window.aim(Midi(40.0), Midi(54.0));
        run(&mut window, 3.0, FRAME_S);
        assert!((window.lo().0 - 40.0).abs() < 1e-3, "lo {:?}", window.lo());
    }

    /// A pinch keeps the anchor where it was and is bounded by the bank and the minimum
    /// span; a fling into the bank's edge stops there.
    #[test]
    fn a_hand_aimed_window_stays_inside_the_bank() {
        let mut window = PitchWindow::new(Midi(60.0), Midi(80.0));
        window.zoom_about(2.0, Midi(65.0));
        assert_eq!((window.lo().0, window.hi().0), (62.5, 72.5), "anchor moved");
        window.zoom_about(100.0, Midi(65.0));
        window.tick(FRAME_S, bank());
        assert_eq!(window.span(), MANUAL_MIN_SPAN);

        window.drag(Semitones(0.0), SemitonesPerSec(500.0), FlingHand::Touch);
        run(&mut window, 3.0, FRAME_S);
        assert_eq!(window.hi().0, 96.0, "flung past the bank's top");
        assert_eq!(window.center.vel, 0.0, "the edge must stop the coast");
    }
}
