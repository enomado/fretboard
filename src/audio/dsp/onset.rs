//! Note-onset (attack) detection from the frame energy envelope.
//!
//! A bowed string has no sharp click at note start, so we don't look for a
//! transient spike — we track a slowly-adapting energy *baseline* and fire when
//! the current energy rises well above it. The catch that makes this useful for
//! notation is the **re-arm**: after firing, the detector won't fire again until
//! the energy dips back down. A sustained note therefore yields exactly one onset,
//! but a *re-articulated* note (a new bow stroke on the same pitch — a dip then a
//! rise, with no pitch change) dips below the baseline and re-arms, so its second
//! attack fires too. That is what lets the staff split repeated same-pitch notes
//! instead of merging them.
//!
//! Runs once per analysis frame (~40 ms), on the window RMS. Ratio-based against
//! the adaptive baseline, so it is robust to input gain.

/// How fast the baseline follows a *rising* envelope (slow — it must lag attacks).
const BASELINE_UP: f32 = 0.05;
/// …and a *falling* one (faster, so the baseline is ready for the next note soon
/// after one ends rather than sitting stuck high).
const BASELINE_DOWN: f32 = 0.30;
/// Energy must exceed `baseline · RISE_RATIO + RISE_BIAS` to be an onset.
const RISE_RATIO: f32 = 1.5;
/// Absolute floor in the rise test — the trigger level for an attack out of near
/// silence (where the baseline is ~0), and a guard against firing on room noise.
const RISE_BIAS: f32 = 0.01;
/// Energy must fall below `baseline · REARM_RATIO` to re-arm (i.e. a real dip,
/// not just steady sustain), preventing a held note from retriggering.
const REARM_RATIO: f32 = 0.8;
/// Minimum frames between onsets (~80 ms at the 40 ms analysis cadence) — a
/// refractory guard against a single ragged attack firing twice.
const REFRACTORY_FRAMES: usize = 2;

/// Stateful energy-envelope onset detector; one per audio stream.
pub(crate) struct OnsetDetector {
    baseline:     f32,
    /// False from an onset until the energy dips enough to re-arm.
    armed:        bool,
    frames_since: usize,
}

impl OnsetDetector {
    pub(crate) fn new() -> Self {
        Self {
            baseline:     0.0,
            armed:        true,
            frames_since: REFRACTORY_FRAMES,
        }
    }

    /// Feed one frame's energy (window RMS). Returns `true` on the frame an onset
    /// fires. The baseline adapts *after* the decision, so the test compares
    /// against the pre-frame baseline.
    pub(crate) fn detect(&mut self, energy: f32) -> bool {
        self.frames_since += 1;
        let onset = self.armed
            && self.frames_since >= REFRACTORY_FRAMES
            && energy > self.baseline * RISE_RATIO + RISE_BIAS;
        if onset {
            self.armed = false;
            self.frames_since = 0;
        }
        // Re-arm on a genuine dip (note release, or the trough between bow strokes).
        if energy < self.baseline * REARM_RATIO {
            self.armed = true;
        }
        let alpha = if energy > self.baseline {
            BASELINE_UP
        } else {
            BASELINE_DOWN
        };
        self.baseline += alpha * (energy - self.baseline);
        onset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step from silence into a sustained note fires exactly one onset.
    #[test]
    fn single_attack_fires_once() {
        let mut d = OnsetDetector::new();
        for _ in 0..3 {
            assert!(!d.detect(0.0)); // silence
        }
        let mut onsets = 0;
        for _ in 0..30 {
            if d.detect(0.3) {
                onsets += 1;
            }
        }
        assert_eq!(onsets, 1, "a sustained note should fire one onset");
    }

    /// A re-bow — a dip in energy then a fresh rise, same level, no pitch change —
    /// fires a second onset. This is the split-repeated-notes capability.
    #[test]
    fn re_bow_after_a_dip_fires_again() {
        let mut d = OnsetDetector::new();
        let mut onsets = 0;
        for _ in 0..12 {
            if d.detect(0.3) {
                onsets += 1;
            }
        }
        assert_eq!(onsets, 1);
        // Bow-change trough…
        for _ in 0..3 {
            d.detect(0.02);
        }
        // …then the next stroke at the same level.
        for _ in 0..12 {
            if d.detect(0.3) {
                onsets += 1;
            }
        }
        assert_eq!(onsets, 2, "the second bow stroke should fire another onset");
    }

    /// Steady room noise below the bias never fires.
    #[test]
    fn noise_floor_stays_quiet() {
        let mut d = OnsetDetector::new();
        let mut onsets = 0;
        for _ in 0..50 {
            if d.detect(0.004) {
                onsets += 1;
            }
        }
        assert_eq!(onsets, 0);
    }
}
