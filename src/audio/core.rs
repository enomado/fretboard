//! Platform-agnostic analysis core shared by the native and wasm engines.
//!
//! `SharedState` is the snapshot the UI reads (behind `Arc<Mutex<…>>`); the two
//! pipelines turn a stream of mono `f32` samples into that snapshot. On native a
//! background thread drives them; on wasm the Web Audio callback does — but the
//! analysis (FFT/YIN/resonator + all the publishing/waterfall bookkeeping) is
//! identical, which is the whole point of keeping it here rather than per-engine.
//!
//! `Arc<Mutex<…>>` and the atomics are used on wasm too: it is single-threaded
//! there, so the mutex never contends, but the types compile and the pipeline
//! code stays byte-identical across targets.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{
    AtomicU32,
    Ordering,
};

use rustfft::FftPlanner;
// `web_time::Instant` re-exports `std` on native/Android and uses
// `performance.now()` on wasm, where `std::time::Instant` panics.
use web_time::{
    Duration,
    Instant,
};

use crate::audio::dsp::analysis_math::{
    frequency_to_note,
    note_bucket_labels,
    smooth_frequency,
};
use crate::audio::dsp::pitch::{
    LOWEST_TRACKED_FREQUENCY,
    detect_pitch_yin,
};
use crate::audio::dsp::resonator::{
    ResonatorAnalyzer,
    ResonatorSnapshot,
    ResonatorViewSettings,
};
use crate::audio::dsp::spectrum::spectrum_bars_for_window;
use crate::audio::types::{
    AnalysisSettings,
    AudioStatus,
    TunerReading,
};

// ------------------------------------------------------------------
// Конфигурация анализа
// ------------------------------------------------------------------
pub(crate) const MAX_WINDOW_SIZE: usize = 16384;
pub(crate) const WATERFALL_HISTORY: usize = 52;
pub(crate) const ANALYSIS_INTERVAL: Duration = Duration::from_millis(40);
pub(crate) const SILENCE_RMS_THRESHOLD: f32 = 0.0;
pub(crate) const INPUT_WAVEFORM_HISTORY: usize = 2048;

// ------------------------------------------------------------------
// Данные, которые UI читает через AudioEngine
// ------------------------------------------------------------------
pub(crate) struct SharedState {
    pub(crate) status:              AudioStatus,
    pub(crate) reading:             Option<TunerReading>,
    pub(crate) input_waveform:      VecDeque<f32>,
    pub(crate) waterfall:           VecDeque<Vec<f32>>,
    pub(crate) note_waterfall:      VecDeque<Vec<f32>>,
    pub(crate) spiral_waterfall:    VecDeque<Vec<f32>>,
    pub(crate) resonator_spectrum:  Vec<f32>,
    pub(crate) resonator_waterfall: VecDeque<Vec<f32>>,
    pub(crate) resonator_labels:    Vec<String>,
    pub(crate) smoothed_frequency:  Option<f32>,
}

impl SharedState {
    pub(crate) fn new() -> Self {
        Self {
            status:              AudioStatus::Idle,
            reading:             None,
            input_waveform:      VecDeque::with_capacity(INPUT_WAVEFORM_HISTORY),
            waterfall:           VecDeque::with_capacity(WATERFALL_HISTORY),
            note_waterfall:      VecDeque::with_capacity(WATERFALL_HISTORY),
            spiral_waterfall:    VecDeque::with_capacity(WATERFALL_HISTORY),
            resonator_spectrum:  Vec::new(),
            resonator_waterfall: VecDeque::with_capacity(WATERFALL_HISTORY),
            resonator_labels:    Vec::new(),
            smoothed_frequency:  None,
        }
    }

    /// Drop all accumulated analysis and mark the engine as actively listening.
    /// Called when a capture (re)starts so stale spectra/waterfalls from the
    /// previous device don't bleed into the new stream.
    pub(crate) fn reset(&mut self) {
        self.reading = None;
        self.input_waveform.clear();
        self.waterfall.clear();
        self.note_waterfall.clear();
        self.spiral_waterfall.clear();
        self.resonator_spectrum.clear();
        self.resonator_waterfall.clear();
        self.resonator_labels.clear();
        self.smoothed_frequency = None;
        self.status = AudioStatus::Listening;
    }
}

// Used by the native engine; the wasm worker reports errors a different way, so
// it's dead code in wasm builds — silence the lint rather than split the module.
#[allow(dead_code)]
pub(crate) fn set_shared_error(shared: &Arc<Mutex<SharedState>>, msg: &str) {
    if let Ok(mut state) = shared.lock() {
        state.status = AudioStatus::Error(msg.to_owned());
    }
}

// ------------------------------------------------------------------
// Pipelines: чистые функции, ничего аудио-специфичного.
// ------------------------------------------------------------------
pub(crate) struct ResonatorPipeline {
    analyzer:     ResonatorAnalyzer,
    last_publish: Instant,
}

