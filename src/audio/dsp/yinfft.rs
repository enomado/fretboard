//! Spectral YIN, with the fixed 2048-sample analysis window measured in R2.
//!
//! This is the shared native/web calculation, without a temporal decoder. Callers
//! supply complete windows, oldest sample first; `None` means unvoiced, not an
//! unfilled window. Cadence and timestamps belong to the sample-clock driver.
//! Numerical reference: aubio's YINFFT as captured by the R2 WASM adapter. See
//! `docs/pitch_benchmark.md`, R3, for the formulas and measured differences.

use std::sync::Arc;
use std::time::Duration;

use realfft::{
    RealFftPlanner,
    RealToComplex,
};
use rustfft::num_complex::Complex32;

use super::analysis_math::parabolic_tau;
use crate::audio::sample_rate::SampleRate;
use crate::core_types::pitch::Hz;

pub(crate) const WINDOW_SAMPLES: usize = 2048;
pub(crate) const HOP: Duration = Duration::from_millis(16);
// Reference defaults, not corpus-fitted thresholds. The level gate is measured
// on the untapered full window, just as in R2 (not on a callback-sized hop).
const TOLERANCE: f32 = 0.85;
const SILENCE_DBFS: f32 = -50.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct YinFftEstimate {
    pub(crate) hz:          Hz,
    /// 1 − CMNDF at the selected lag; a periodicity measure, not a probability.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) periodicity: f32,
}

pub(crate) struct YinFft {
    sample_rate: SampleRate,
    taper:       Vec<f32>,
    weights:     Vec<f32>,
    fft:         Arc<dyn RealToComplex<f32>>,
    // Both FFT passes reuse these allocations; estimate allocates nothing.
    input:       Vec<f32>,
    spectrum:    Vec<Complex32>,
    scratch:     Vec<Complex32>,
    difference:  Vec<f32>,
}

impl YinFft {
    pub(crate) fn new(sample_rate: SampleRate) -> Self {
        assert!(sample_rate.0 > 0, "YINFFT needs a positive sample rate");
        let fft = RealFftPlanner::new().plan_fft_forward(WINDOW_SAMPLES);
        Self {
            sample_rate,
            // Periodic Hann (denominator N), unlike the symmetric N−1 taper
            // used by the spectral display. Keep R2's window definition.
            taper: (0..WINDOW_SAMPLES)
                .map(|i| {
                    let phase = (std::f64::consts::TAU * i as f64 / WINDOW_SAMPLES as f64) as f32;
                    (0.5 * (1.0 - phase.cos() as f64)) as f32
                })
                .collect(),
            weights: (0..=WINDOW_SAMPLES / 2)
                .map(|i| spectral_weight(i as f32 / WINDOW_SAMPLES as f32 * sample_rate.hz()))
                .collect(),
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            difference: vec![1.0; WINDOW_SAMPLES / 2 + 1],
            fft,
        }
    }

    /// Analyze exactly one full window. No history, padding, HMM or smoothing.
    /// The representable periods are 1..=N/2 samples; no instrument band is
    /// imposed here, so R2's search and voicing can be compared directly.
    pub(crate) fn estimate(&mut self, window: &[f32]) -> Option<YinFftEstimate> {
        assert_eq!(window.len(), WINDOW_SAMPLES, "YINFFT expects a complete window");
        let energy = window.iter().map(|x| x * x).sum::<f32>() / WINDOW_SAMPLES as f32;
        if 10.0 * energy.log10() < SILENCE_DBFS {
            return None;
        }
        for ((out, sample), taper) in self.input.iter_mut().zip(window).zip(&self.taper) {
            *out = sample * taper;
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .unwrap();

        // A real, even power spectrum has a real forward transform. Its real
        // part is the unnormalized circular autocorrelation. Using a second
        // forward FFT avoids an inverse normalization and matches R2's scale.
        let mut sum = 0.0;
        for (i, (bin, weight)) in self.spectrum.iter().zip(&self.weights).enumerate() {
            let power = (bin.re * bin.re + bin.im * bin.im) * weight;
            self.input[i] = power;
            if i > 0 && i < WINDOW_SAMPLES / 2 {
                self.input[WINDOW_SAMPLES - i] = power;
            }
            sum += power;
        }
        // Deliberately double DC and Nyquist too: R2 does so. Replacing this
        // with sum(full mirrored spectrum) changes the normalized differences.
        sum *= 2.0;
        self.fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .unwrap();

        self.difference[0] = 1.0;
        let mut cumulative = 0.0;
        let mut best = 0;
        for lag in 1..self.difference.len() {
            let delta = sum - self.spectrum[lag].re;
            cumulative += delta;
            self.difference[lag] = if cumulative != 0.0 {
                delta * (lag as f32 / cumulative)
            } else {
                1.0
            };
            if self.difference[lag] < self.difference[best] {
                best = lag;
            }
        }
        if self.difference[best] >= TOLERANCE {
            return None;
        }
        // R2's within-frame short-period check. Integer halving is intentional
        // (the reference divides unsigned integers before rounding).
        if best <= (self.sample_rate.hz() as f64 / 1300.0).round() as usize
            && self.difference[best / 2] < TOLERANCE
        {
            best /= 2;
        }
        let period = parabolic_tau(&self.difference, best);
        (period > 0.0 && period.is_finite()).then(|| {
            YinFftEstimate {
                hz:          Hz(self.sample_rate.hz() / period),
                periodicity: 1.0 - self.difference[best],
            }
        })
    }
}

/// R2's fixed auditory weighting: linear interpolation in Hz of tabulated dB,
/// then 10^(dB/20), applied to *power*. Above the last knot hold the endpoint;
/// this also defines the otherwise unspecified case of rates above 50.2 kHz.
fn spectral_weight(hz: f32) -> f32 {
    const FREQUENCIES: [f32; 34] = [
        0., 20., 25., 31.5, 40., 50., 63., 80., 100., 125., 160., 200., 250., 315., 400., 500., 630., 800.,
        1000., 1250., 1600., 2000., 2500., 3150., 4000., 5000., 6300., 8000., 9000., 10000., 12500., 15000.,
        20000., 25100.,
    ];
    const DB: [f32; 34] = [
        -75.8, -70.1, -60.8, -52.1, -44.2, -37.5, -31.3, -25.6, -20.9, -16.5, -12.6, -9.60, -7.00, -4.70,
        -3.00, -1.80, -0.80, -0.20, 0.00, 0.50, 1.60, 3.20, 5.40, 7.80, 8.10, 5.30, -2.40, -11.1, -12.8,
        -12.2, -7.40, -17.8, -17.8, -17.8,
    ];
    let upper = FREQUENCIES
        .partition_point(|f| *f < hz)
        .clamp(1, FREQUENCIES.len() - 1);
    let lo = FREQUENCIES[upper - 1];
    let hi = FREQUENCIES[upper];
    let db = if hz >= FREQUENCIES[FREQUENCIES.len() - 1] {
        DB[DB.len() - 1]
    } else if lo == 0.0 {
        (DB[upper] - DB[upper - 1]) / hi * hz + DB[upper - 1]
    } else {
        // Preserve the reference's f32 interpolation order; the algebraically
        // simpler lerp has slightly different rounding at the spectral knots.
        (DB[upper] - DB[upper - 1]) / (hi - lo) * hz
            + (DB[upper - 1] - (DB[upper] - DB[upper - 1]) / (hi / lo - 1.0))
    };
    10.0f32.powf(db * 0.05)
}

#[cfg(test)]
mod tests;
