//! Wire protocol between the main thread and the DSP web worker (wasm only).
//!
//! Messages are bincode-encoded and shipped as `Uint8Array` over `postMessage`.
//! The main thread streams captured sample blocks + control updates to the
//! worker; the worker streams back analysis snapshots. Keeping the analysis off
//! the main thread is the whole reason the worker exists — see `audio::worker`.

use crate::audio::types::{
    AnalysisSettings,
    AudioStatus,
    ResonatorReading,
    TunerReading,
};

/// Main thread → worker.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum ToWorker {
    /// (Re)initialize the pipelines for a freshly opened capture at this rate.
    Init { sample_rate: f32 },
    /// One contiguous block of mono samples from the capture callback.
    Samples(Vec<f32>),
    /// Analysis settings changed (already sanitized on the worker side too).
    Settings(Box<AnalysisSettings>),
    /// Input gain changed (linear).
    Gain(f32),
    /// Resonator gate: `true` keeps the (expensive) bank running for a grace
    /// period; the worker parks it when the pushes stop. Mirrors native.
    ResonatorWanted(bool),
}

/// Worker → main thread.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum FromWorker {
    /// Sent once when the worker's message handler is installed.
    Ready,
    /// Latest analysis state for the UI to read.
    Snapshot(Box<WorkerSnapshot>),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct WorkerSnapshot {
    pub(crate) status:    AudioStatus,
    pub(crate) reading:   Option<TunerReading>,
    pub(crate) resonator: Option<ResonatorReading>,
    pub(crate) level:     f32,
    pub(crate) waveform:  Vec<f32>,
}

pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    // Infallible for our plain data types; fall back to empty on the impossible
    // error so a hiccup never panics an audio callback.
    bincode::serialize(value).unwrap_or_default()
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    bincode::deserialize(bytes).ok()
}
