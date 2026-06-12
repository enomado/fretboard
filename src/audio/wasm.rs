//! Main-thread Web Audio capture for wasm — the browser counterpart of
//! `audio::native`.
//!
//! cpal does not run in the browser, so capture goes through Web Audio:
//! `getUserMedia` (microphone) or `getDisplayMedia` (system / tab audio,
//! best-effort, Chromium-only) → `MediaStreamAudioSourceNode` →
//! `ScriptProcessorNode`. The script node hands us contiguous blocks of mono
//! `f32` on the main thread; we forward each block to the DSP **web worker**
//! ([`crate::audio::worker`]) and read analysis snapshots back. The heavy FFT /
//! resonator math runs in the worker, off the render thread, so the UI stays
//! smooth — wasm has no real threads, so doing it inline would jank rendering.
//!
//! `ScriptProcessorNode` is deprecated but universally supported and needs no
//! AudioWorklet module or COOP/COEP headers; its callback here is light (copy
//! samples + postMessage), so its main-thread cost is negligible.

use std::cell::{
    Cell,
    RefCell,
};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::spawn_local;
use web_time::{
    Duration,
    Instant,
};

use crate::audio::types::{
    AnalysisSettings,
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
    DroneState,
    ResonatorReading,
    TunerReading,
};
use crate::audio::worker_proto::{
    FromWorker,
    ToWorker,
    decode,
    encode,
};
use crate::core_types::pitch::PNote;

const MIN_INPUT_GAIN: f32 = 0.1;
const MAX_INPUT_GAIN: f32 = 12.0;
const DEFAULT_INPUT_GAIN: f32 = 1.0;

// ScriptProcessor block size. 4096 @ ~48 kHz ≈ 85 ms — fine for a tuner, and the
// callback only copies + posts, so the size barely matters here. Power of two
// in 256..=16384.
const SCRIPT_BUFFER_SIZE: u32 = 4096;

// Synthetic input ids. Real microphones use `MIC_PREFIX` + the MediaDevices
// deviceId; the system-audio entry is a single best-effort getDisplayMedia.
const MIC_PREFIX: &str = "mic::";
const SYSTEM_ID: &str = "system";

// The resonator gate is re-asserted at most this often — `request_resonator` is
// called every frame, but the worker only needs an occasional keep-alive.
const RESONATOR_POST_INTERVAL: Duration = Duration::from_millis(100);

// trunk's `data-loader-shim` emits `<bin>_loader.js` (un-hashed). Resolved
// against `<base data-trunk-public-url>` so it works under `--public-url`.
const WORKER_URL: &str = "./dsp_worker_loader.js";

/// Latest analysis state pushed up from the worker; the UI getters read this.
#[derive(Default)]
struct Latest {
    status:    Option<AudioStatus>,
    reading:   Option<TunerReading>,
    resonator: Option<ResonatorReading>,
    level:     f32,
    waveform:  Vec<f32>,
}

/// Live capture objects. Dropping this stops the tracks, closes the context, and
/// drops the onaudioprocess closure in one move (device switch / shutdown).
struct Capture {
    ctx:       web_sys::AudioContext,
    stream:    web_sys::MediaStream,
    _source:   web_sys::MediaStreamAudioSourceNode,
    _script:   web_sys::ScriptProcessorNode,
    _on_audio: Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let tracks = self.stream.get_tracks();
        for i in 0..tracks.length() {
            if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                track.stop();
            }
        }
        let _ = self.ctx.close();
    }
}

struct Inner {
    // `None` only if the worker failed to spawn (loader missing) — then status
    // carries the error and capture is refused.
    worker:              Option<web_sys::Worker>,
    latest:              Rc<RefCell<Latest>>,
    settings:            RefCell<AnalysisSettings>,
    input_gain:          Cell<f32>,
    available:           RefCell<Vec<AudioInputOption>>,
    selected_input_id:   RefCell<Option<String>>,
    capture:             RefCell<Option<Capture>>,
    sample_rate:         Cell<u32>,
    // Состояние дрона держим, чтобы UI на wasm работал и персистился; синтеза
    // нет (Web Audio output-граф дрона ещё не реализован) — методы инертны.
    drone:               RefCell<DroneState>,
    last_resonator_post: Cell<Instant>,
    // Kept alive for as long as the engine lives so the worker callback fires.
    _on_message:         Closure<dyn FnMut(web_sys::MessageEvent)>,
}

