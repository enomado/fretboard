//! UI elements that draw themselves non-trivially each live in their own module.
//! Each is a pure renderer (it paints from a borrowed snapshot, owns no state):
//!   - `fretboard` — the neck model and its note/string/fret/position renderers,
//!   - `snail`     — the logarithmic pitch spiral,
//!   - `waterfall` — spectrogram-style history strips,
//!   - `theme`     — shared colours and the egui style.
//!
//! Callers reach into these with explicit paths (e.g. `crate::ui::snail::draw_spiral_chart`)
//! rather than via re-exports, so it stays obvious where each renderer lives.

pub mod fretboard;
pub mod snail;
pub mod theme;
pub mod waterfall;
