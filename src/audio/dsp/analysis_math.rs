pub(crate) const SPECTRUM_BINS: usize = 72;
pub(crate) const NOTE_BUCKET_MIN_MIDI: usize = 12;
pub(crate) const NOTE_BUCKET_MAX_MIDI: usize = 84;
pub(crate) const SPIRAL_BINS_PER_SEMITONE: usize = 8;
pub(crate) const SPIRAL_BIN_COUNT: usize =
    (NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI) * SPIRAL_BINS_PER_SEMITONE + 1;

use crate::core_types::note::AccidentalStyle;

pub(crate) fn frequency_to_note(
    frequency_hz: f32,
    reference_hz: f32,
    style: AccidentalStyle,
) -> (String, f32) {
    let midi = 69.0 + 12.0 * (frequency_hz / reference_hz).log2();
    let nearest = midi.round();
    let cents = (midi - nearest) * 100.0;
    (style.midi_name(nearest as i32), cents)
}

pub(crate) fn parabolic_tau(values: &[f32], tau: usize) -> f32 {
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

pub(crate) fn normalize_bars(values: &mut [f32], gamma: f32) {
    let max = values.iter().copied().fold(0.0, f32::max);
    if max > 0.0 {
        for v in values {
            *v = (*v / max).clamp(0.0, 1.0).powf(gamma);
        }
    }
}

pub(crate) fn smooth_bars(values: &mut [f32], passes: usize) {
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

pub(crate) fn spectrum_bucket_index(frequency: f32, min_frequency: f32, max_frequency: f32) -> Option<usize> {
    if !(min_frequency..=max_frequency).contains(&frequency) {
        return None;
    }
    let min_log = min_frequency.log2();
    let max_log = max_frequency.log2();
    let normalized = ((frequency.log2() - min_log) / (max_log - min_log)).clamp(0.0, 1.0);
    Some((normalized * (SPECTRUM_BINS - 1) as f32).round() as usize)
}

pub(crate) fn accumulate_note_energy(
    note_bars: &mut [f32],
    frequency: f32,
    energy: f32,
    note_spread: f32,
    reference_hz: f32,
) {
    if frequency <= 0.0 || note_bars.is_empty() {
        return;
    }
    let midi = 69.0 + 12.0 * (frequency / reference_hz).log2();
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

pub(crate) fn accumulate_spiral_energy(
    spiral_bars: &mut [f32],
    frequency: f32,
    energy: f32,
    reference_hz: f32,
) {
    if frequency <= 0.0 || spiral_bars.is_empty() {
        return;
    }
    let midi = 69.0 + 12.0 * (frequency / reference_hz).log2();
    if !(NOTE_BUCKET_MIN_MIDI as f32..=NOTE_BUCKET_MAX_MIDI as f32).contains(&midi) {
        return;
    }
    let position = (midi - NOTE_BUCKET_MIN_MIDI as f32) * SPIRAL_BINS_PER_SEMITONE as f32;
    splat_linear(spiral_bars, position, energy);
}

/// Splat `weight` into `bars` at the fractional bin index `position`, using a
/// two-tap linear interpolation: the weight is shared between the two nearest
/// bins in proportion to the sub-bin fraction. Positions outside the grid are
/// dropped. This is the shared kernel behind both the FFT spiral
/// ([`accumulate_spiral_energy`]) and the resonator's instantaneous-frequency
/// reassignment splat — both place continuous-frequency energy onto a discrete
/// pitch grid, so they must round the same way.
pub(crate) fn splat_linear(bars: &mut [f32], position: f32, weight: f32) {
    if weight <= 0.0 || bars.is_empty() || position < 0.0 {
        return;
    }
    let left = position.floor() as usize;
    let frac = position - left as f32;
    if left < bars.len() {
        bars[left] += weight * (1.0 - frac);
    }
    if left + 1 < bars.len() {
        bars[left + 1] += weight * frac;
    }
}

pub(crate) fn note_bucket_labels(style: AccidentalStyle) -> Vec<String> {
    (NOTE_BUCKET_MIN_MIDI..=NOTE_BUCKET_MAX_MIDI)
        .map(|m| style.midi_name(m as i32))
        .collect()
}

pub(crate) fn resonator_note_labels(min_midi: usize, max_midi: usize, style: AccidentalStyle) -> Vec<String> {
    (min_midi..=max_midi).map(|m| style.midi_name(m as i32)).collect()
}

/// How many harmonics [`resonator_fundamental`] sums when scoring a candidate as a
/// fundamental. 5 is enough to out-vote a single loud overtone without dragging in
/// noise from the far end of the bank.
pub(crate) const RESONATOR_HARMONICS: usize = 5;

/// Harmonic-aware fundamental from one reassigned resonator column — the *fast*
/// pitch prior for the note detector.
///
/// `column` is a normalized (0..1) magnitude per output bin, bin `b` mapping to
/// pitch `min_midi + b / bins_per_semitone`. A plain argmax picks whichever partial
/// is loudest, which on a bowed string is often an overtone — an octave/fifth error.
/// Instead we score every bin *as if it were the fundamental* by summing the energy
/// at its first [`RESONATOR_HARMONICS`] harmonics (which sit at fixed
/// `+12·log2(h)` semitone offsets on this log-pitch grid) and keep the best.
///
/// Two failure modes handled:
/// - **Octave-up** (crowning an overtone): a real fundamental collects its *own*
///   partials and outscores any single overtone, which collects only its sparser
///   higher ones.
/// - **Sub-octave** (half-pitch phantom): a bin an octave *below* the tone would
///   score well purely from the real tone landing in it as a 2nd harmonic — so we
///   require the candidate bin to carry real energy itself (`>= floor`), which a
///   phantom fundamental does not.
///
/// Returns `(fractional_midi, strength)` where `strength` is the fundamental bin's
/// own normalized magnitude (for a downstream silence gate), or `None` if nothing
/// crosses `floor`.
pub(crate) fn resonator_fundamental(
    column: &[f32],
    min_midi: f32,
    bins_per_semitone: f32,
    floor: f32,
) -> Option<(f32, f32)> {
    if column.len() < 2 || bins_per_semitone <= 0.0 {
        return None;
    }
    let mut best_bin: Option<usize> = None;
    let mut best_score = 0.0f32;
    for b in 0..column.len() {
        // The fundamental itself must carry energy — this is the sub-octave guard.
        if column[b] < floor {
            continue;
        }
        let mut score = 0.0;
        for h in 1..=RESONATOR_HARMONICS {
            // Harmonic h is +12·log2(h) semitones up → a fixed bin offset here.
            // Weight 1/h so the fundamental and octave dominate the decision and
            // higher partials only break ties.
            let offset = (bins_per_semitone * 12.0 * (h as f32).log2()).round() as usize;
            let idx = b + offset;
            if idx >= column.len() {
                break;
            }
            score += column[idx] / h as f32;
        }
        if score > best_score {
            best_score = score;
            best_bin = Some(b);
        }
    }
    let bin = best_bin?;
    // Sub-bin refine on the fundamental's own peak; clamp so a bin that is not a
    // clean local extremum cannot fling the estimate more than half a bin.
    let refined = parabolic_tau(column, bin).clamp(bin as f32 - 0.5, bin as f32 + 0.5);
    let midi = min_midi + refined / bins_per_semitone;
    Some((midi, column[bin]))
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
        accumulate_note_energy(
            &mut bars,
            440.0,
            1.0,
            AnalysisSettings::default().note_spread,
            440.0,
        );
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
        let labels = note_bucket_labels(AccidentalStyle::Sharps);

        assert_eq!(labels.first().map(String::as_str), Some("C0"));
        assert!(labels.iter().any(|label| label == "C1"));
        assert!(labels.iter().any(|label| label == "C2"));
    }

    /// One reassigned column with an A4 fundamental (bin 456) and a *louder* 2nd
    /// harmonic an octave up (bin 552). A plain argmax would call it A5; the
    /// harmonic scoring must keep it at A4.
    #[test]
    fn resonator_fundamental_picks_fundamental_over_louder_overtone() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32; // reassigned grid = 8/semitone
        let min_midi = NOTE_BUCKET_MIN_MIDI as f32; // 12
        let mut col = vec![0.0f32; SPIRAL_BIN_COUNT];
        let a4 = ((69.0 - min_midi) * bps) as usize; // 456
        let a5 = a4 + (12.0 * bps) as usize; // +12 semitones = 552
        col[a4] = 0.6; // fundamental
        col[a5] = 1.0; // louder overtone (would win a naive argmax)

        let (midi, _strength) = resonator_fundamental(&col, min_midi, bps, 0.12).unwrap();
        assert!((midi - 69.0).abs() < 0.2, "expected ~A4 (69), got {midi}");
    }

    /// A lone peak returns its own pitch, sub-bin refined.
    #[test]
    fn resonator_fundamental_lone_peak_returns_its_pitch() {
        let bps = SPIRAL_BINS_PER_SEMITONE as f32;
        let min_midi = NOTE_BUCKET_MIN_MIDI as f32;
        let mut col = vec![0.0f32; SPIRAL_BIN_COUNT];
        let d5 = ((74.0 - min_midi) * bps) as usize;
        // Split across two bins so the parabolic refine has something to interpolate.
        col[d5] = 0.9;
        col[d5 + 1] = 0.3;
        let (midi, strength) = resonator_fundamental(&col, min_midi, bps, 0.12).unwrap();
        assert!((midi - 74.0).abs() < 0.2, "expected ~D5 (74), got {midi}");
        assert!(strength > 0.12);
    }

    /// Silence (all below the floor) yields no fundamental rather than a phantom.
    #[test]
    fn resonator_fundamental_silence_returns_none() {
        let col = vec![0.05f32; SPIRAL_BIN_COUNT];
        assert!(resonator_fundamental(&col, NOTE_BUCKET_MIN_MIDI as f32, 8.0, 0.12).is_none());
    }

    #[test]
    fn low_octave_energy_lands_in_note_and_spiral_buckets() {
        let mut note_bars = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
        accumulate_note_energy(
            &mut note_bars,
            16.3516,
            1.0,
            AnalysisSettings::default().note_spread,
            440.0,
        );
        assert!(note_bars[0] > 0.9);

        let mut spiral_bars = vec![0.0; SPIRAL_BIN_COUNT];
        accumulate_spiral_energy(&mut spiral_bars, 32.7032, 1.0, 440.0);
        let c1_index = 12 * 8;
        assert!(spiral_bars[c1_index] > 0.9);
    }
}
