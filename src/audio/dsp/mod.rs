//! Target-agnostic signal analysis shared by the native and wasm audio engines.
//!
//! Pure DSP only — FFT spectrum, YIN pitch detection, the resonator bank, and
//! the note/spiral bucketing math. Nothing here touches cpal, threads, or the
//! browser; the platform engines (`audio::native`, `audio::wasm`) own capture
//! and drive these through [`crate::audio::core`]'s pipelines. The split exists
//! so wasm reuses the exact same analysis instead of a parallel copy.
pub(crate) mod analysis_math;
pub(crate) mod melody;
pub(crate) mod octave_gate;
pub(crate) mod onset;
// A simple FFT peak comb kept as a selectable yardstick. Its reference mean stays
// separate from the melody decoder. Plain comment for the same reason as `rtswipe`'s.
pub(crate) mod peak_comb;
pub(crate) mod window_frontend;
pub(crate) mod pitch;
/// RPA of the shipped scorer against a corpus with a perfect f0 annotation — the
/// baseline any detector change is argued against. Test-only, and needs the git-ignored
/// `datasets/` corpus (`docs/pitch_benchmark.md`); nothing ships from here.
#[cfg(test)]
mod pitch_bench;
pub(crate) mod pyin;
pub(crate) mod resonator;
// SWIPE′ over windowed FFTs — the same kernel as `swipe`, a different frontend from
// `resonator`; selectable as `PitchFrontend::RtSwipe`. A plain comment, not `///`: an
// outer doc here merges with the module's own `//!` and rustdoc then resolves that
// file's intra-doc links in *this* scope, where none of its names exist.
pub(crate) mod rtswipe;
pub(crate) mod segmenter;
// Harmonic sieve over spectral peaks — H6's experimental curve frontend under melody's
// shared trellis; the standalone estimator is also kept as a benchmark baseline.
// Plain comment for the same reason as `rtswipe`'s.
pub(crate) mod sieve;
pub(crate) mod spectrum;
pub(crate) mod swipe;
pub(crate) mod trellis;
// Spectral YIN's shared native/web calculation, selected through window_frontend.
pub(crate) mod yinfft;
