use super::analysis_math::parabolic_tau;

pub(super) const LOWEST_TRACKED_FREQUENCY: f32 = 16.0;
const YIN_THRESHOLD: f32 = 0.12;

pub(super) fn detect_pitch_yin(window: &[f32], sample_rate: f32) -> Option<(f32, f32)> {
    let min_lag = (sample_rate / 1000.0).max(1.0) as usize;
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

    let mut best_tau = None;
    for tau in min_lag..search_end {
        if cumulative[tau] < YIN_THRESHOLD && cumulative[tau] <= cumulative[tau + 1] {
            best_tau = Some(tau);
            break;
        }
    }

    let tau = best_tau.unwrap_or_else(|| {
        (min_lag..=search_end)
            .min_by(|l, r| cumulative[*l].total_cmp(&cumulative[*r]))
            .unwrap_or(min_lag)
    });

    let tau = parabolic_tau(&cumulative, tau);
    if !tau.is_finite() || tau <= 0.0 {
        return None;
    }
    let tau = tau.clamp(min_lag as f32, search_end as f32);
    let tau_index = tau.round().clamp(min_lag as f32, search_end as f32) as usize;
    let clarity = (1.0 - cumulative[tau_index].clamp(0.0, 1.0)).clamp(0.0, 1.0);

    Some((sample_rate / tau, clarity))
}

#[cfg(test)]
mod tests {
    use super::detect_pitch_yin;

    fn sine_wave(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let phase = i as f32 * frequency_hz * std::f32::consts::TAU / sample_rate;
                phase.sin()
            })
            .collect()
    }

    #[test]
    fn yin_handles_flat_windows_without_invalid_indices() {
        let window = vec![1.0; 981];
        let result = std::panic::catch_unwind(|| detect_pitch_yin(&window, 44_100.0));
        assert!(result.is_ok());
    }

    #[test]
    fn yin_detects_c2_on_raw_signal() {
        let sample_rate = 44_100.0;
        let expected = 65.40639;
        let window = sine_wave(expected, sample_rate, 6144);
        let (detected, _clarity) = detect_pitch_yin(&window, sample_rate).unwrap();
        assert!(
            (detected - expected).abs() < 1.0,
            "detected {detected} expected {expected}"
        );
    }

    #[test]
    fn yin_detects_c1_on_raw_signal() {
        let sample_rate = 44_100.0;
        let expected = 32.7032;
        let window = sine_wave(expected, sample_rate, 8192);
        let (detected, _clarity) = detect_pitch_yin(&window, sample_rate).unwrap();
        assert!(
            (detected - expected).abs() < 1.0,
            "detected {detected} expected {expected}"
        );
    }

    #[test]
    fn yin_detects_c3_on_raw_signal() {
        let sample_rate = 44_100.0;
        let expected = 130.81278;
        let window = sine_wave(expected, sample_rate, 6144);
        let (detected, _clarity) = detect_pitch_yin(&window, sample_rate).unwrap();
        assert!(
            (detected - expected).abs() < 1.0,
            "detected {detected} expected {expected}"
        );
    }
}