pub struct AudioEngine {
    inner: Rc<Inner>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        let latest = Rc::new(RefCell::new(Latest::default()));
        let worker = create_worker(&latest);

        // Worker → main: decode each snapshot into `latest`.
        let latest_for_cb = latest.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let bytes = js_sys::Uint8Array::new(&event.data()).to_vec();
            if let Some(FromWorker::Snapshot(snapshot)) = decode::<FromWorker>(&bytes) {
                let mut latest = latest_for_cb.borrow_mut();
                latest.status = Some(snapshot.status);
                latest.reading = snapshot.reading;
                latest.resonator = snapshot.resonator;
                latest.level = snapshot.level;
                latest.waveform = snapshot.waveform;
            }
        });
        if let Some(worker) = worker.as_ref() {
            worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        }

        let inner = Rc::new(Inner {
            worker,
            latest,
            settings: RefCell::new(AnalysisSettings::default()),
            input_gain: Cell::new(DEFAULT_INPUT_GAIN),
            available: RefCell::new(default_inputs()),
            selected_input_id: RefCell::new(None),
            capture: RefCell::new(None),
            sample_rate: Cell::new(0),
            drone: RefCell::new(DroneState::default()),
            last_resonator_post: Cell::new(Instant::now() - RESONATOR_POST_INTERVAL),
            _on_message: on_message,
        });
        refresh_devices(inner.clone());
        Self { inner }
    }

    pub fn status(&self) -> AudioStatus {
        self.inner.latest.borrow().status.clone().unwrap_or(AudioStatus::Idle)
    }

    pub fn reading(&self) -> Option<TunerReading> {
        self.inner.latest.borrow().reading.clone()
    }

    pub fn resonator_reading(&self) -> Option<ResonatorReading> {
        self.inner.latest.borrow().resonator.clone()
    }

    pub fn analysis_settings(&self) -> AnalysisSettings {
        self.inner.settings.borrow().clone()
    }

    pub fn set_analysis_settings(&self, settings: AnalysisSettings) {
        let sanitized = settings.sanitized();
        *self.inner.settings.borrow_mut() = sanitized.clone();
        post_to(&self.inner, &ToWorker::Settings(Box::new(sanitized)));
    }

    pub fn input_gain(&self) -> f32 {
        self.inner.input_gain.get()
    }

    pub fn set_input_gain(&self, gain: f32) {
        let gain = gain.clamp(MIN_INPUT_GAIN, MAX_INPUT_GAIN);
        self.inner.input_gain.set(gain);
        post_to(&self.inner, &ToWorker::Gain(gain));
    }

    pub fn input_gain_range(&self) -> (f32, f32) {
        (MIN_INPUT_GAIN, MAX_INPUT_GAIN)
    }

    pub fn input_level(&self) -> f32 {
        self.inner.latest.borrow().level
    }

    pub fn input_waveform(&self) -> Vec<f32> {
        self.inner.latest.borrow().waveform.clone()
    }

    // ── Monitoring / loopback have no browser equivalent → inert no-ops. ──
    pub fn monitor_enabled(&self) -> bool {
        false
    }

    pub fn set_monitor_enabled(&self, _enabled: bool) {}

    pub fn monitor_gain(&self) -> f32 {
        0.0
    }

    pub fn set_monitor_gain(&self, _gain: f32) {}

    pub fn current_input_sample_rate(&self) -> u32 {
        self.inner.sample_rate.get()
    }

    pub fn monitor_output_sample_rate(&self) -> Option<u32> {
        None
    }

    pub fn default_output_device_name(&self) -> Option<String> {
        None
    }

    pub fn available_inputs(&self) -> Vec<AudioInputOption> {
        self.inner.available.borrow().clone()
    }

    pub fn selected_input_id(&self) -> Option<String> {
        self.inner.selected_input_id.borrow().clone()
    }

    /// (Re)start capture for the chosen input. Called from the UI in response to
    /// a click, so the implied user gesture lets `getUserMedia`/`getDisplayMedia`
    /// resolve and the `AudioContext` resume without an autoplay block.
    pub fn set_selected_input_id(&self, input_id: Option<String>) {
        *self.inner.selected_input_id.borrow_mut() = input_id.clone();
        // Drop the previous capture before opening the next.
        *self.inner.capture.borrow_mut() = None;
        start_capture(self.inner.clone(), input_id);
    }

    /// No speaker output graph on wasm — test-tone playback is a no-op here.
    pub fn play_test_note(&self, _midi: PNote) {}

    // ── Drone: state round-trips for the UI/persist, but no audio on wasm yet. ──
    pub fn drone_state(&self) -> DroneState {
        self.inner.drone.borrow().clone()
    }

    pub fn set_drone_state(&self, state: DroneState) {
        *self.inner.drone.borrow_mut() = state.sanitized();
    }

    pub fn drone_playing(&self) -> bool {
        false
    }

    pub fn start_drone(&self) {}

    pub fn stop_drone(&self) {}

    pub fn request_resonator(&self) {
        // Re-assert the gate at most every RESONATOR_POST_INTERVAL; the UI calls
        // this every frame, but the worker's grace window is far longer.
        let now = Instant::now();
        if now.saturating_duration_since(self.inner.last_resonator_post.get()) >= RESONATOR_POST_INTERVAL {
            self.inner.last_resonator_post.set(now);
            post_to(&self.inner, &ToWorker::ResonatorWanted(true));
        }
    }
}

