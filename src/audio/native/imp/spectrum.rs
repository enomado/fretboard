use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use super::analysis_math::{
    NOTE_BUCKET_MAX_MIDI,
    NOTE_BUCKET_MIN_MIDI,
    SPECTRUM_BINS,
    SPIRAL_BIN_COUNT,
    accumulate_note_energy,
    accumulate_spiral_energy,
    normalize_bars,
    smooth_bars,
    spectrum_bucket_index,
};
use crate::audio::types::AnalysisSettings;

pub(super) fn spectrum_bars_for_window(
    window: &[f32],
    sample_rate: f32,
    settings: &AnalysisSettings,
    planner: &mut FftPlanner<f32>,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let windowed = apply_hann_window(window);
    spectrum_bars(&windowed, sample_rate, settings, planner)
}

fn apply_hann_window(input: &[f32]) -> Vec<f32> {
    let len = input.len() as f32;
    input
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let phase = (2.0 * std::f32::consts::PI * i as f32) / (len - 1.0);
            let mult = 0.5 * (1.0 - phase.cos());
            s * mult
        })
        .collect()
}

fn spectrum_bars(
    window: &[f32],
    sample_rate: f32,
    settings: &AnalysisSettings,
    planner: &mut FftPlanner<f32>,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let fft_size = settings.fft_size.max(window.len().next_power_of_two());
    let mut input = vec![Complex32::new(0.0, 0.0); fft_size];
    for (slot, sample) in input.iter_mut().zip(window.iter().copied()) {
        slot.re = sample;
    }
    let fft = planner.plan_fft_forward(input.len());
    fft.process(&mut input);

    let magnitudes: Vec<f32> = input.iter().take(input.len() / 2).map(|v| v.norm_sqr()).collect();

    let hz_per_bin = sample_rate / input.len() as f32;
    let mut bars: Vec<f32> = vec![0.0; SPECTRUM_BINS];
    let mut note_bars: Vec<f32> = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
    let mut spiral_bars: Vec<f32> = vec![0.0; SPIRAL_BIN_COUNT];

    for (i, magnitude) in magnitudes.iter().enumerate() {
        let frequency = i as f32 * hz_per_bin;
        if !(settings.min_frequency..=settings.max_frequency).contains(&frequency) {
            continue;
        }
        if let Some(bucket) = spectrum_bucket_index(frequency, settings.min_frequency, settings.max_frequency)
        {
            bars[bucket] += *magnitude;
        }
        accumulate_note_energy(
            &mut note_bars,
            frequency,
            *magnitude,
            settings.note_spread,
            settings.concert_pitch_hz,
        );
        accumulate_spiral_energy(&mut spiral_bars, frequency, *magnitude, settings.concert_pitch_hz);
    }

    normalize_bars(&mut bars, settings.spectrum_gamma);
    normalize_bars(&mut note_bars, settings.note_gamma);
    normalize_bars(&mut spiral_bars, 1.0);
    smooth_bars(&mut bars, settings.spectrum_smoothing);

    (bars, note_bars, spiral_bars)
}
