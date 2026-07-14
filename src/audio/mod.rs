// Platform-agnostic analysis, shared by both engines (see `core`/`dsp`).
mod core;
// Private, in every build: the panels read finished values off `TunerReading` and
// have no business reaching into the DSP.
//
// It used to be `pub(crate)` under `cfg(test)` so the end-to-end latency probe could
// live in `app::staff_panel` and drive the real bank + tracker from there. It no
// longer has to reach across: note segmentation moved into `dsp::segmenter`, so the
// whole path the probe measures — bank → melody → segmenter — is now inside `dsp`,
// and the probe sits next to it. The test that must never be missing is still there
// (its absence is how the melody line came to be driven from pYIN alone at 128 ms
// without anyone noticing); it simply no longer needs a hole in the wall.
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
    MelodyFrame,
    MelodyHistory,
    NoteLine,
    ResonatorReading,
    ResonatorSettings,
    StaffNote,
    Timbre,
    TunerReading,
};
#[cfg(target_arch = "wasm32")]
pub use wasm::AudioEngine;
// Entry point for the DSP worker binary (`src/bin/dsp_worker.rs`).
#[cfg(target_arch = "wasm32")]
pub use worker::worker_entry;
