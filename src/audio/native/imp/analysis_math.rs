pub(super) const SPECTRUM_BINS: usize = 72;
pub(super) const NOTE_BUCKET_MIN_MIDI: usize = 12;
pub(super) const NOTE_BUCKET_MAX_MIDI: usize = 84;
const SPIRAL_BINS_PER_SEMITONE: usize = 8;
pub(super) const SPIRAL_BIN_COUNT: usize =
    (NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI) * SPIRAL_BINS_PER_SEMITONE + 1;

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub(super) fn frequency_to_note(frequency_hz: f32) -> (String, f32) {
    let midi = 69.0 + 12.0 * (frequency_hz / 440.0).log2();
    let nearest = midi.round();
    let cents = (midi - nearest) * 100.0;
    let note_index = ((nearest as i32).rem_euclid(12)) as usize;
    let octave = (nearest as i32 / 12) - 1;
    (format!("{}{}", NOTE_NAMES[note_index], octave), cents)
}

pub(super) fn parabolic_tau(values: &[f32], tau: usize) -> f32 {
    if tau == 0 || tau + 1 >= values.len() {
        return tau as f32;
    }
    let left = values[tau - 1];
    let center = values[tau];
    let right = values[tau + 1];
    let denom = left - 2.0 * center + right;
    if denom.abs() < f32::EPSILON {
        tau as f32
    } else {
        tau as f32 + 0.5 * (left - right) / denom
    }
}

pub(super) fn smooth_frequency(previous: Option<f32>, next: f32) -> f32 {
    match previous {
        Some(prev) => {
            let corrected = correct_octave_jump(prev, next);
            let ratio = (corrected / prev).max(prev / corrected);
            let alpha = if ratio > 1.04 { 0.18 } else { 0.10 };
            prev + (corrected - prev) * alpha
        }
        None => next,
    }
}

fn correct_octave_jump(previous: f32, next: f32) -> f32 {
    let ratio = next / previous;
    if (1.85..=2.15).contains(&ratio) {
        next * 0.5
    } else if (0.46..=0.54).contains(&ratio) {
        next * 2.0
    } else {
        next
    }
}

pub(super) fn normalize_bars(values: &mut [f32], gamma: f32) {
    let max = values.iter().copied().fold(0.0, f32::max);
    if max > 0.0 {
        for v in values {
            *v = (*v / max).clamp(0.0, 1.0).powf(gamma);
        }
    }
}

pub(super) fn smooth_bars(values: &mut [f32], passes: usize) {
    if values.len() < 3 || passes == 0 {
        return;
    }
    let mut scratch = values.to_vec();
    for _ in 0..passes {
        scratch.copy_from_slice(values);
        for i in 0..values.len() {
            let l = scratch[i.saturating_sub(1)];
            let c = scratch[i];
            let r = scratch[(i + 1).min(scratch.len() - 1)];
            values[i] = l * 0.2 + c * 0.6 + r * 0.2;
        }
    }
}

pub(super) fn spectrum_bucket_index(frequency: f32, min_frequency: f32, max_frequency: f32) -> Option<usize> {
    if !(min_frequency..=max_frequency).contains(&frequency) {
        return None;
    }
    let min_log = min_frequency.log2();
    let max_log = max_frequency.log2();
    let normalized = ((frequency.log2() - min_log) / (max_log - min_log)).clamp(0.0, 1.0);
    Some((normalized * (SPECTRUM_BINS - 1) as f32).round() as usize)
}

pub(super) fn accumulate_note_energy(note_bars: &mut [f32], frequency: f32, energy: f32, note_spread: f32) {
    if frequency <= 0.0 || note_bars.is_empty() {
        return;
    }
    let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
    let note_position = midi - NOTE_BUCKET_MIN_MIDI as f32;
    let center = note_position.round() as isize;
    for index in (center - 2)..=(center + 2) {
        if !(0..note_bars.len() as isize).contains(&index) {
            continue;
        }
        let distance = (index as f32 - note_position).abs();
        if distance > 1.25 {
            continue;
        }
        let weight = (-0.5 * (distance / note_spread).powi(2)).exp();
        note_bars[index as usize] += energy * weight;
    }
}

