// Platform-agnostic analysis, shared by both engines (see `core`/`dsp`).
mod core;
// Private in a real build: the panels read finished values off `TunerReading` and
// have no business reaching into the DSP. Opened up under `cfg(test)` only, so the
// latency regression tests in `app::staff_panel` can drive the real bank + tracker
// end to end — the absence of exactly that test is how the melody line came to be
// driven from pYIN alone (128 ms) without anyone noticing.
#[cfg(test)]
pub(crate) mod dsp;
#[cfg(not(test))]
mod dsp;
mod types;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;
// DSP web worker (runs the analysis off the main thread) + its wire protocol.
#[cfg(target_arch = "wasm32")]
mod worker;
#[cfg(target_arch = "wasm32")]
mod worker_proto;

#[cfg(not(target_arch = "wasm32"))]
pub use native::imp::AudioEngine;
pub use types::{
    AnalysisSettings,
    ArpPattern,
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
    DroneMode,
    DroneState,
    ResonatorReading,
    ResonatorSettings,
    Timbre,
    TunerReading,
};
#[cfg(target_arch = "wasm32")]
pub use wasm::AudioEngine;
// Entry point for the DSP worker binary (`src/bin/dsp_worker.rs`).
#[cfg(target_arch = "wasm32")]
pub use worker::worker_entry;