pub(crate) struct AnalysisPipeline {
    buffer:        VecDeque<f32>,
    last_analysis: Instant,
    planner:       FftPlanner<f32>,
    sample_rate:   f32,
}

#[derive(Clone, Copy, Debug)]
struct PitchEstimate {
    frequency_hz: f32,
    clarity:      f32,
}

#[derive(Clone, Debug)]
struct AnalysisFrame {
    pitch:            Option<PitchEstimate>,
    spectrum:         Vec<f32>,
    note_spectrum:    Vec<f32>,
    spiral_spectrum:  Vec<f32>,
    // Камертон момента анализа: нота считается после сглаживания частоты
    // (в publish_*), а settings туда не доходят — несём значение во фрейме.
    concert_pitch_hz: f32,
}

impl ResonatorPipeline {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            analyzer:     ResonatorAnalyzer::new(sample_rate),
            last_publish: Instant::now() - Duration::from_millis(16),
        }
    }

    pub(crate) fn push_samples(
        &mut self,
        samples: impl IntoIterator<Item = f32>,
        shared: &Arc<Mutex<SharedState>>,
        settings: &Arc<Mutex<AnalysisSettings>>,
        input_gain: &Arc<AtomicU32>,
    ) {
        let analysis_settings = settings.lock().map(|g| g.clone()).unwrap_or_default().sanitized();
        self.sync_settings(&analysis_settings, shared);

        let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
        let samples: Vec<f32> = samples.into_iter().map(|sample| sample * gain).collect();
        self.analyzer.process_samples(&samples, analysis_settings.resonator.reassign);

        let publish_interval = Duration::from_millis(analysis_settings.resonator.update_ms);
        if self.last_publish.elapsed() < publish_interval {
            return;
        }
        self.last_publish = Instant::now();
        publish_resonator_snapshot(
            shared,
            self.analyzer.snapshot(analysis_settings.resonator.reassign),
            analysis_settings.resonator.history,
        );
    }

    fn sync_settings(&mut self, settings: &AnalysisSettings, shared: &Arc<Mutex<SharedState>>) {
        let requested = ResonatorViewSettings::from(settings);
        if !self.analyzer.sync_settings(requested) {
            return;
        }
        if let Ok(mut state) = shared.lock() {
            state.resonator_spectrum.clear();
            state.resonator_waterfall.clear();
            state.resonator_labels = self.analyzer.note_labels();
            let resonator_labels = state.resonator_labels.clone();
            if let Some(reading) = state.reading.as_mut() {
                reading.resonator_spectrum.clear();
                reading.resonator_waterfall.clear();
                reading.resonator_note_labels = resonator_labels;
            }
        }
    }
}

impl AnalysisPipeline {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            buffer: VecDeque::with_capacity(MAX_WINDOW_SIZE * 2),
            last_analysis: Instant::now() - ANALYSIS_INTERVAL,
            planner: FftPlanner::new(),
            sample_rate,
        }
    }

    pub(crate) fn push_samples(
        &mut self,
        samples: impl IntoIterator<Item = f32>,
        shared: &Arc<Mutex<SharedState>>,
        settings: &Arc<Mutex<AnalysisSettings>>,
        input_gain: &Arc<AtomicU32>,
        input_level: &Arc<AtomicU32>,
    ) {
        let analysis_settings = settings.lock().map(|g| g.clone()).unwrap_or_default().sanitized();
        let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
        let mut recent: Vec<f32> = Vec::new();

        // Применяем гейн без хард-клипа: обрезка в signal-path рождает
        // гармоники, сбивает YIN/FFT. Отрисовка сама клипит на ±1 при ренде.
        for s in samples {
            let scaled = s * gain;
            self.buffer.push_back(scaled);
            recent.push(scaled);
        }

        append_input_waveform(shared, &recent);

        while self.buffer.len() > MAX_WINDOW_SIZE * 2 {
            self.buffer.pop_front();
        }

        if self.buffer.len() < analysis_settings.window_size
            || self.last_analysis.elapsed() < ANALYSIS_INTERVAL
        {
            return;
        }
        self.last_analysis = Instant::now();

        let start = self.buffer.len().saturating_sub(analysis_settings.window_size);
        let window: Vec<f32> = self.buffer.iter().skip(start).copied().collect();
        let level = normalized_level(&window);
        let previous_level = f32::from_bits(input_level.load(Ordering::Relaxed));
        let smoothed_level_value = smoothed_level(previous_level, level);
        input_level.store(smoothed_level_value.to_bits(), Ordering::Relaxed);
        let frame = analyze_window(&window, self.sample_rate, &analysis_settings, &mut self.planner);
        publish_analysis_reading(shared, frame);
    }
}

