//! Fretboard — the geometric model of the instrument neck and its renderers.
//!
//! Every piece is reached through an explicit path (no re-exports), so it stays
//! obvious where each one lives:
//!   - [`geometry`]  — the neck model: musical coordinates → screen pixels,
//!   - [`draw`]      — note marks, string lines, fret lines and inlays,
//!   - [`positions`] — the cello-position bracket overlay.

pub mod draw;
pub mod geometry;
pub mod positions;