fn create_worker(latest: &Rc<RefCell<Latest>>) -> Option<web_sys::Worker> {
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    match web_sys::Worker::new_with_options(WORKER_URL, &options) {
        Ok(worker) => Some(worker),
        Err(e) => {
            latest.borrow_mut().status =
                Some(AudioStatus::Error(format!("audio worker failed to start: {}", err_text(&e))));
            None
        }
    }
}

fn post_msg(worker: &web_sys::Worker, message: &ToWorker) {
    let bytes = encode(message);
    let array = js_sys::Uint8Array::from(&bytes[..]);
    let _ = worker.post_message(array.as_ref());
}

fn post_to(inner: &Inner, message: &ToWorker) {
    if let Some(worker) = inner.worker.as_ref() {
        post_msg(worker, message);
    }
}

fn set_error(inner: &Inner, message: String) {
    inner.latest.borrow_mut().status = Some(AudioStatus::Error(message));
}

/// The list shown before any device is opened: a generic mic plus the
/// best-effort system-audio share. Real per-device entries replace this once
/// [`refresh_devices`] resolves.
fn default_inputs() -> Vec<AudioInputOption> {
    vec![
        AudioInputOption {
            id:    MIC_PREFIX.to_owned(),
            label: "Микрофон".to_owned(),
            kind:  AudioInputKind::Microphone,
        },
        AudioInputOption {
            id:    SYSTEM_ID.to_owned(),
            label: "Системный звук (поделиться)".to_owned(),
            kind:  AudioInputKind::System,
        },
    ]
}

/// Enumerate audio inputs and cache them on `inner`. Device labels are only
/// populated by the browser after a capture permission has been granted, so this
/// is called both at startup (generic names) and after each successful capture.
fn refresh_devices(inner: Rc<Inner>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(media_devices) = window.navigator().media_devices() else {
        return;
    };
    let Ok(promise) = media_devices.enumerate_devices() else {
        return;
    };
    spawn_local(async move {
        let Ok(result) = JsFuture::from(promise).await else {
            return;
        };
        let array = js_sys::Array::from(&result);
        let mut options = Vec::new();
        for value in array.iter() {
            let Ok(info) = value.dyn_into::<web_sys::MediaDeviceInfo>() else {
                continue;
            };
            if info.kind() != web_sys::MediaDeviceKind::Audioinput {
                continue;
            }
            let device_id = info.device_id();
            let label = {
                let raw = info.label();
                if raw.is_empty() { "Микрофон".to_owned() } else { raw }
            };
            options.push(AudioInputOption {
                id: format!("{MIC_PREFIX}{device_id}"),
                label,
                kind: AudioInputKind::Microphone,
            });
        }
        // Always offer system-audio share last (browser-gated, best-effort).
        options.push(AudioInputOption {
            id:    SYSTEM_ID.to_owned(),
            label: "Системный звук (поделиться)".to_owned(),
            kind:  AudioInputKind::System,
        });
        if options.is_empty() {
            options = default_inputs();
        }
        *inner.available.borrow_mut() = options;
    });
}

/// Kick off an async capture; report failures into shared state so the UI shows
/// them (denied permission, no device, unsupported getDisplayMedia, …).
fn start_capture(inner: Rc<Inner>, id: Option<String>) {
    let Some(worker) = inner.worker.clone() else {
        set_error(&inner, "audio worker unavailable".to_owned());
        return;
    };
    spawn_local(async move {
        if let Err(message) = run_capture(inner.clone(), worker, id).await {
            set_error(&inner, message);
        }
    });
}

