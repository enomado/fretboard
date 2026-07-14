/// Lowest fundamental the tracker will look for — C0, the bottom of the note grid
/// (`NOTE_BUCKET_MIN_MIDI` = 12). Sets YIN's longest lag.
pub(crate) const LOWEST_TRACKED_FREQUENCY: f32 = 16.0;
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