// ------------------------------------------------------------------
// Чистые функции анализа (FFT / YIN / резонаторы / метки)
// ------------------------------------------------------------------
fn normalized_level(window: &[f32]) -> f32 {
    let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
    if rms <= f32::EPSILON {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 54.0) / 48.0).clamp(0.0, 1.0)
}

fn smoothed_level(previous: f32, current: f32) -> f32 {
    let alpha = if current > previous { 0.32 } else { 0.12 };
    previous + (current - previous) * alpha
}

fn analyze_window(
    window: &[f32],
    sample_rate: f32,
    settings: &AnalysisSettings,
    planner: &mut FftPlanner<f32>,
) -> AnalysisFrame {
    let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
    let (spectrum, note_spectrum, spiral_spectrum) =
        spectrum_bars_for_window(window, sample_rate, settings, planner);
    let pitch = if rms < SILENCE_RMS_THRESHOLD {
        None
    } else {
        detect_pitch_yin(window, sample_rate).and_then(|(f, c)| {
            (LOWEST_TRACKED_FREQUENCY..=1200.0)
                .contains(&f)
                .then_some(PitchEstimate {
                    frequency_hz: f,
                    clarity:      c,
                })
        })
    };

    AnalysisFrame {
        pitch,
        spectrum,
        note_spectrum,
        spiral_spectrum,
        concert_pitch_hz: settings.concert_pitch_hz,
    }
}

fn append_input_waveform(shared: &Arc<Mutex<SharedState>>, samples: &[f32]) {
    if samples.is_empty() {
        return;
    }
    if let Ok(mut state) = shared.lock() {
        state.input_waveform.extend(samples.iter().copied());
        while state.input_waveform.len() > INPUT_WAVEFORM_HISTORY {
            state.input_waveform.pop_front();
        }
    }
}

fn push_limited_history<T>(history: &mut VecDeque<T>, item: T, max_len: usize) {
    history.push_back(item);
    while history.len() > max_len {
        history.pop_front();
    }
}

fn publish_analysis_reading(shared: &Arc<Mutex<SharedState>>, frame: AnalysisFrame) {
    if let Ok(mut state) = shared.lock() {
        let (smoothed_frequency, clarity) = match frame.pitch {
            Some(pitch) => {
                let sf = smooth_frequency(state.smoothed_frequency, pitch.frequency_hz);
                state.smoothed_frequency = Some(sf);
                (sf, pitch.clarity)
            }
            None => {
                let Some(sf) = state.smoothed_frequency else {
                    return;
                };
                (sf, 0.0)
            }
        };

        let (note_name, cents) = frequency_to_note(smoothed_frequency, frame.concert_pitch_hz);
        push_limited_history(&mut state.waterfall, frame.spectrum.clone(), WATERFALL_HISTORY);
        push_limited_history(
            &mut state.note_waterfall,
            frame.note_spectrum.clone(),
            WATERFALL_HISTORY,
        );
        push_limited_history(
            &mut state.spiral_waterfall,
            frame.spiral_spectrum.clone(),
            WATERFALL_HISTORY,
        );
        state.reading = Some(TunerReading {
            frequency_hz: smoothed_frequency,
            note_name,
            cents,
            clarity,
            spectrum: frame.spectrum,
            waterfall: state.waterfall.iter().cloned().collect(),
            note_spectrum: frame.note_spectrum,
            note_waterfall: state.note_waterfall.iter().cloned().collect(),
            spiral_spectrum: frame.spiral_spectrum,
            spiral_waterfall: state.spiral_waterfall.iter().cloned().collect(),
            resonator_spectrum: state.resonator_spectrum.clone(),
            resonator_waterfall: state.resonator_waterfall.iter().cloned().collect(),
            resonator_note_labels: state.resonator_labels.clone(),
            note_labels: note_bucket_labels(),
        });
        state.status = AudioStatus::Listening;
    }
}

fn publish_resonator_snapshot(
    shared: &Arc<Mutex<SharedState>>,
    snapshot: ResonatorSnapshot,
    history_len: usize,
) {
    if let Ok(mut state) = shared.lock() {
        state.resonator_spectrum = snapshot.spectrum;
        state.resonator_labels = snapshot.note_labels;
        let resonator_spectrum = state.resonator_spectrum.clone();
        let resonator_labels = state.resonator_labels.clone();
        push_limited_history(
            &mut state.resonator_waterfall,
            resonator_spectrum.clone(),
            history_len,
        );
        let resonator_waterfall: Vec<Vec<f32>> = state.resonator_waterfall.iter().cloned().collect();

        if let Some(reading) = state.reading.as_mut() {
            reading.resonator_spectrum = resonator_spectrum;
            reading.resonator_waterfall = resonator_waterfall;
            reading.resonator_note_labels = resonator_labels;
        }
    }
}
