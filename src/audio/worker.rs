//! DSP web worker entry point (wasm only).
//!
//! Runs the FFT/YIN/resonator analysis off the main thread. The main thread
//! (`audio::wasm`) captures audio and streams sample blocks here; we run the same
//! [`AnalysisPipeline`]/[`ResonatorPipeline`] the native engine uses and post the
//! resulting [`WorkerSnapshot`] back. This is what keeps the render thread smooth
//! despite wasm having no real threads — the heavy math lives in this worker.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{
    AtomicU32,
    Ordering,
};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    DedicatedWorkerGlobalScope,
    MessageEvent,
};
use web_time::{
    Duration,
    Instant,
};

use crate::audio::core::{
    AnalysisPipeline,
    ResonatorPipeline,
    SharedState,
};
use crate::audio::types::{
    AnalysisSettings,
    AudioStatus,
    ResonatorReading,
};
use crate::audio::worker_proto::{
    FromWorker,
    ToWorker,
    WorkerSnapshot,
    decode,
    encode,
};

// How long the resonator bank keeps running after the last `ResonatorWanted`
// push. Matches the native engine's gate.
const RESONATOR_PARK_GRACE: Duration = Duration::from_millis(300);

struct WorkerState {
    // `None` until the first `Init` names the capture sample rate.
    pipelines:           Option<(AnalysisPipeline, ResonatorPipeline)>,
    shared:              Arc<Mutex<SharedState>>,
    settings:            Arc<Mutex<AnalysisSettings>>,
    input_gain:          Arc<AtomicU32>,
    input_level:         Arc<AtomicU32>,
    resonator_deadline:  Instant,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            pipelines:          None,
            shared:             Arc::new(Mutex::new(SharedState::new())),
            settings:           Arc::new(Mutex::new(AnalysisSettings::default())),
            input_gain:         Arc::new(AtomicU32::new(1.0f32.to_bits())),
            input_level:        Arc::new(AtomicU32::new(0.0f32.to_bits())),
            resonator_deadline: Instant::now(),
        }
    }
}

/// Worker bootstrap: install the message handler and announce readiness. Runs in
/// the dedicated worker's global scope (the trunk loader shim calls into here).
pub fn worker_entry() {
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let state = Rc::new(RefCell::new(WorkerState::new()));

    let handler_scope = scope.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let bytes = js_sys::Uint8Array::new(&event.data()).to_vec();
        let Some(message) = decode::<ToWorker>(&bytes) else {
            return;
        };
        handle(&mut state.borrow_mut(), &handler_scope, message);
    });
    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // The closure must outlive this function (it fires for every message); the
    // worker lives for the whole session, so leaking it is correct here.
    onmessage.forget();

    post(&scope, &FromWorker::Ready);
}

fn handle(state: &mut WorkerState, scope: &DedicatedWorkerGlobalScope, message: ToWorker) {
    match message {
        ToWorker::Init { sample_rate } => {
            state.pipelines = Some((
                AnalysisPipeline::new(sample_rate),
                ResonatorPipeline::new(sample_rate),
            ));
            if let Ok(mut shared) = state.shared.lock() {
                shared.reset();
            }
        }
        ToWorker::Samples(samples) => {
            let Some((analysis, resonator)) = state.pipelines.as_mut() else {
                return;
            };
            analysis.push_samples(
                samples.iter().copied(),
                &state.shared,
                &state.settings,
                &state.input_gain,
                &state.input_level,
            );
            // Resonator gate: only run the expensive bank while the UI is asking.
            if Instant::now() < state.resonator_deadline {
                resonator.push_samples(
                    samples.iter().copied(),
                    &state.shared,
                    &state.settings,
                    &state.input_gain,
                );
            }
            let snapshot = snapshot(state);
            post(scope, &snapshot);
        }
        ToWorker::Settings(settings) => {
            if let Ok(mut guard) = state.settings.lock() {
                *guard = (*settings).sanitized();
            }
        }
        ToWorker::Gain(gain) => {
            state.input_gain.store(gain.to_bits(), Ordering::Relaxed);
        }
        ToWorker::ResonatorWanted(wanted) => {
            state.resonator_deadline = if wanted {
                Instant::now() + RESONATOR_PARK_GRACE
            } else {
                Instant::now()
            };
        }
    }
}

fn snapshot(state: &WorkerState) -> FromWorker {
    let level = f32::from_bits(state.input_level.load(Ordering::Relaxed));
    let (status, reading, resonator, waveform) = match state.shared.lock() {
        Ok(shared) => (
            shared.status.clone(),
            shared.reading.clone(),
            (!shared.resonator_spectrum.is_empty()).then(|| ResonatorReading {
                spectrum:    shared.resonator_spectrum.clone(),
                waterfall:   shared.resonator_waterfall.iter().cloned().collect(),
                note_labels: shared.resonator_labels.clone(),
            }),
            shared.input_waveform.iter().copied().collect(),
        ),
        Err(_) => (
            AudioStatus::Error("worker state poisoned".to_owned()),
            None,
            None,
            Vec::new(),
        ),
    };
    FromWorker::Snapshot(Box::new(WorkerSnapshot {
        status,
        reading,
        resonator,
        level,
        waveform,
    }))
}

fn post(scope: &DedicatedWorkerGlobalScope, message: &FromWorker) {
    let bytes = encode(message);
    let array = js_sys::Uint8Array::from(&bytes[..]);
    let _ = scope.post_message(array.as_ref());
}
