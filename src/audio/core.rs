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
use crate::audio::dsp::rtswipe::RtSwipe;
use crate::audio::dsp::segmenter::NoteSegmenter;
use crate::audio::dsp::spectrum::spectrum_bars_for_window;
use crate::audio::dsp::swipe::SalienceFrame;
use crate::audio::types::{
    AnalysisSettings,
    AudioStatus,
    MelodyFrame,
    MelodyHistory,
    NoteLine,
    PitchFrontend,
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
/// Below this normalized input level the melody line is **silent**.
///
/// This is the real silence gate, and it has to be an *absolute* level: the engine
/// never declares silence itself (`SILENCE_RMS_THRESHOLD == 0.0`), and the bank's
/// column is normalized to its own max, so it reports *some* fundamental for room
/// noise. Nothing downstream can reconstruct silence from the bank's output alone.
///
/// It lives here, and not in the panels, because the melody line's segmentation is
/// decided here now: a decision needs to know when the sound stopped. It was
/// duplicated verbatim in both melody panels, which also meant the `MelodyTracker`
/// upstream ran its leap/slip hysteresis on room noise through every rest — the
/// panels hid that by gating the *result*, but the tracker's state had already
/// absorbed it. Gating the input instead means silence properly ends the phrase.
const MELODY_LEVEL_GATE: f32 = 0.02;

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
    /// Cuts the melody line into written notes. Like [`Self::melody`], driven **only**
    /// by the bank's publish path — its note timers are specified in seconds, so the
    /// cadence it is called at is what fixes their timescale. See [`NoteSegmenter`].
    pub(crate) segmenter:           NoteSegmenter,
    /// Last line [`Self::segmenter`] produced, re-stamped by the 40 ms path for the
    /// same reason as [`Self::melody_pitch`].
    pub(crate) note_line:           NoteLine,
    /// The engine's monotonic attack counter, written by the 40 ms analysis path and
    /// read by the 16 ms bank path (which is what feeds it to [`Self::segmenter`]).
    ///
    /// Here rather than only on the reading because the two planes run at different
    /// cadences on different threads — the same reason [`Self::octave_anchor`] is.
    pub(crate) onset_seq:           u64,
    /// Every bank frame of the melody line, for the panels that draw its *history*
    /// rather than its instant — read with [`SharedState::melody_since`].
    ///
    /// It exists because a panel sampling `melody_pitch` once per UI frame does not
    /// see the melody: the bank publishes at ~62 Hz, so a 60 fps panel silently drops
    /// a few percent of its frames and a 30 fps one drops **half**. The trills and
    /// vibrato that are the whole point of the fast path were being decimated by the
    /// renderer's clock. Same rule as `dsp::melody` and `dsp::segmenter` (see
    /// `note_detection.md` §4), one layer further out: what the audio produced is not
    /// the UI's to sample.
    pub(crate) melody_history:      MelodyHistory,
    /// Next [`MelodyFrame::seq`]. Monotonic for the life of the engine, and NOT
    /// cleared by [`Self::reset`] — see the field's docs: it answers a delivery
    /// question, and a consumer's cursor outlives any one stream. `MelodyFrame::t` is
    /// the one that restarts, because it is about the audio.
    pub(crate) melody_seq:          u64,
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
            segmenter:           NoteSegmenter::default(),
            note_line:           NoteLine::default(),
            onset_seq:           0,
            melody_history:      MelodyHistory::default(),
            melody_seq:          0,
            smoothed_frequency:  None,
        }
    }

    /// The melody line's frames newer than `after`, oldest → newest — see
    /// [`MelodyHistory::since`] for why this is a cursor rather than a drain.
    pub(crate) fn melody_since(&self, after: Option<u64>) -> Vec<MelodyFrame> {
        self.melody_history.since(after)
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
        self.segmenter = NoteSegmenter::default();
        self.note_line = NoteLine::default();
        self.onset_seq = 0;
        // The frames go: they were heard through the previous device, and the new
        // stream's sample clock starts over, so keeping them would splice two
        // unrelated moments together. `melody_seq` deliberately does NOT go with
        // them — a panel's cursor survives the switch, and rewinding the counter
        // would make every frame of the new stream look already-seen to it.
        self.melody_history.clear();
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
    /// The alternative note frontend (`PitchFrontend::RtSwipe`). Fed the **same** samples as
    /// the bank on every push so its rolling history stays warm and a switch is instant, but
    /// only *scored* (`frame()` runs eight FFTs) at publish time when it is the active
    /// frontend — so the parallel run costs a memcpy per push while the bank is selected.
    rtswipe:      RtSwipe,
    /// The A4 `rtswipe` was built against. Its ladder and grid are cut from the concert
    /// pitch, so a retune has to rebuild it — see [`Self::sync_settings`], which mirrors the
    /// bank's own rebuild-on-settings-change.
    rtswipe_ref:  f32,
    last_publish: Instant,
    /// The **audio clock**: samples this pipeline has actually processed, and the rate
    /// to read them at. `samples_seen / sample_rate` is the timestamp handed to the
    /// melody segmenter, and it is the reason a note's duration is now a musical
    /// quantity rather than a count of renderer ticks (see `dsp::segmenter`).
    ///
    /// Counts *processed* samples, so it stalls while the bank is parked (no consumer
    /// asking) — deliberately: while parked nothing downstream of it runs either, and
    /// a clock that ran on regardless would expire the held note of whatever phrase
    /// was playing when the panel was closed. It is monotonic whenever it is read.
    sample_rate:  f32,
    samples_seen: u64,
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
        // A4 = 440 until the first settings sync tells us the user's concert pitch; the bank
        // starts the same way (`ResonatorViewSettings::default`), and `sync_settings` rebuilds
        // both the instant a real value arrives.
        let rtswipe_ref = 440.0;
        Self {
            analyzer: ResonatorAnalyzer::new(sample_rate),
            rtswipe: RtSwipe::new(sample_rate, rtswipe_ref),
            rtswipe_ref,
            last_publish: Instant::now() - Duration::from_millis(16),
            sample_rate,
            samples_seen: 0,
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
        self.sync_settings(&analysis_settings, shared);

        let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
        let samples: Vec<f32> = samples.into_iter().map(|sample| sample * gain).collect();
        self.samples_seen += samples.len() as u64;
        self.analyzer
            .process_samples(&samples, analysis_settings.resonator.reassign);
        // Keep RT-SWIPE's rolling history warm on every push, whichever frontend is active:
        // a switch must be able to score the very next frame, and this is only a memcpy. The
        // eight FFTs of `frame()` are deferred to publish, and only when it is the frontend.
        self.rtswipe.process_samples(&samples);

        let publish_interval = Duration::from_millis(analysis_settings.resonator.update_ms);
        if self.last_publish.elapsed() < publish_interval {
            return;
        }
        self.last_publish = Instant::now();
        let frontend = analysis_settings.resonator.frontend;
        // The note frontend RT-SWIPE offers this frame — scored only when it is the one that
        // will decide the note, so the bank path pays nothing for it. The bank's own snapshot
        // still drives every display layer regardless (the parallel-run the user chose).
        let rtswipe_frame = (frontend == PitchFrontend::RtSwipe)
            .then(|| self.rtswipe.frame())
            .flatten();
        // The level the 40 ms analysis plane last measured. Up to one analysis hop
        // stale, which is exactly what the panels' own gate read before it moved here
        // — same atomic, same staleness, one copy of the rule instead of two.
        let level = f32::from_bits(input_level.load(Ordering::Relaxed));
        publish_resonator_snapshot(
            shared,
            self.analyzer
                .snapshot(analysis_settings.resonator.reassign, analysis_settings.accidental),
            rtswipe_frame,
            frontend,
            analysis_settings.resonator.history,
            level,
            self.samples_seen as f64 / self.sample_rate as f64,
        );
    }

    fn sync_settings(&mut self, settings: &AnalysisSettings, shared: &Arc<Mutex<SharedState>>) {
        let requested = ResonatorViewSettings::from(settings);
        if !self.analyzer.sync_settings(requested) {
            return;
        }
        // A retune reaches RT-SWIPE the same way it reaches the bank: its ladder and grid are
        // cut from A4, so a new concert pitch means a new analyser. The bank only reports a
        // change (returns true above) when a `ResonatorViewSettings` field moved, and the
        // concert pitch is one of them, so this rides the same gate — and toggling the
        // *frontend* alone never gets here, which is right: no grid changed.
        if settings.concert_pitch_hz != self.rtswipe_ref {
            self.rtswipe = RtSwipe::new(self.sample_rate, settings.concert_pitch_hz);
            self.rtswipe_ref = settings.concert_pitch_hz;
        }
        if let Ok(mut state) = shared.lock() {
            state.resonator_spectrum.clear();
            state.resonator_waterfall.clear();
            state.resonator_labels = self.analyzer.note_labels(settings.accidental);
            state.fast_pitch = None;
            // The bank's grid just changed under us, so the melody line's octave
            // dispute is about a reading that no longer exists — start it fresh
            // rather than let it carry into the rebuilt bank. The same goes for the
            // note being held: it was heard through the old grid, and its timers are
            // about to be stamped from a clock that kept running across the rebuild.
            state.melody = MelodyTracker::default();
            state.melody_pitch = None;
            state.segmenter = NoteSegmenter::default();
            state.note_line = NoteLine::default();
            // The heat columns in the history are the old grid's: a different length,
            // and a different bin→pitch mapping. A panel drawing them against the new
            // grid's `min_midi..max_midi` would paint them at the wrong pitch.
            state.melody_history.clear();
            let resonator_labels = state.resonator_labels.clone();
            if let Some(reading) = state.reading.as_mut() {
                reading.resonator_spectrum.clear();
                reading.resonator_waterfall.clear();
                reading.resonator_note_labels = resonator_labels;
                reading.fast_pitch = None;
                reading.melody_pitch = None;
                reading.note_line = NoteLine::default();
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

        // Note-onset (attack) detection off the window RMS. A new onset bumps the
        // monotonic counter the UI diffs to split re-bowed repeats of one pitch.
        //
        // This drives *segmentation only*. It used to be handed to the pitch tracker
        // as well, to put it in an "attack mode" that let the bank lead a leap — but
        // that mode fed the bank's octave straight back to `MelodyTracker` as pYIN's
        // supposedly independent opinion of it. See `PitchTracker::process`.
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        let is_onset = self.onset.detect(rms);
        if is_onset {
            self.onset_seq = self.onset_seq.wrapping_add(1);
        }

        // Pitch via probabilistic YIN (the stateful HMM tracker) — octave-robust and
        // already smoothed, so `publish_analysis_reading` no longer runs the old EMA.
        let pitch = if rms < SILENCE_RMS_THRESHOLD {
            None
        } else {
            self.pitch_tracker
                .process(&window, self.sample_rate)
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
        // Hand the attack counter to the bank path, which is what feeds it to the
        // segmenter. Onsets are found on this plane (off the window RMS) but consumed
        // on that one, so like `octave_anchor` the value has to cross between them.
        state.onset_seq = frame.onset_seq;
        // Re-stamp what the bank path last computed: this path rebuilds the whole
        // reading every 40 ms, so without this it would blank the bank's 16 ms values.
        // Deliberately NOT recomputed here — the hysteresis in `MelodyTracker` and the
        // note timers in `NoteSegmenter` are both counted in *bank* frames, and driving
        // them from this path too would double-count every one of them.
        let melody_pitch = state.melody_pitch;
        let note_line = state.note_line.clone();

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
            note_line,
        });
        state.status = AudioStatus::Listening;
    }
}

/// Publish one bank frame: the heat column, the melody line's note, and the written
/// line the segmenter cuts out of it.
///
/// `level` is the absolute input level (the silence gate — see [`MELODY_LEVEL_GATE`]);
/// `now_seconds` is the **audio** clock, off the sample count. This is the only place
/// allowed to drive `melody`/`segmenter`, because both count their timescales in the
/// bank frames this function publishes.
// `melody_override`: the RT-SWIPE frame to decode the note from, or `None` to use the bank's
// own (`snapshot.salience`). `Some` only when `frontend` is `RtSwipe`. The frontend tag rides
// alongside because it is what selects the β (`melody::salience_beta`), not inferable from the
// frame — the two frontends' curves are on different scales (see `dsp::rtswipe`).
fn publish_resonator_snapshot(
    shared: &Arc<Mutex<SharedState>>,
    snapshot: ResonatorSnapshot,
    melody_override: Option<SalienceFrame>,
    frontend: PitchFrontend,
    history_len: usize,
    level: f32,
    now_seconds: f64,
) {
    if let Ok(mut state) = shared.lock() {
        state.resonator_spectrum = snapshot.spectrum;
        state.resonator_labels = snapshot.note_labels;
        state.fast_pitch = snapshot.fundamental;
        let resonator_spectrum = state.resonator_spectrum.clone();
        let resonator_labels = state.resonator_labels.clone();
        // `fast_pitch` stays on the reading raw, octave and all — it is the bank's own
        // reading. What the melody is built from is gated: below the gate the bank is
        // reporting the shape of room noise, and feeding that to the tracker keeps its
        // hysteresis alive through every rest.
        //
        // The melody is handed the frame's whole **salience curve**, not the argmax above:
        // the errors left on a real violin are 4–6% near-ties, and a scalar cannot express a
        // tie for continuity to break. See `dsp::melody::SalienceDecoder`.
        let fast_pitch = state.fast_pitch;
        // The salience the note is decoded from: RT-SWIPE's frame when it is the chosen
        // frontend, the bank's own otherwise. `fast_pitch` above stays the bank's raw argmax
        // regardless — it is the bank's reading for the tuner and the octave cross-check, not
        // the melody's decision.
        let melody_source = melody_override.as_ref().or(snapshot.salience.as_ref());
        let bank = (level >= MELODY_LEVEL_GATE).then_some(melody_source).flatten();
        // The melody line's whole latency win happens here: the bank publishes every
        // ~16 ms, so the played note is refreshed at the bank's cadence instead of
        // waiting for the 40 ms pYIN rebuild (which is itself ~128 ms behind). This
        // is also the only caller allowed to drive the tracker — see `melody_pitch`.
        //
        // `now_seconds` is the **audio** clock, and the tracker's Viterbi measures its frame
        // length off it. That is not incidental: the bank's publish cadence is a user-facing
        // slider (`ResonatorSettings::update_ms`, 8..80 ms), and a continuity model with
        // per-frame costs would hand that slider the detector's smoothing — see
        // `dsp::trellis`.
        let octave_anchor = state.octave_anchor;
        let melody_pitch = state.melody.update(bank, octave_anchor, now_seconds, frontend);
        state.melody_pitch = melody_pitch;
        // …and the note the melody line is sounding is cut into written notes right
        // here too, on the sample clock. `None` covers silence and a rejected slip
        // alike, which is what the segmenter's release grace is built to absorb.
        let onset_seq = state.onset_seq;
        let note_line = state
            .segmenter
            .update(melody_pitch.map(|(midi, _)| midi), onset_seq, now_seconds);
        state.note_line = note_line.clone();
        // Everything this frame decided, kept as one record for the panels that draw a
        // history. Stamped with the SAMPLE clock — the same `now_seconds` the segmenter
        // just measured the note with, so the line a panel plots and the notes the
        // engine wrote are on one ruler. See `MelodyFrame::t` for why not `seq`.
        let seq = state.melody_seq;
        state.melody_seq += 1;
        state.melody_history.push(MelodyFrame {
            seq,
            t: now_seconds,
            pitch: melody_pitch.map(|(midi, _)| midi),
            level,
            heat: resonator_spectrum.clone(),
            // The scorer's own view of this same column, for the roll's debug layer. Taken
            // from the snapshot rather than re-scored: re-running SWIPE′ for the picture
            // would make the picture a *different* frame's evidence, drawn under a line it
            // did not decide — which is the exact failure the 1:1 alignment above exists to
            // rule out.
            // Raw, exactly like `heat` above: the display gate is the panel's
            // (`HEAT_LEVEL_GATE`), not the engine's. `MELODY_LEVEL_GATE` gated the copy the
            // *decoder* saw and has no business deciding a picture.
            salience: snapshot.salience_heat,
        });
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
            reading.note_line = note_line;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// A bowed-string-ish tone: a fundamental plus the partials that make the bank's
    /// harmonic scoring do real work.
    fn violin_tone(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        use std::f32::consts::TAU;
        let partials = [1.0f32, 0.8, 0.6, 0.35, 0.2];
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                partials
                    .iter()
                    .enumerate()
                    .map(|(h, a)| a * (TAU * frequency_hz * (h + 1) as f32 * t).sin())
                    .sum::<f32>()
                    / 3.0
            })
            .collect()
    }

    /// The engine as the platform layers actually wire it: both pipelines, the shared
    /// state between them, the level atomic one writes and the other reads.
    struct Rig {
        analysis:  AnalysisPipeline,
        resonator: ResonatorPipeline,
        shared:    Arc<Mutex<SharedState>>,
        settings:  Arc<Mutex<AnalysisSettings>>,
        gain:      Arc<AtomicU32>,
        level:     Arc<AtomicU32>,
    }

    impl Rig {
        fn new(sample_rate: f32) -> Self {
            let shared = Arc::new(Mutex::new(SharedState::new()));
            shared.lock().unwrap().reset();
            Self {
                analysis: AnalysisPipeline::new(sample_rate),
                resonator: ResonatorPipeline::new(sample_rate),
                shared,
                settings: Arc::new(Mutex::new(AnalysisSettings::default())),
                gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
                level: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            }
        }

        /// Feed both planes at ~real time, as the two workers do.
        ///
        /// The sleep is load-bearing, not politeness: both pipelines gate their cadence
        /// on `Instant::elapsed`, so audio shovelled in with no wall-clock passing would
        /// be analysed once and never again. Sleeping per chunk keeps wall time ≥ audio
        /// time, which is the condition the real capture always satisfies.
        fn feed(&mut self, samples: &[f32], sample_rate: f32) {
            let chunk = (sample_rate / 100.0) as usize; // 10 ms, as a device callback would
            for c in samples.chunks(chunk) {
                self.analysis.push_samples(
                    c.iter().copied(),
                    &self.shared,
                    &self.settings,
                    &self.gain,
                    &self.level,
                );
                self.resonator.push_samples(
                    c.iter().copied(),
                    &self.shared,
                    &self.settings,
                    &self.gain,
                    &self.level,
                );
                thread::sleep(Duration::from_secs_f32(c.len() as f32 / sample_rate));
            }
        }

        fn melody_since(&self, after: Option<u64>) -> Vec<MelodyFrame> {
            self.shared.lock().unwrap().melody_since(after)
        }

        fn note_line(&self) -> NoteLine {
            self.shared
                .lock()
                .unwrap()
                .reading
                .as_ref()
                .map(|r| r.note_line.clone())
                .unwrap_or_default()
        }
    }

    /// DIAGNOSTIC: does the engine put notes on the line for a **real violin**, through the
    /// real wiring?
    ///
    /// [`engine_writes_a_played_note_end_to_end`] proves the wiring with a synthetic tone
    /// that starts instantly at full amplitude and never stops. A bowed string does none of
    /// that — it has an attack, a level that moves with the bow, and rests between strokes.
    /// So a staff that stays empty on the instrument is not something that test can see, and
    /// "empty" is not a report anyone can act on: it does not say whether the audio arrived,
    /// whether the bank scored it, whether the decoder rejected it, or whether the segmenter
    /// threw the note away as too short. This prints each of those, at its own boundary.
    ///
    /// Not an assertion of a number: the takes are ground truth for the *octave decision*
    /// (`testdata/README.md`), and the counts here depend on the bow. It fails only on zero
    /// — which is the one reading that means the pipeline is broken rather than picky.
    #[test]
    #[ignore = "needs testdata/*.wav — run with --ignored --nocapture"]
    fn real_violin_through_the_whole_engine() {
        let path = format!("{}/testdata/g_open_slow_strokes.wav", env!("CARGO_MANIFEST_DIR"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let sr = reader.spec().sample_rate as f32;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let mut rig = Rig::new(sr);
        rig.feed(&samples, sr);

        // Only the last `MELODY_HISTORY_SECONDS` survive here — the engine's ring is a
        // bridge between two panel reads, not an archive. That is enough to answer "is
        // anything coming out at all".
        let frames = rig.melody_since(None);
        let voiced = frames.iter().filter(|f| f.pitch.is_some()).count();
        let scored = frames.iter().filter(|f| f.salience.is_some()).count();
        let line = rig.note_line();

        println!("\n=== g_open_slow_strokes through the real engine ===");
        println!("  audio fed          : {:.1} s", samples.len() as f32 / sr);
        println!("  frames (last ~2 s) : {}", frames.len());
        println!("  ...scored by SWIPE′: {scored}");
        println!("  ...with a pitch    : {voiced}");
        println!("  written notes      : {}", line.history.len());
        println!("  still sounding     : {:?}", line.current.map(|n| n.midi));

        assert!(
            !frames.is_empty(),
            "the bank published nothing at all — audio never arrived"
        );
        assert!(
            voiced > 0,
            "the engine heard a violin for {:.1} s and decided no pitch, ever",
            samples.len() as f32 / sr
        );
    }

    /// REGRESSION: a played note reaches `TunerReading::note_line` through the REAL
    /// engine wiring — and is committed to the written line when it stops.
    ///
    /// The unit tests either exercise `NoteSegmenter` alone or compose the DSP modules
    /// by hand. Neither touches what this change actually rearranged: the level atomic
    /// the analysis plane writes and the bank plane reads, the onset counter crossing
    /// between them via `SharedState`, the sample clock, and `note_line` being stamped
    /// by both publish paths without one blanking the other. A wiring mistake in any of
    /// those is invisible to every other test in the suite and total to the user — the
    /// staff would simply stay empty.
    #[test]
    fn engine_writes_a_played_note_end_to_end() {
        let sr = 48_000.0f32;
        let mut rig = Rig::new(sr);

        // Play A4 (69) long enough to clear the analysis window (the level gate cannot
        // open until the first 6144-sample window has been measured) and MIN_NOTE.
        rig.feed(&violin_tone(440.0, sr, (sr * 0.4) as usize), sr);
        let line = rig.note_line();
        assert_eq!(
            line.current.map(|n| n.midi),
            Some(69),
            "the engine should be holding A4; note_line = {line:?}"
        );

        // Bow off. The melody eventually goes to `None` and the release grace expires —
        // on the SAMPLE clock, which only advances because we keep feeding.
        //
        // 0.7 s, not the ~0.2 s the grace period alone would need, because the silence
        // gate is slow to close: `smoothed_level` falls at 0.88/frame, so `level` takes
        // ~500 ms to decay below `MELODY_LEVEL_GATE` after the sound has actually
        // stopped. Everything the bank invents in that window is passed through as
        // melody — see `release_ghosts_are_written_after_a_note` for what that costs.
        rig.feed(&vec![0.0f32; (sr * 0.7) as usize], sr);
        let line = rig.note_line();
        assert!(line.current.is_none(), "the note should have been released");
        assert!(
            line.history.iter().any(|n| n.midi == 69),
            "A4 should have been written to the line; history = {:?}",
            line.history
        );
    }

    /// REGRESSION: selecting the **RT-SWIPE** frontend routes its frame through the real
    /// engine wiring to the melody line — and the bank keeps driving the display alongside it.
    ///
    /// The twin of [`engine_writes_a_played_note_end_to_end`] with one field flipped, because
    /// the setting has a whole path to travel that no dsp test sees: `push_samples` reads
    /// `resonator.frontend`, scores `RtSwipe::frame()` in parallel with the bank, hands *that*
    /// frame to `melody.update` with the RT-SWIPE β, and still publishes the bank's snapshot
    /// for the waterfall. A break anywhere on that path is invisible below `core` and total to
    /// a user who picked RT-SWIPE — the staff would stay empty while the spectrum still moved.
    #[test]
    fn engine_decodes_with_the_rtswipe_frontend() {
        let sr = 48_000.0f32;
        let mut rig = Rig::new(sr);
        rig.settings.lock().unwrap().resonator.frontend = PitchFrontend::RtSwipe;

        // Same A4 as the bank twin. RT-SWIPE reads A4 in ~9 ms, so 0.4 s is ample.
        rig.feed(&violin_tone(440.0, sr, (sr * 0.4) as usize), sr);
        let line = rig.note_line();
        assert_eq!(
            line.current.map(|n| n.midi),
            Some(69),
            "the RT-SWIPE frontend should be holding A4; note_line = {line:?}"
        );

        // The bank runs in parallel and still feeds the display: the waterfall is the bank's
        // even while the note is RT-SWIPE's. If this is empty the parallel run is not
        // happening, i.e. picking RT-SWIPE silently killed the spectrum.
        let has_display = rig
            .shared
            .lock()
            .unwrap()
            .reading
            .as_ref()
            .is_some_and(|r| !r.resonator_spectrum.is_empty());
        assert!(has_display, "the bank stopped driving the display under RT-SWIPE");
    }

    /// REGRESSION: the melody history is published on the **audio** clock, and a
    /// consumer's cursor hands it over exactly once, in order.
    ///
    /// This is the wiring the panels' history now rests on, and like every other
    /// `core::*` test here it exists because the DSP suite cannot see it: the frames
    /// are stamped in `publish_resonator_snapshot` from the resonator pipeline's
    /// sample count, delivered through `SharedState`, and read by a cursor the panel
    /// owns. Nothing below this level touches any of that.
    ///
    /// What it pins, and why each one is a mistake somebody could actually make:
    /// - **`t` rides the sample clock, not the wall clock.** The publish is *gated* on
    ///   the wall clock, so frames land ~`update_ms` apart with real jitter; if `t`
    ///   were read off `Instant` (or inferred from `seq`), the axis would be wrong in
    ///   a way no unit test of the ring could see.
    /// - **Each frame is handed out once.** A cursor that mis-compares would either
    ///   replay frames (the roll would stutter and double its trill) or skip them.
    /// - **Two cursors are independent.** The staff's trail is the next consumer, and
    ///   a *drain* would have it steal frames from the roll.
    #[test]
    fn the_melody_history_is_published_on_the_audio_clock() {
        let sr = 48_000.0f32;
        let mut rig = Rig::new(sr);
        rig.feed(&violin_tone(440.0, sr, (sr * 0.5) as usize), sr);

        let all = rig.melody_since(None);
        assert!(
            all.len() >= 20,
            "0.5 s at the bank's ~16 ms cadence should be ~30 frames, got {}",
            all.len()
        );

        // Ordered, unique, and strictly advancing in time — the ring's invariant, seen
        // from outside, through the real pipeline rather than a hand-built ring.
        assert!(
            all.windows(2).all(|w| w[1].seq > w[0].seq && w[1].t > w[0].t),
            "history must be strictly increasing in both seq and t"
        );

        // The audio clock is the ruler: 0.5 s of samples must read as ~0.5 s of `t`,
        // regardless of how the worker's wall-clock gate happened to space the
        // publishes. Loose bounds — the point is that it tracks the audio at all, not
        // that it is exact to the frame.
        let played = all.last().unwrap().t - all.first().unwrap().t;
        assert!(
            (0.35..=0.5).contains(&played),
            "0.5 s of audio produced {played:.3} s of history — `t` is not on the \
             sample clock"
        );

        // A cursor hands each frame over exactly once…
        let cursor = all[all.len() / 2].seq;
        let rest = rig.melody_since(Some(cursor));
        assert!(
            rest.iter().all(|f| f.seq > cursor),
            "melody_since handed back a frame the caller already had"
        );
        assert_eq!(
            rest.len(),
            all.iter().filter(|f| f.seq > cursor).count(),
            "melody_since skipped frames past the cursor"
        );
        // …and reading with one cursor must not consume the history for another: this
        // is a cursor, not a drain, precisely so a second panel can exist.
        assert_eq!(
            rig.melody_since(None).len(),
            all.len(),
            "reading with one cursor emptied the history for everyone else"
        );
    }

    /// A KNOWN BUG, pinned so it cannot be lost or silently "fixed" by accident.
    ///
    /// Every note is followed onto the written line by one or more **ghosts** — notes
    /// nobody played. Two mechanisms have to line up for it, and both are upstream of
    /// note segmentation (this test predates none of them; the move of the segmenter
    /// into the engine is what made it *visible*, not what caused it):
    ///
    /// 1. **The silence gate closes ~500 ms late.** `input_level` is the *smoothed*
    ///    level — `smoothed_level` uses alpha 0.12 falling, i.e. ×0.88 per 40 ms frame,
    ///    so after the sound stops it takes ~30 frames to fall from ~0.93 through
    ///    `MELODY_LEVEL_GATE`. That smoothing exists so the UI's level *meter* does not
    ///    flicker; the melody gate inherited it by sharing the atomic.
    /// 2. **The bank cannot be quiet.** Its column is normalized to its own max, so
    ///    once the note stops it reports whichever bins ring longest — the bottom of
    ///    the grid as the low resonators decay slowest, then noise — at full
    ///    confidence. `OctaveGate` rejects the first few frames, then its median moves
    ///    onto the garbage (~50 ms) and passes it.
    ///
    /// So for ~400 ms the gate is open, the bank is inventing pitches, and each one
    /// that holds for `MIN_NOTE_SECONDS` is written. Measured here: A4 → ghosts at MIDI
    /// 16, 12, 13, 80, 76 on an instant cut.
    ///
    /// **How bad this is live is not yet known**, and this test cannot say: it cuts the
    /// tone off instantly, which no instrument does. A real note decays over hundreds
    /// of ms, so the bank may keep hearing the true pitch all the way down and the two
    /// may fall together. That is a question for the instrument, not for reasoning —
    /// see the plan's handoff.
    #[test]
    fn release_ghosts_are_written_after_a_note() {
        let sr = 48_000.0f32;
        let mut rig = Rig::new(sr);
        rig.feed(&violin_tone(440.0, sr, (sr * 0.4) as usize), sr);
        rig.feed(&vec![0.0f32; (sr * 0.7) as usize], sr);

        let ghosts: Vec<i32> = rig
            .note_line()
            .history
            .iter()
            .map(|n| n.midi)
            .filter(|&m| m != 69)
            .collect();
        assert!(
            !ghosts.is_empty(),
            "no ghosts — if this is genuinely fixed, delete this test and the plan's \
             entry for it rather than loosening the assert"
        );
        println!("release ghosts after a single A4: {ghosts:?}");
    }

    /// REGRESSION: room noise must not write ghost notes.
    ///
    /// The silence gate moved out of the panels and into `publish_resonator_snapshot`
    /// with this change, so this is the test that it is still connected at all. It has
    /// to be an *absolute* level check: the bank's column is normalized to its own max,
    /// so it reports a confident-looking fundamental for near-silence, and `fast_pitch`
    /// alone can never distinguish the two. Ghost notes off the noise floor are not
    /// hypothetical — they are what Phase 1.1 was reported for.
    #[test]
    fn the_silence_gate_keeps_room_noise_off_the_line() {
        let sr = 48_000.0f32;
        let mut rig = Rig::new(sr);

        // A tone far below the gate: the bank will still find a "fundamental" in it.
        let noise: Vec<f32> = violin_tone(440.0, sr, (sr * 0.6) as usize)
            .iter()
            .map(|s| s * 0.0005)
            .collect();
        rig.feed(&noise, sr);

        let state = rig.shared.lock().unwrap();
        let level = f32::from_bits(rig.level.load(Ordering::Relaxed));
        assert!(
            level < MELODY_LEVEL_GATE,
            "the rig must actually be below the gate"
        );
        assert!(
            state.melody_pitch.is_none(),
            "room noise reached the melody line at level {level}"
        );
        let line = &state.reading.as_ref().unwrap().note_line;
        assert!(
            line.current.is_none() && line.history.is_empty(),
            "room noise wrote a ghost note: {line:?}"
        );
    }
}