async fn run_capture(inner: Rc<Inner>, worker: web_sys::Worker, id: Option<String>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let media_devices = window
        .navigator()
        .media_devices()
        .map_err(|e| format!("mediaDevices unavailable: {}", err_text(&e)))?;

    let is_system = id.as_deref() == Some(SYSTEM_ID);
    let promise = if is_system {
        // getDisplayMedia requires a video track even when we only want audio;
        // we ignore the video track and keep just the audio.
        let constraints = web_sys::DisplayMediaStreamConstraints::new();
        constraints.set_audio_bool(true);
        constraints.set_video_bool(true);
        media_devices
            .get_display_media_with_constraints(&constraints)
            .map_err(|e| format!("getDisplayMedia failed: {}", err_text(&e)))?
    } else {
        let constraints = web_sys::MediaStreamConstraints::new();
        match mic_device_id(&id) {
            Some(device_id) if !device_id.is_empty() => {
                // { deviceId: { exact: <id> } }
                let exact = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&exact, &"exact".into(), &device_id.into());
                let audio = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&audio, &"deviceId".into(), &exact);
                constraints.set_audio(&audio);
            }
            _ => constraints.set_audio_bool(true),
        }
        media_devices
            .get_user_media_with_constraints(&constraints)
            .map_err(|e| format!("getUserMedia failed: {}", err_text(&e)))?
    };

    let stream: web_sys::MediaStream = JsFuture::from(promise)
        .await
        .map_err(|e| format!("capture denied / failed: {}", err_text(&e)))?
        .dyn_into()
        .map_err(|_| "capture returned a non-MediaStream".to_owned())?;

    let ctx = web_sys::AudioContext::new().map_err(|e| format!("AudioContext failed: {}", err_text(&e)))?;
    let _ = ctx.resume(); // best-effort: leave any rejection to the no-audio symptom
    let sample_rate = ctx.sample_rate();

    let source = ctx
        .create_media_stream_source(&stream)
        .map_err(|e| format!("createMediaStreamSource failed: {}", err_text(&e)))?;
    let script = ctx
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
            SCRIPT_BUFFER_SIZE,
            1,
            1,
        )
        .map_err(|e| format!("createScriptProcessor failed: {}", err_text(&e)))?;

    // Audio callback: forward each block to the worker; the DSP runs there.
    let worker_for_cb = worker.clone();
    let on_audio = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new(
        move |event: web_sys::AudioProcessingEvent| {
            let Ok(input_buffer) = event.input_buffer() else {
                return;
            };
            let Ok(samples) = input_buffer.get_channel_data(0) else {
                return;
            };
            post_msg(&worker_for_cb, &ToWorker::Samples(samples));

            // Silence the node's output so the mic isn't routed to the speakers
            // (the node still needs a destination connection to keep firing).
            if let Ok(output) = event.output_buffer() {
                let silence = vec![0.0f32; output.length() as usize];
                let _ = output.copy_to_channel(&silence, 0);
            }
        },
    );

    script.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
    source
        .connect_with_audio_node(&script)
        .map_err(|e| format!("connect(source→script) failed: {}", err_text(&e)))?;
    script
        .connect_with_audio_node(&ctx.destination())
        .map_err(|e| format!("connect(script→dest) failed: {}", err_text(&e)))?;

    // Spin the worker pipelines up for this rate and hand it current settings.
    post_msg(&worker, &ToWorker::Init { sample_rate });
    post_msg(&worker, &ToWorker::Settings(Box::new(inner.settings.borrow().clone())));
    post_msg(&worker, &ToWorker::Gain(inner.input_gain.get()));

    inner.sample_rate.set(sample_rate as u32);
    inner.latest.borrow_mut().status = Some(AudioStatus::Listening);
    *inner.capture.borrow_mut() = Some(Capture {
        ctx,
        stream,
        _source: source,
        _script: script,
        _on_audio: on_audio,
    });

    // Labels are available now that permission was granted — refresh the list.
    refresh_devices(inner.clone());
    Ok(())
}

/// Strip the `mic::` prefix to recover the MediaDevices deviceId, if this id
/// names a specific microphone (vs the default / system entry).
fn mic_device_id(id: &Option<String>) -> Option<String> {
    id.as_deref().and_then(|s| s.strip_prefix(MIC_PREFIX)).map(str::to_owned)
}

fn err_text(value: &JsValue) -> String {
    value
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .unwrap_or_else(|| format!("{value:?}"))
}