pub(super) fn accumulate_spiral_energy(spiral_bars: &mut [f32], frequency: f32, energy: f32) {
    if frequency <= 0.0 || spiral_bars.is_empty() {
        return;
    }
    let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
    if !(NOTE_BUCKET_MIN_MIDI as f32..=NOTE_BUCKET_MAX_MIDI as f32).contains(&midi) {
        return;
    }
    let position = (midi - NOTE_BUCKET_MIN_MIDI as f32) * SPIRAL_BINS_PER_SEMITONE as f32;
    let left_index = position.floor() as usize;
    let frac = position - left_index as f32;
    if left_index < spiral_bars.len() {
        spiral_bars[left_index] += energy * (1.0 - frac);
    }
    if left_index + 1 < spiral_bars.len() {
        spiral_bars[left_index + 1] += energy * frac;
    }
}

pub(super) fn note_bucket_labels() -> Vec<String> {
    (NOTE_BUCKET_MIN_MIDI..=NOTE_BUCKET_MAX_MIDI)
        .map(|m| midi_to_note_label(m as i32))
        .collect()
}

pub(super) fn resonator_note_labels(min_midi: usize, max_midi: usize) -> Vec<String> {
    (min_midi..=max_midi)
        .map(|m| midi_to_note_label(m as i32))
        .collect()
}

fn midi_to_note_label(midi: i32) -> String {
    let note_index = midi.rem_euclid(12) as usize;
    let octave = midi / 12 - 1;
    format!("{}{}", NOTE_NAMES[note_index], octave)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::types::AnalysisSettings;

    #[test]
    fn parabolic_tau_can_overshoot_without_producing_invalid_index() {
        let values = vec![0.0, 0.5, 0.0, -0.499];
        let refined = parabolic_tau(&values, 2);
        assert!(refined > values.len() as f32);
    }

    #[test]
    fn spectrum_bucket_index_is_monotonic_in_log_space() {
        let low = spectrum_bucket_index(40.0, 20.0, 2_000.0).unwrap();
        let mid = spectrum_bucket_index(160.0, 20.0, 2_000.0).unwrap();
        let high = spectrum_bucket_index(640.0, 20.0, 2_000.0).unwrap();
        assert!(low < mid);
        assert!(mid < high);
    }

    #[test]
    fn note_energy_prefers_the_closest_semitone() {
        let mut bars = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
        accumulate_note_energy(&mut bars, 440.0, 1.0, AnalysisSettings::default().note_spread);
        let a4_index = 69 - NOTE_BUCKET_MIN_MIDI;

        let strongest = bars
            .iter()
            .enumerate()
            .max_by(|(_, l), (_, r)| l.total_cmp(r))
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(strongest, a4_index);
        assert!(bars[a4_index] > bars[a4_index - 1]);
        assert!(bars[a4_index] > bars[a4_index + 1]);
    }

    #[test]
    fn note_bucket_labels_include_low_octaves() {
        let labels = note_bucket_labels();

        assert_eq!(labels.first().map(String::as_str), Some("C0"));
        assert!(labels.iter().any(|label| label == "C1"));
        assert!(labels.iter().any(|label| label == "C2"));
    }

    #[test]
    fn low_octave_energy_lands_in_note_and_spiral_buckets() {
        let mut note_bars = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
        accumulate_note_energy(
            &mut note_bars,
            16.3516,
            1.0,
            AnalysisSettings::default().note_spread,
        );
        assert!(note_bars[0] > 0.9);

        let mut spiral_bars = vec![0.0; SPIRAL_BIN_COUNT];
        accumulate_spiral_energy(&mut spiral_bars, 32.7032, 1.0);
        let c1_index = 12 * 8;
        assert!(spiral_bars[c1_index] > 0.9);
    }
}
