//! The engine as the platform layers wire it, driven off a sample count instead of a device.
//!
//! One rig for every test that needs the *shipped* chain rather than a DSP module alone:
//! `core`'s wiring tests and `dsp::pitch_bench`'s melody-line benchmark. It lived inside
//! `core::tests` until the benchmark needed it too; a second copy there would be a second
//! engine that could drift from the first, which is exactly what a benchmark of "what ships"
//! must not measure.

use std::sync::atomic::AtomicU32;
use std::sync::{
    Arc,
    Mutex,
};

use web_time::{
    Duration,
    Instant,
};

use crate::audio::core::{
    AnalysisPipeline,
    ResonatorPipeline,
    SharedState,
};
use crate::audio::sample_rate::SampleRate;
use crate::audio::types::{
    AnalysisSettings,
    MelodyFrame,
    NoteLine,
};

/// Both pipelines, the shared state between them, the level atomic one writes and the
/// other reads — what `native`/`wasm` build around a device stream.
pub(crate) struct Rig {
    pub(crate) analysis: AnalysisPipeline,
    pub(crate) resonator: ResonatorPipeline,
    pub(crate) shared: Arc<Mutex<SharedState>>,
    pub(crate) settings: Arc<Mutex<AnalysisSettings>>,
    pub(crate) gain: Arc<AtomicU32>,
    pub(crate) level: Arc<AtomicU32>,
    /// The rig's clock: `start` plus the audio fed so far. Stands in for the wall
    /// clock the pipelines' cadence gates read in production.
    start: Instant,
    fed: usize,
}

impl Rig {
    pub(crate) fn new(sample_rate: SampleRate) -> Self {
        let shared = Arc::new(Mutex::new(SharedState::new()));
        shared.lock().unwrap().reset();
        Self {
            analysis: AnalysisPipeline::new(sample_rate),
            resonator: ResonatorPipeline::new(sample_rate),
            shared,
            settings: Arc::new(Mutex::new(AnalysisSettings::default())),
            gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            level: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            // After the pipelines are built: each starts its gate one interval in the
            // past, so the first chunk is analysed, as it is live.
            start: Instant::now(),
            fed: 0,
        }
    }

    /// Feed both planes in [`Self::chunk`]s, as a device callback would, each stamped
    /// with the moment its last sample arrived — **audio** time, not wall time.
    ///
    /// Both pipelines gate their cadence on the clock they are handed. This rig used
    /// to pace the real one with `thread::sleep`, and a sleep only ever oversleeps:
    /// under a loaded test runner each 10 ms chunk took longer, every chunk passed
    /// the bank's 16 ms gate instead of every second one, and the frame-counted
    /// hysteresis downstream saw a different cadence — `release_ghosts_…` flaked on
    /// it (3 of 7 runs at load ~30). Stamping chunks off the sample count is the
    /// ideal capture, exactly, on any machine.
    ///
    /// The cadence that follows: the bank publishes on the first chunk at least
    /// `update_ms` after the last publish, so the default 16 ms becomes **20 ms** here —
    /// whole 10 ms chunks. A device with another buffer size gives another cadence.
    ///
    /// Chunks are cut from *this* call's samples, so a caller feeding a long take in
    /// pieces cuts the pieces on chunk boundaries to keep the cadence uniform.
    pub(crate) fn feed(&mut self, samples: &[f32], sample_rate: SampleRate) {
        for c in samples.chunks(Self::chunk(sample_rate)) {
            self.fed += c.len();
            let now = self.start + sample_rate.duration_of(self.fed);
            self.analysis.push_samples_at(
                c.iter().copied(),
                now,
                &self.shared,
                &self.settings,
                &self.gain,
                &self.level,
            );
            self.resonator.push_samples_at(
                c.iter().copied(),
                now,
                &self.shared,
                &self.settings,
                &self.gain,
                &self.level,
            );
        }
    }

    /// One device callback's worth of samples (10 ms) — the unit [`Self::feed`] pushes in.
    pub(crate) fn chunk(sample_rate: SampleRate) -> usize {
        sample_rate.samples_in(Duration::from_millis(10))
    }

    pub(crate) fn melody_since(&self, after: Option<u64>) -> Vec<MelodyFrame> {
        self.shared.lock().unwrap().melody_since(after)
    }

    pub(crate) fn note_line(&self) -> NoteLine {
        self.shared
            .lock()
            .unwrap()
            .reading
            .as_ref()
            .map(|r| r.note_line.clone())
            .unwrap_or_default()
    }
}
