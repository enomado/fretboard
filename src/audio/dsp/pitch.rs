/// Lowest fundamental the tracker will look for — a semitone below C1. Sets YIN's
/// longest lag.
///
/// **It follows pYIN's HMM grid, not the note grid.** [`super::pyin`]'s `MIN_MIDI` is
/// 24 (C1), so C1 is the lowest pitch the HMM has a state for. This used to sit at
/// 16 Hz instead — C0, the bottom of `NOTE_BUCKET_MIN_MIDI`, which is the *resonator
/// bank and spectrum's* grid and has no say over YIN's lag search. That cost an
/// octave of pure waste every frame: a candidate below C1 cannot be emitted into any
/// pitch state at all (`pyin::step` skips the negative bin), so lags from sr/32.7 out
/// to sr/16 were scanned to produce hypotheses the HMM structurally could not
/// represent. Measured at the 6144 window / 48 kHz: **13.93M → 8.34M ops per frame**,
/// with the candidate set bit-identical on every real note tested (C2…E5, and a
/// decaying release tail) — see `pyin::tests::floor_probe`.
///
/// That identity is not luck, it is structural: `d[tau] = difference[tau]·tau / Σ
/// difference[1..=tau]` depends only on the **prefix**, so shortening the search
/// cannot perturb a single surviving `d[tau]`. Raising the floor can therefore only
/// change frames whose first sub-threshold dip lay in the discarded tail.
///
/// The extra semitone of margin below C1 is deliberate and not slack: `max_lag` must
/// land *past* the lowest period, not on it. Pinned at exactly C1, `max_lag` at 48 kHz
/// is 1467 while C1's own dip bottoms at 1468 — one sample outside — so the lowest
/// note on the grid would be picked off the clipped edge of its dip instead of its
/// true minimum. `super::pyin::tests::floor_clears_the_hmm_grid` holds both ends of
/// this — it lives there because only that module can see both the floor and the
/// `MIN_MIDI` it has to agree with.
pub(crate) const LOWEST_TRACKED_FREQUENCY: f32 = 30.868;
/// Highest fundamental the tracker will look for — C8 (MIDI 108), the top of the
/// note grid. Sets YIN's *shortest* lag, and getting it wrong is not a soft failure.
///
/// `yin_pick` scans lags upward and takes the first dip below threshold, so a period
/// shorter than this is not merely *missed*: the first dip it can still reach is the
/// note's **sub-octave**, and the note is reported an octave flat. Measured at the
/// old value of 1000 Hz (B5): C6, E6 and A6 all came back exactly −12.00 semitones
/// (`pyin::tests::ceiling_probe`), silently transposing the whole upper half of the
/// violin's range. C8 clears every instrument here — violin's E7 = 2637 Hz included —
/// with margin, and costs only a few extra lags to scan.
pub(crate) const HIGHEST_TRACKED_FREQUENCY: f32 = 4186.0;

/// **The app's tracked pitch domain, in MIDI: C1..C8.** The same span the two constants
/// above express in Hz — this is the MIDI form, and both belong to the same decision.
///
/// It lives here, once, because it is a claim about *what this app detects*, and every
/// detector has to make the same claim or they disagree about reality. `dsp::pyin`'s HMM
/// grid is cut to it; `dsp::swipe` searches candidates in it.
///
/// # This is not the bank's range, and must never be taken from it
///
/// `dsp::resonator`'s column spans C0..C8 because that is a good range for the **spectrum
/// waterfall** — a display. Reading the candidate range off it (which `swipe` did until
/// this constant existed) hands the detector candidates down to **C0 = 16 Hz**, where no
/// instrument here plays and where a low candidate's kernel lobes are wide in absolute Hz
/// (h=1 spans ~9 semitones at 32 Hz). Room rumble falls off with frequency, which is the
/// exact shape SWIPE's `1/√r` envelope expects of a harmonic series, so rumble scores as a
/// sub-bass note — measured beating a real bowed G3 by 4-6% on its own recording, and seen
/// live as the pitch roll plunging three octaves and dragging its own auto-framing down
/// with it.
///
/// That was the third time a *display* setting was found steering the detector, after the
/// silence gate sharing the UI meter's smoothing (Phase 1.9) and `gamma` reaching the
/// octave decision (Phase 1.11). Same rule each time: the detector's domain is a property
/// of the task, never of the picture.
pub(crate) const TRACKED_MIN_MIDI: f32 = 24.0;
/// See [`TRACKED_MIN_MIDI`]. C8 — clears every instrument here, violin E7 (2637 Hz)
/// included, and matches [`HIGHEST_TRACKED_FREQUENCY`].
pub(crate) const TRACKED_MAX_MIDI: f32 = 108.0;

