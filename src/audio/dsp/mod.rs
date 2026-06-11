//! Target-agnostic signal analysis shared by the native and wasm audio engines.
//!
//! Pure DSP only — FFT spectrum, YIN pitch detection, the resonator bank, and
//! the note/spiral bucketing math. Nothing here touches cpal, threads, or the
//! browser; the platform engines (`audio::native`, `audio::wasm`) own capture
//! and drive these through [`crate::audio::core`]'s pipelines. The split exists
//! so wasm reuses the exact same analysis instead of a parallel copy.
pub(crate) mod analysis_math;
pub(crate) mod pitch;
pub(crate) mod resonator;
pub(crate) mod spectrum;
