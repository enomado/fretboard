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
use std::sync::atomic::{
    AtomicU32,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
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
};
use crate::audio::dsp::melody::MelodyTracker;
use crate::audio::dsp::onset::OnsetDetector;
use crate::audio::dsp::pitch::{
    HIGHEST_TRACKED_FREQUENCY,
    LOWEST_TRACKED_FREQUENCY,
};
use crate::audio::dsp::pyin::PitchTracker;
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
use crate::core_types::note::AccidentalStyle;

// ------------------------------------------------------------------
// Конфигурация анализа
// ------------------------------------------------------------------
pub(crate) const MAX_WINDOW_SIZE: usize = 16384;
pub(crate) const WATERFALL_HISTORY: usize = 52;
pub(crate) const ANALYSIS_INTERVAL: Duration = Duration::from_millis(40);
pub(crate) const SILENCE_RMS_THRESHOLD: f32 = 0.0;
pub(crate) const INPUT_WAVEFORM_HISTORY: usize = 2048;
/// Below this normalized window level the resonator bank's fast pitch is not fused
/// into pYIN — the bank's column is normalized, so it reports *some* fundamental
/// even for room noise; gating on real level keeps that noise out of the tracker.
pub(crate) const BANK_FUSE_LEVEL: f32 = 0.02;

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
    /// Latest fast played-note prior from the resonator bank, kept on the shared
    /// state so both publish paths (the fast 16 ms snapshot and the 40 ms YIN
    /// reading) stamp the *same* value onto `TunerReading::fast_pitch` — otherwise
    /// the 40 ms path would blank it every frame it rebuilds the reading.
    pub(crate) fast_pitch:          Option<(f32, f32)>,
    /// pYIN's octave opinion for the melody line: `(fractional_midi, voiced
    /// probability)`, or `None` before the first reading.
    ///
    /// Kept here as MIDI rather than Hz because the two publish paths run at
    /// different cadences on different threads: the 40 ms pYIN path owns the concert
    /// pitch it was computed against and writes this, and the 16 ms bank path reads
    /// it without needing the settings at all. See `dsp::melody`.
    pub(crate) octave_anchor:       Option<(f32, f32)>,
    /// Marries the two into the melody line's played note. Driven **only** by the
    /// bank's ~16 ms publish path, which is the cadence its leap/slip hysteresis is
    /// counted in — see [`MelodyTracker`].
    pub(crate) melody:              MelodyTracker,
    /// Last note [`Self::melody`] produced, kept so the 40 ms pYIN path can re-stamp
    /// it onto the reading it rebuilds instead of recomputing (which would
    /// double-count the hysteresis) or blanking it.
    pub(crate) melody_pitch:        Option<(f32, f32)>,
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
            fast_pitch:          None,
            octave_anchor:       None,
            melody:              MelodyTracker::default(),
            melody_pitch:        None,
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
        self.fast_pitch = None;
        self.octave_anchor = None;
        self.melody = MelodyTracker::default();
        self.melody_pitch = None;
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
    // Probabilistic-YIN tracker: the octave-robust, HMM-smoothed pitch source that
    // replaced plain YIN + the `smooth_frequency` EMA. Stateful (carries the
    // Viterbi trellis across frames), so it lives on the per-stream pipeline.
    pitch_tracker: PitchTracker,
    // Energy-attack onset detector + a monotonic counter bumped on each onset. The
    // UI diffs the counter to spot a new attack (splitting re-bowed repeats), so
    // the value on the reading is a sequence number, not a per-frame flag.
    onset:         OnsetDetector,
    onset_seq:     u64,
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
    // Стиль знаков альтерации (диезы/бемоли) на момент кадра — как и камертон,
    // нужен при подписи нот в publish_*, куда settings не доходят.
    accidental:       AccidentalStyle,
    // Monotonic onset counter as of this frame (see `AnalysisPipeline::onset_seq`).
    onset_seq:        u64,
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
        self.analyzer
            .process_samples(&samples, analysis_settings.resonator.reassign);

        let publish_interval = Duration::from_millis(analysis_settings.resonator.update_ms);
        if self.last_publish.elapsed() < publish_interval {
            return;
        }
        self.last_publish = Instant::now();
        publish_resonator_snapshot(
            shared,
            self.analyzer
                .snapshot(analysis_settings.resonator.reassign, analysis_settings.accidental),
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
            state.resonator_labels = self.analyzer.note_labels(settings.accidental);
            state.fast_pitch = None;
            // The bank's grid just changed under us, so the melody line's octave
            // dispute is about a reading that no longer exists — start it fresh
            // rather than let it carry into the rebuilt bank.
            state.melody = MelodyTracker::default();
            state.melody_pitch = None;
            let resonator_labels = state.resonator_labels.clone();
            if let Some(reading) = state.reading.as_mut() {
                reading.resonator_spectrum.clear();
                reading.resonator_waterfall.clear();
                reading.resonator_note_labels = resonator_labels;
                reading.fast_pitch = None;
                reading.melody_pitch = None;
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
            pitch_tracker: PitchTracker::new(),
            onset: OnsetDetector::new(),
            onset_seq: 0,
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

        // Fuse the resonator bank's fast pitch into pYIN as a low-latency candidate,
        // when the bank is running and the window carries real level. `fast_pitch`
        // is a fractional MIDI in the bank's concert-pitch frame → absolute Hz here.
        // The bank leads YIN's long window by ~100 ms, so this both quickens the
        // tracker's onset response and lets the two sources settle the octave inside
        // the HMM (the panel no longer octave-locks). Empty when the bank is parked.
        let bank_pitch = if level >= BANK_FUSE_LEVEL {
            shared
                .lock()
                .ok()
                .and_then(|s| s.fast_pitch)
                .map(|(midi, strength)| {
                    let hz = analysis_settings.concert_pitch_hz * 2.0f32.powf((midi - 69.0) / 12.0);
                    (hz, strength)
                })
        } else {
            None
        };

        // Note-onset (attack) detection off the window RMS. A new onset bumps the
        // monotonic counter the UI diffs to split re-bowed repeats of one pitch.
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        let is_onset = self.onset.detect(rms);
        if is_onset {
            self.onset_seq = self.onset_seq.wrapping_add(1);
        }

        // Pitch via probabilistic YIN (the stateful HMM tracker) — octave-robust and
        // already smoothed, so `publish_analysis_reading` no longer runs the old EMA.
        // On an onset the tracker enters its attack mode (fast bank leads the leap).
        let pitch = if rms < SILENCE_RMS_THRESHOLD {
            None
        } else {
            self.pitch_tracker
                .process(&window, self.sample_rate, bank_pitch, is_onset)
                .and_then(|(f, c)| {
                    (LOWEST_TRACKED_FREQUENCY..=HIGHEST_TRACKED_FREQUENCY)
                        .contains(&f)
                        .then_some(PitchEstimate {
                            frequency_hz: f,
                            clarity:      c,
                        })
                })
        };
        let frame = analyze_window(
            &window,
            self.sample_rate,
            &analysis_settings,
            &mut self.planner,
            pitch,
            self.onset_seq,
        );
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
    pitch: Option<PitchEstimate>,
    onset_seq: u64,
) -> AnalysisFrame {
    let (spectrum, note_spectrum, spiral_spectrum) =
        spectrum_bars_for_window(window, sample_rate, settings, planner);

    AnalysisFrame {
        pitch,
        spectrum,
        note_spectrum,
        spiral_spectrum,
        concert_pitch_hz: settings.concert_pitch_hz,
        accidental: settings.accidental,
        onset_seq,
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
        // pYIN's HMM already smooths and octave-stabilises the pitch, so we take its
        // frequency verbatim (no more EMA). `smoothed_frequency` is now just the
        // last voiced pitch, held through a brief unvoiced gap so the note doesn't
        // blink out between frames.
        let (smoothed_frequency, clarity) = match frame.pitch {
            Some(pitch) => {
                state.smoothed_frequency = Some(pitch.frequency_hz);
                (pitch.frequency_hz, pitch.clarity)
            }
            None => {
                let Some(sf) = state.smoothed_frequency else {
                    return;
                };
                (sf, 0.0)
            }
        };

        // pYIN's octave opinion for the melody line. Stored as MIDI against the
        // concert pitch it was measured with, so the 16 ms bank path can snap to it
        // without reaching for the settings. Clarity is pYIN's voiced probability,
        // which is what decides whether the opinion is worth taking at all.
        let anchor_midi = 69.0 + 12.0 * (smoothed_frequency / frame.concert_pitch_hz).log2();
        state.octave_anchor = Some((anchor_midi, clarity));
        // Re-stamp the melody pitch the bank path last computed: this path rebuilds
        // the whole reading every 40 ms, so without this it would blank the bank's
        // 16 ms value. Deliberately NOT recomputed here — `MelodyTracker`'s hysteresis
        // counts bank frames, and driving it from this path too would double-count.
        let melody_pitch = state.melody_pitch;

        let (note_name, cents) =
            frequency_to_note(smoothed_frequency, frame.concert_pitch_hz, frame.accidental);
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
            note_labels: note_bucket_labels(frame.accidental),
            fast_pitch: state.fast_pitch,
            melody_pitch,
            onset_seq: frame.onset_seq,
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
        state.fast_pitch = snapshot.fundamental;
        let resonator_spectrum = state.resonator_spectrum.clone();
        let resonator_labels = state.resonator_labels.clone();
        let fast_pitch = state.fast_pitch;
        // The melody line's whole latency win happens here: the bank publishes every
        // ~16 ms, so the played note is refreshed at the bank's cadence instead of
        // waiting for the 40 ms pYIN rebuild (which is itself ~128 ms behind). This
        // is also the only caller allowed to drive the tracker — see `melody_pitch`.
        let octave_anchor = state.octave_anchor;
        let melody_pitch = state.melody.update(fast_pitch, octave_anchor);
        state.melody_pitch = melody_pitch;
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
            reading.fast_pitch = fast_pitch;
            reading.melody_pitch = melody_pitch;
        }
    }
}
