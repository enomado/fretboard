#[cfg(not(target_arch = "wasm32"))]
mod native;
mod types;
#[cfg(target_arch = "wasm32")]
mod wasm;

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
