//! Wire protocol between the main thread and the DSP web worker (wasm only).
//!
//! Messages are postcard-encoded and shipped as `Uint8Array` over `postMessage`.
//! The main thread streams captured sample blocks + control updates to the
//! worker; the worker streams back analysis snapshots. Keeping the analysis off
//! the main thread is the whole reason the worker exists — see `audio::worker`.

use crate::audio::sample_rate::SampleRate;
use crate::audio::types::{
    AnalysisSettings,
    AudioStatus,
    MelodyFrame,
    ResonatorReading,
    TunerReading,
};

/// Main thread → worker.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum ToWorker {
    /// (Re)initialize the pipelines for a freshly opened capture at this rate.
    Init { sample_rate: SampleRate },
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
    pub(crate) frontend:  crate::audio::types::PitchFrontend,
    pub(crate) status:    AudioStatus,
    pub(crate) reading:   Option<TunerReading>,
    pub(crate) resonator: Option<ResonatorReading>,
    pub(crate) level:     f32,
    pub(crate) waveform:  Vec<f32>,
    /// The melody frames published since the **previous** snapshot — a delta, not a
    /// history.
    ///
    /// The main thread cannot read the worker's ring (no shared memory), so the ring
    /// is rebuilt up there from these. A delta rather than the whole thing because
    /// that is the entire cost argument: the history is ~600 columns × ~480 bins, so
    /// shipping it every publish would be ~70 MB/s, while the frames actually new
    /// since the last post are 0–5 columns — three orders of magnitude less. No
    /// acknowledgement is needed to make that safe: `postMessage` delivers in order,
    /// so "since the last post" is unambiguous on both ends.
    pub(crate) melody:    Vec<MelodyFrame>,
}

pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(value)
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        FromWorker,
        ToWorker,
        WorkerSnapshot,
        decode,
        encode,
    };
    use crate::audio::types::{
        AudioStatus,
        PitchFrontend,
    };

    #[test]
    fn sample_message_round_trips_without_changing_the_wire() {
        let bytes = encode(&ToWorker::Samples(vec![0.25, -0.5])).unwrap();
        // Variant index, vector length, then two little-endian f32 samples.
        assert_eq!(bytes, [1, 2, 0, 0, 128, 62, 0, 0, 0, 191]);
        let ToWorker::Samples(samples) = decode(&bytes).unwrap() else {
            panic!("decoded a different message");
        };
        assert_eq!(samples, [0.25, -0.5]);
    }

    #[test]
    fn protocol_errors_are_returned_instead_of_empty_data() {
        struct Refused;
        impl serde::Serialize for Refused {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("refused by serializer"))
            }
        }
        assert!(encode(&Refused).is_err());
        assert!(decode::<ToWorker>(&[]).is_err());
        assert!(decode::<ToWorker>(&[255]).is_err());
    }

    #[test]
    fn worker_replies_preserve_readiness_and_absent_readings() {
        let ready = encode(&FromWorker::Ready).unwrap();
        assert_eq!(ready, [0]);
        assert!(matches!(decode::<FromWorker>(&ready).unwrap(), FromWorker::Ready));
        let snapshot = FromWorker::Snapshot(Box::new(WorkerSnapshot {
            frontend:  PitchFrontend::default(),
            status:    AudioStatus::Listening,
            reading:   None,
            resonator: None,
            level:     0.25,
            waveform:  vec![-0.5, 0.25],
            melody:    Vec::new(),
        }));
        let bytes = encode(&snapshot).unwrap();
        let FromWorker::Snapshot(back) = decode(&bytes).unwrap() else {
            panic!("decoded a different reply");
        };
        assert!(matches!(back.status, AudioStatus::Listening));
        assert!(back.reading.is_none());
        assert!(back.resonator.is_none());
        assert!(back.melody.is_empty());
        assert_eq!(back.waveform, [-0.5, 0.25]);
        assert_eq!(back.level, 0.25);
    }
}
