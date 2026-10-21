//! Window-based note evidence, without a temporal pitch decoder. The engine clocks
//! these frontends at their fixed sample hops; the display cadence cannot alter them.

use super::analysis_math::slide_in;
use super::peak_comb::{
    self,
    PeakComb,
};
use super::pyin::PyinFrontend;
use super::swipe::SalienceFrame;
use super::yinfft::{
    self,
    YinFft,
};
use crate::audio::sample_rate::SampleRate;
use crate::audio::types::PitchFrontend;
use crate::core_types::pitch::{
    Hz,
    Midi,
};

pub(crate) struct WindowFrontend {
    history:     Vec<f32>,
    seen:        usize,
    comb:        PeakComb,
    yinfft:      YinFft,
    pyin:        PyinFrontend,
    sample_rate: SampleRate,
}

pub(crate) struct WindowEvidence {
    pub(crate) frame:     Option<SalienceFrame>,
    pub(crate) raw:       Option<Midi>,
    pub(crate) reference: Option<Midi>,
}

impl WindowFrontend {
    pub(crate) fn new(sample_rate: SampleRate) -> Self {
        Self {
            history: vec![0.0; crate::audio::core::MAX_WINDOW_SIZE],
            seen: 0,
            comb: PeakComb::new(sample_rate, peak_comb::DEFAULT_GATE_DBFS),
            yinfft: YinFft::new(sample_rate),
            pyin: PyinFrontend::new(),
            sample_rate,
        }
    }

    pub(crate) fn process_samples(&mut self, samples: &[f32]) {
        slide_in(&mut self.history, samples);
        self.seen = (self.seen + samples.len()).min(self.history.len());
        self.comb.process_samples(samples);
    }

    pub(crate) fn frame(
        &mut self,
        frontend: PitchFrontend,
        window_size: usize,
        reference: Hz,
    ) -> WindowEvidence {
        let (frame, raw, smoothed) = match frontend {
            PitchFrontend::Pyin => {
                let frame = (self.seen >= window_size)
                    .then(|| {
                        self.pyin.frame(
                            &self.history[self.history.len() - window_size..],
                            self.sample_rate,
                            reference,
                        )
                    })
                    .flatten();
                let raw = frame
                    .as_ref()
                    .and_then(SalienceFrame::argmax)
                    .map(|(m, _)| Midi(m));
                (frame, raw, None)
            }
            PitchFrontend::PeakComb => {
                let raw = (self.seen >= peak_comb::WINDOW)
                    .then(|| self.comb.raw_pitch())
                    .flatten();
                // Exactly one reference smoothing step per 8 ms, including unvoiced frames.
                let smoothed = self
                    .comb
                    .smooth_pitch(raw)
                    .map(|m| Midi(m.0 + 12.0 * (440.0 / reference.0).log2()));
                let raw = raw.map(|f| f.to_midi(reference));
                (raw.map(point_evidence), raw, smoothed)
            }
            PitchFrontend::YinFft => {
                let estimate = (self.seen >= yinfft::WINDOW_SAMPLES)
                    .then(|| {
                        self.yinfft
                            .estimate(&self.history[self.history.len() - yinfft::WINDOW_SAMPLES..])
                    })
                    .flatten();
                let raw = estimate.map(|e| e.hz.to_midi(reference));
                (raw.map(point_evidence), raw, None)
            }
            _ => unreachable!("only window frontends have a fixed-hop evidence source"),
        };
        WindowEvidence {
            frame,
            raw,
            reference: smoothed,
        }
    }
}

/// A scalar detector provides one hypothesis, not invented octave alternatives.
/// Represent its grid uncertainty by a 20-cent Gaussian on the 10-cent grid;
/// keep fine pitch only near that hypothesis. The evidence scale is swept in melody.
fn point_evidence(pitch: Midi) -> SalienceFrame {
    const MIN: f32 = 24.0;
    const BPS: f32 = 10.0;
    let fits: Vec<f32> = (0..841)
        .map(|i| {
            let midi = MIN + i as f32 / BPS;
            if (midi - pitch.0).abs() <= 0.4 {
                pitch.0
            } else {
                midi
            }
        })
        .collect();
    let curve = (0..841)
        .map(|i| {
            let distance = (MIN + i as f32 / BPS - pitch.0) / 0.2;
            (-0.5 * distance * distance).exp()
        })
        .collect();
    SalienceFrame::from_fitted_curve(curve, fits, MIN, BPS)
}