/// The cumulative mean normalized difference function (YIN step 3) plus the lag
/// search bounds — the substrate probabilistic YIN ([`super::pyin`]) turns into
/// weighted pitch candidates. `d[0] == 1`; `d[tau]` dips toward 0 near true
/// periods.
pub(crate) struct Cmndf {
    pub(crate) d:       Vec<f32>,
    pub(crate) min_lag: usize,
    pub(crate) max_lag: usize,
}

/// Compute the CMNDF for one analysis window. `None` when the window is too short
/// for even a single lag of the tracked range.
pub(crate) fn cmndf(window: &[f32], sample_rate: f32) -> Option<Cmndf> {
    let min_lag = (sample_rate / HIGHEST_TRACKED_FREQUENCY).max(1.0) as usize;
    let max_lag = (sample_rate / LOWEST_TRACKED_FREQUENCY) as usize;
    let search_end = max_lag.min(window.len().saturating_sub(1));
    if min_lag >= search_end {
        return None;
    }

    let mut difference = vec![0.0f32; search_end + 1];
    let mut cumulative = vec![0.0f32; search_end + 1];

    for tau in 1..=search_end {
        let limit = window.len().saturating_sub(tau);
        let mut sum = 0.0;
        for i in 0..limit {
            let d = window[i] - window[i + tau];
            sum += d * d;
        }
        difference[tau] = sum;
    }

    cumulative[0] = 1.0;
    let mut running_sum = 0.0;
    for tau in 1..=search_end {
        running_sum += difference[tau];
        cumulative[tau] = if running_sum > 0.0 {
            difference[tau] * tau as f32 / running_sum
        } else {
            1.0
        };
    }

    Some(Cmndf {
        d: cumulative,
        min_lag,
        max_lag: search_end,
    })
}

#[cfg(test)]
mod tests {
    use super::cmndf;

    fn sine_wave(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let phase = i as f32 * frequency_hz * std::f32::consts::TAU / sample_rate;
                phase.sin()
            })
            .collect()
    }

    /// Frequency of the CMNDF's *first* dip below `threshold`, walked to its bottom
    /// — YIN's period-selection rule and the substrate pYIN's candidate stage
    /// integrates over thresholds. (The *global* min sits on a later period
    /// multiple, which is exactly why one takes the first dip, not the deepest.)
    fn first_dip_frequency(window: &[f32], sample_rate: f32, threshold: f32) -> f32 {
        let c = cmndf(window, sample_rate).unwrap();
        for tau in c.min_lag..c.max_lag {
            if c.d[tau] < threshold {
                let mut t = tau;
                while t + 1 <= c.max_lag && c.d[t + 1] < c.d[t] {
                    t += 1;
                }
                return sample_rate / t as f32;
            }
        }
        panic!("no dip below {threshold}");
    }

    #[test]
    fn cmndf_handles_flat_windows_without_invalid_indices() {
        let window = vec![1.0; 981];
        let result = std::panic::catch_unwind(|| cmndf(&window, 44_100.0));
        assert!(result.is_ok());
    }

    #[test]
    fn cmndf_dip_lands_on_c2_period() {
        let sr = 44_100.0;
        let expected = 65.40639;
        assert!((first_dip_frequency(&sine_wave(expected, sr, 6144), sr, 0.1) - expected).abs() < 1.0);
    }

    #[test]
    fn cmndf_dip_lands_on_c1_period() {
        let sr = 44_100.0;
        let expected = 32.7032;
        assert!((first_dip_frequency(&sine_wave(expected, sr, 8192), sr, 0.1) - expected).abs() < 1.0);
    }

    #[test]
    fn cmndf_dip_lands_on_c3_period() {
        let sr = 44_100.0;
        let expected = 130.81278;
        assert!((first_dip_frequency(&sine_wave(expected, sr, 6144), sr, 0.1) - expected).abs() < 1.0);
    }
}
