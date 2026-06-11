// Platform-agnostic analysis, shared by both engines (see `core`/`dsp`).
mod core;
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
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
    ResonatorReading,
    ResonatorSettings,
    TunerReading,
};
#[cfg(target_arch = "wasm32")]
pub use wasm::AudioEngine;
// Entry point for the DSP worker binary (`src/bin/dsp_worker.rs`).
#[cfg(target_arch = "wasm32")]
pub use worker::worker_entry;
