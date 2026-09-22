use std::path::PathBuf;
use std::sync::atomic::{
    AtomicBool,
    AtomicU32,
    Ordering,
};
use std::sync::mpsc::{
    self,
    Receiver,
    Sender,
};
use std::sync::{
    Arc,
    Mutex,
    TryLockError,
};
use std::thread::{
    self,
    JoinHandle,
};
use std::time::{
    Duration,
    Instant,
};

use cpal::traits::{
    DeviceTrait,
    HostTrait,
    StreamTrait,
};
use ringbuf::HeapRb;
use ringbuf::traits::Split;

use super::super::types::{
    AnalysisSettings,
    AudioInputOption,
    AudioStatus,
    DroneState,
    MelodyFrame,
    RecorderStatus,
    ReplayStatus,
    ResonatorReading,
    TakeOnDisk,
    TunerReading,
};
use crate::core_types::pitch::PNote;

mod capture;
mod devices;
mod drone;
mod output;
mod recorder;
mod replay;
mod workers;

/// Диагностика аудио-пути на Android.
///
/// Зачем отдельный канал: на устройстве `eprintln!` уходит в никуда (stdout/stderr
/// приложения не попадают в logcat без `log.redirect-stdio`), а крейт `log` тут
/// молчит — какая-то зависимость вырезает его через `log/max_level_*`. Поэтому
/// единственный способ узнать, почему захват не поднялся, — liblog напрямую, тем же
/// тегом `snail`, что и пермишен-драйвер: `adb logcat -s snail`.
///
/// На остальных платформах — no-op: там ошибка и так видна в UI (`AudioStatus::Error`)
/// и в терминале.
#[cfg(target_os = "android")]
fn audio_alog(msg: &str) {
    crate::android_perm::alog(msg);
}
#[cfg(not(target_os = "android"))]
fn audio_alog(_msg: &str) {
}

/// Единственный канал для ошибок, приходящих ИЗ колбэка драйвера.
///
/// Такую ошибку некуда вернуть (`Result` у колбэка нет), а на устройстве
/// `eprintln!` уходит в никуда — см. [`audio_alog`]. При этом отвал устройства
/// на телефоне самый частый случай: свернули приложение → AAudio закрыл вход.
/// На десктопе поведение прежнее: `audio_alog` там no-op, остаётся stderr.
fn report_stream_error(what: &str, err: &cpal::Error) {
    audio_alog(&format!("{what} error: {err}"));
    eprintln!("{what} error: {err}");
}

use capture::{
    ActiveCapture,
    ActiveInput,
    InputFanout,
    build_input,
    build_pulse_input,
};
use devices::{
    cpal_device_display_name,
    enumerate_input_options,
    low_latency_monitor_ring_len,
    preferred_low_latency_buffer,
    route_id_for_this_platform,
    select_cpal_capture,
};
use drone::DroneSynth;
use output::{
    build_monitor_output,
    play_test_note_thread,
};
use recorder::{
    RecorderHandle,
    recorder_ring,
    start_recorder_worker,
};
use replay::{
    ReplayHandle,
    list_takes,
    load_take,
    start_replay_source,
};
use workers::{
    AnalysisWorker,
    WorkerPipeline,
    analysis_ring,
    start_worker,
};

// The analysis (FFT/YIN/resonator) and the pipelines that drive it live in
// the target-agnostic `audio::core`/`audio::dsp` so the wasm engine reuses
// exactly the same code. Native owns only capture + threading below.
use crate::audio::core::{
    AnalysisPipeline,
    ResonatorPipeline,
    SharedState,
    set_shared_error,
};
use crate::audio::dsp::analysis_math::{
    NOTE_BUCKET_MAX_MIDI,
    NOTE_BUCKET_MIN_MIDI,
};
use crate::audio::sample_rate::SampleRate;

const CPAL_INPUT_ID_PREFIX: &str = "cpal::";
#[cfg(target_os = "windows")]
const CPAL_DEFAULT_OUTPUT_LOOPBACK_ID: &str = "cpal-loopback::@DEFAULT_OUTPUT@";
const PULSE_INPUT_ID_PREFIX: &str = "pulse::";
const PULSE_DEFAULT_SOURCE_ID: &str = "pulse::@DEFAULT_SOURCE@";
const PULSE_DEFAULT_MONITOR_ID: &str = "pulse::@DEFAULT_MONITOR@";
const PULSE_CAPTURE_RATE: SampleRate = SampleRate(48_000);
const LOW_LATENCY_TARGET_FRAMES: u32 = 256;

// Analysis tuning constants (window/waterfall/interval) live in `audio::core`.

// Gain
const DEFAULT_INPUT_GAIN: f32 = 1.0;
const MIN_INPUT_GAIN: f32 = 0.1;
const MAX_INPUT_GAIN: f32 = 12.0;
const MONITOR_DEFAULT_GAIN: f32 = 0.35;

// Время паузы воркера, когда в кольце нет свежих сэмплов
const ANALYSIS_IDLE_SLEEP: Duration = Duration::from_millis(5);
/// Сколько резонаторный банк ещё молотит после последнего запроса от UI.
/// UI двигает дедлайн каждый кадр, пока панель-потребитель видна; когда панель
/// закрылась и запросы прекратились, через этот грейс воркер паркуется.
const RESONATOR_PARK_GRACE: Duration = Duration::from_millis(300);

type SampleProducer = <HeapRb<f32> as Split>::Prod;
type SampleConsumer = <HeapRb<f32> as Split>::Cons;

// `SharedState` (the UI-facing snapshot) is defined in `audio::core`.

// ------------------------------------------------------------------
// AudioEngine: тонкий фасад для UI. Всё живое в отдельном audio-треде,
// UI общается с ним через mpsc-канал и набор атомиков/мутексов.
// ------------------------------------------------------------------
pub struct AudioEngine {
    shared:              Arc<Mutex<SharedState>>,
    settings:            Arc<Mutex<AnalysisSettings>>,
    input_gain:          Arc<AtomicU32>,
    input_level:         Arc<AtomicU32>,
    monitor_enabled:     Arc<AtomicBool>,
    monitor_gain:        Arc<AtomicU32>,
    input_sample_rate:   Arc<AtomicU32>,
    monitor_output_rate: Arc<AtomicU32>, // 0 = output не запущен
    selected_input_id:   Arc<Mutex<Option<String>>>,
    resonator_wanted:    Arc<Mutex<Instant>>, // дедлайн «банк нужен до» (гейт)
    // Живое состояние дрона: UI пишет его целиком (lock+write), реалтайм-
    // колбэк дрон-стрима читает try_lock каждый блок. Источник истины для
    // персиста, поэтому копии на App нет.
    drone:               Arc<Mutex<DroneState>>,
    // true, пока дрон-стрим поднят. Ставит/снимает audio-тред.
    drone_playing:       Arc<AtomicBool>,
    // Состояние записи дублей. Живёт НАД capture: смена устройства
    // пересоздаёт весь capture, и писатель нового подхватывает то же
    // намерение вместо того, чтобы потерять его.
    recorder:            RecorderHandle,
    /// Состояние реплея. Живёт НАД capture по той же причине, что и рекордер:
    /// статус должен пережить capture, который его произвёл — UI спрашивает
    /// «что там с дублем, который я проиграл» уже после того, как источник
    /// отработал и встал.
    replay:              ReplayHandle,
    command_tx:          Option<Sender<Command>>,
    audio_thread:        Option<JoinHandle<()>>,
}

enum Command {
    SwitchInput(Option<String>),
    SetMonitorEnabled(bool),
    PlayTestNote(PNote),
    StartDrone,
    StopDrone,
    /// Проиграть дубль через движок вместо живого входа.
    StartReplay(PathBuf),
    /// Вернуть живой вход. Отдельная команда, а не `SwitchInput`: вернуться
    /// надо на ТО ЖЕ устройство, которое было выбрано до реплея, и знает его
    /// audio-тред (capture'а), а не UI.
    StopReplay,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(SharedState::new()));
        let settings = Arc::new(Mutex::new(AnalysisSettings::default()));
        let input_gain = Arc::new(AtomicU32::new(DEFAULT_INPUT_GAIN.to_bits()));
        let input_level = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let monitor_enabled = Arc::new(AtomicBool::new(false));
        let monitor_gain = Arc::new(AtomicU32::new(MONITOR_DEFAULT_GAIN.to_bits()));
        let input_sample_rate = Arc::new(AtomicU32::new(0));
        let monitor_output_rate = Arc::new(AtomicU32::new(0));
        let selected_input_id = Arc::new(Mutex::new(None));
        // Гейт резонатора: дедлайн в прошлом → пока никто не просит, банк не молотит.
        let resonator_wanted = Arc::new(Mutex::new(Instant::now()));
        let drone = Arc::new(Mutex::new(DroneState::default()));
        let drone_playing = Arc::new(AtomicBool::new(false));
        let recorder = RecorderHandle::new();
        let replay = ReplayHandle::new();

        let (command_tx, command_rx) = mpsc::channel::<Command>();

        // Запускаем audio-тред: он единственный владеет cpal::Stream.
        // UI шлёт команды через канал и мгновенно возвращается.
        let audio_thread = thread::spawn({
            let ctx = AudioContext {
                shared:              shared.clone(),
                settings:            settings.clone(),
                input_gain:          input_gain.clone(),
                input_level:         input_level.clone(),
                monitor_enabled:     monitor_enabled.clone(),
                monitor_gain:        monitor_gain.clone(),
                input_sample_rate:   input_sample_rate.clone(),
                monitor_output_rate: monitor_output_rate.clone(),
                selected_input_id:   selected_input_id.clone(),
                resonator_wanted:    resonator_wanted.clone(),
                drone:               drone.clone(),
                drone_playing:       drone_playing.clone(),
                recorder:            recorder.clone(),
                replay:              replay.clone(),
            };
            move || {
                audio_thread_main(command_rx, ctx);
            }
        });

        Self {
            shared,
            settings,
            input_gain,
            input_level,
            monitor_enabled,
            monitor_gain,
            input_sample_rate,
            monitor_output_rate,
            selected_input_id,
            resonator_wanted,
            drone,
            drone_playing,
            recorder,
            replay,
            command_tx: Some(command_tx),
            audio_thread: Some(audio_thread),
        }
    }

    pub fn status(&self) -> AudioStatus {
        self.shared.lock().unwrap().status.clone()
    }

    pub fn reading(&self) -> Option<TunerReading> {
        self.shared.lock().unwrap().reading.clone()
    }

    /// The melody line's recent history: every bank frame newer than `after`,
    /// oldest → newest. `None` for a cold start.
    ///
    /// `after` is the **caller's own** cursor (the `seq` of the last frame it took),
    /// not a mark the engine keeps — so any number of panels can each read the whole
    /// history without stealing frames from one another. See [`MelodyFrame`].
    ///
    /// This is how a panel draws the melody without decimating it: `reading()` is
    /// the instant, and sampling *that* per UI frame drops bank frames on the floor
    /// (half of them at 30 fps). Like every consumer of the bank, the caller must be
    /// calling [`Self::request_resonator`] or the history simply stops growing.
    pub fn melody_since(&self, after: Option<u64>) -> Vec<MelodyFrame> {
        self.shared.lock().unwrap().melody_since(after)
    }

    pub fn resonator_reading(&self) -> Option<ResonatorReading> {
        let g = self.shared.lock().unwrap();
        (!g.resonator_spectrum.is_empty()).then(|| {
            ResonatorReading {
                spectrum:    g.resonator_spectrum.clone(),
                waterfall:   g.resonator_waterfall.iter().cloned().collect(),
                note_labels: g.resonator_labels.clone(),
            }
        })
    }

    pub fn analysis_settings(&self) -> AnalysisSettings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_analysis_settings(&self, settings: AnalysisSettings) {
        *self.settings.lock().unwrap() = settings.sanitized();
    }

    pub fn input_gain(&self) -> f32 {
        f32::from_bits(self.input_gain.load(Ordering::Relaxed))
    }

    pub fn set_input_gain(&self, gain: f32) {
        self.input_gain.store(
            gain.clamp(MIN_INPUT_GAIN, MAX_INPUT_GAIN).to_bits(),
            Ordering::Relaxed,
        );
    }

    pub fn input_gain_range(&self) -> (f32, f32) {
        (MIN_INPUT_GAIN, MAX_INPUT_GAIN)
    }

    pub fn input_level(&self) -> f32 {
        f32::from_bits(self.input_level.load(Ordering::Relaxed))
    }

    pub fn input_waveform(&self) -> Vec<f32> {
        self.shared
            .lock()
            .unwrap()
            .input_waveform
            .iter()
            .copied()
            .collect()
    }

    pub fn monitor_enabled(&self) -> bool {
        self.monitor_enabled.load(Ordering::Relaxed)
    }

    pub fn set_monitor_enabled(&self, enabled: bool) {
        if let Some(tx) = self.command_tx.as_ref() {
            let _ = tx.send(Command::SetMonitorEnabled(enabled));
        }
    }

    pub fn monitor_gain(&self) -> f32 {
        f32::from_bits(self.monitor_gain.load(Ordering::Relaxed))
    }

    pub fn set_monitor_gain(&self, gain: f32) {
        self.monitor_gain
            .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn current_input_sample_rate(&self) -> u32 {
        self.input_sample_rate.load(Ordering::Relaxed)
    }

    pub fn monitor_output_sample_rate(&self) -> Option<u32> {
        let rate = self.monitor_output_rate.load(Ordering::Relaxed);
        if rate == 0 { None } else { Some(rate) }
    }

    pub fn default_output_device_name(&self) -> Option<String> {
        cpal::default_host()
            .default_output_device()
            .map(|d| cpal_device_display_name(&d))
    }

    pub fn available_inputs(&self) -> Vec<AudioInputOption> {
        enumerate_input_options()
    }

    pub fn selected_input_id(&self) -> Option<String> {
        self.selected_input_id.lock().unwrap().clone()
    }

    pub fn set_selected_input_id(&self, input_id: Option<String>) {
        if let Some(tx) = self.command_tx.as_ref() {
            let _ = tx.send(Command::SwitchInput(input_id));
        }
    }

    pub fn play_test_note(&self, midi: PNote) {
        if let Some(tx) = self.command_tx.as_ref() {
            let _ = tx.send(Command::PlayTestNote(midi));
        }
    }

    /// Снимок текущего состояния дрона (для отрисовки/правки в UI).
    pub fn drone_state(&self) -> DroneState {
        self.drone.lock().unwrap().clone()
    }

    /// Заменить состояние дрона целиком. Реалтайм-колбэк подхватит его на
    /// ближайшем блоке — менять ноты/темп/режим можно прямо во время игры,
    /// перезапуск стрима не нужен.
    pub fn set_drone_state(&self, state: DroneState) {
        *self.drone.lock().unwrap() = state.sanitized();
    }

    pub fn drone_playing(&self) -> bool {
        self.drone_playing.load(Ordering::Relaxed)
    }

    pub fn start_drone(&self) {
        if let Some(tx) = self.command_tx.as_ref() {
            let _ = tx.send(Command::StartDrone);
        }
    }

    pub fn stop_drone(&self) {
        if let Some(tx) = self.command_tx.as_ref() {
            let _ = tx.send(Command::StopDrone);
        }
    }

    /// Запросить резонаторный банк на ближайший грейс. Панели-потребители
    /// (Scale Finder, Resonator *) зовут это каждый кадр, пока видимы; пока
    /// зовут — воркер молотит, перестали (панель закрылась) — паркуется.
    pub fn request_resonator(&self) {
        *self.resonator_wanted.lock().unwrap() = Instant::now() + RESONATOR_PARK_GRACE;
    }

    /// Начать писать дубль в `path`. Идемпотентно по пути: повторный вызов с
    /// тем же путём ничего не делает, с другим — закрывает текущий и открывает
    /// новый. См. `recorder` — дубль пишется ДО `input_gain`.
    pub fn start_take(&self, path: PathBuf) {
        self.recorder.start(path);
    }

    pub fn stop_take(&self) {
        self.recorder.stop();
    }

    pub fn recorder_status(&self) -> RecorderStatus {
        self.recorder.status()
    }

    /// Проиграть записанный дубль через движок вместо живого входа.
    ///
    /// Линия, которая при этом рисуется, — БОЕВАЯ: её строит тот же код, что и
    /// вживую, из тех же сэмплов, в реальном времени (см. `replay`). Живой вход
    /// на время реплея уходит и возвращается по [`Self::stop_replay`].
    pub fn start_replay(&self, path: PathBuf) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(Command::StartReplay(path));
        }
    }

    /// Вернуть живой вход.
    ///
    /// Реплей НЕ делает этого сам, дойдя до конца дубля: линия, которую он
    /// только что нарисовал, — это то, что юзер собрался размечать, а живой
    /// микрофон писал бы поверх неё свои кадры.
    pub fn stop_replay(&self) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(Command::StopReplay);
        }
    }

    pub fn replay_status(&self) -> ReplayStatus {
        self.replay.status()
    }

    /// Какие дубли лежат в `dir` и, значит, могут быть проиграны.
    ///
    /// Читает только заголовки WAV'ов — приговора «улика/не улика» тут НЕТ и
    /// быть не может: он про запись, а не про файл (см. [`TakeOnDisk`]).
    pub fn list_takes(&self, dir: &std::path::Path) -> Vec<TakeOnDisk> {
        list_takes(dir)
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // Роняем sender → audio-тред получает Disconnected, чисто выходит.
        drop(self.command_tx.take());
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }
    }
}

// ------------------------------------------------------------------
// Audio-тред: единственный владелец cpal::Stream.
// ------------------------------------------------------------------
fn audio_thread_main(rx: Receiver<Command>, ctx: AudioContext) {
    // Стартовый capture: берём дефолтный input.
    // Если не поднялся — оставляем в состоянии Error, UI покажет.
    let mut current = ctx.build_capture(None).ok();
    if current.is_none() {
        ctx.set_error("Could not open default audio input");
    }

    // Дрон-стрим живёт параллельно capture: отдельный output, который audio-
    // тред держит ровно пока дрон играет. Дроп = тишина и освобождение устройства.
    let mut drone_stream: Option<cpal::Stream> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::SwitchInput(id) => {
                if let Some(cap) = current.take() {
                    cap.shutdown();
                }
                match ctx.build_capture(id.clone()) {
                    Ok(cap) => current = Some(cap),
                    Err(msg) => ctx.set_error(&msg),
                }
            }
            Command::SetMonitorEnabled(on) => {
                ctx.monitor_enabled.store(on, Ordering::Relaxed);
                // Монитор запускается/останавливается пересозданием capture,
                // так мы без гонок привязываем output-stream к ring-буферу,
                // который входной callback наполняет.
                if let Some(cap) = current.take() {
                    let id = Some(cap.selected_id.clone());
                    cap.shutdown();
                    match ctx.build_capture(id) {
                        Ok(cap) => current = Some(cap),
                        Err(msg) => ctx.set_error(&msg),
                    }
                }
            }
            Command::PlayTestNote(midi) => {
                ctx.play_test_note(midi);
            }
            Command::StartDrone => {
                // Идемпотентно: если уже играет, оставляем текущий стрим.
                if drone_stream.is_none() {
                    match ctx.build_drone_stream() {
                        Ok(stream) => {
                            drone_stream = Some(stream);
                            ctx.drone_playing.store(true, Ordering::Relaxed);
                        }
                        Err(msg) => ctx.set_error(&msg),
                    }
                }
            }
            Command::StopDrone => {
                drone_stream = None; // дроп стрима = тишина
                ctx.drone_playing.store(false, Ordering::Relaxed);
            }
            Command::StartReplay(path) => {
                // Живой вход уходит целиком: два источника в одни кольца — это
                // микрофон, подмешанный в дубль, то есть линия про сигнал,
                // которого не существовало.
                if let Some(cap) = current.take() {
                    cap.shutdown();
                }
                match ctx.build_replay_capture(path) {
                    Ok(cap) => current = Some(cap),
                    Err(msg) => {
                        // Дубль не открылся. Сказать об этом надо ИМЕННО в
                        // статусе реплея (UI ждёт приговор там), а живой вход
                        // вернуть — иначе неудачный клик оставил бы приложение
                        // вообще без входа.
                        ctx.replay.publish(ReplayStatus::Failed(msg));
                        match ctx.build_capture(ctx.selected_input_id.lock().unwrap().clone()) {
                            Ok(cap) => current = Some(cap),
                            Err(msg) => ctx.set_error(&msg),
                        }
                    }
                }
            }
            Command::StopReplay => {
                if let Some(cap) = current.take() {
                    cap.shutdown();
                }
                ctx.replay.publish(ReplayStatus::Idle);
                // Обратно на то устройство, что было выбрано до реплея:
                // `build_replay_capture` намеренно не трогал этот выбор.
                match ctx.build_capture(ctx.selected_input_id.lock().unwrap().clone()) {
                    Ok(cap) => current = Some(cap),
                    Err(msg) => ctx.set_error(&msg),
                }
            }
        }
    }

    drop(drone_stream);
    if let Some(cap) = current.take() {
        cap.shutdown();
    }
}

// Всё, что нужно audio-треду (клоны атомиков/мутексов).
struct AudioContext {
    shared:              Arc<Mutex<SharedState>>,
    settings:            Arc<Mutex<AnalysisSettings>>,
    input_gain:          Arc<AtomicU32>,
    input_level:         Arc<AtomicU32>,
    monitor_enabled:     Arc<AtomicBool>,
    monitor_gain:        Arc<AtomicU32>,
    input_sample_rate:   Arc<AtomicU32>,
    monitor_output_rate: Arc<AtomicU32>,
    selected_input_id:   Arc<Mutex<Option<String>>>,
    resonator_wanted:    Arc<Mutex<Instant>>,
    drone:               Arc<Mutex<DroneState>>,
    drone_playing:       Arc<AtomicBool>,
    recorder:            RecorderHandle,
    replay:              ReplayHandle,
}

impl AudioContext {
    fn set_error(&self, msg: &str) {
        // Единственная воронка всех отказов аудио-треда ⇒ единственное место, где
        // нужен лог: на Android причина иначе не наблюдаема вообще (см. `audio_alog`).
        audio_alog(&format!("audio error: {msg}"));
        self.shared.lock().unwrap().status = AudioStatus::Error(msg.to_owned());
    }

    fn reset_shared_for_test_tone(&self) {
        self.reset_shared_state();
        self.input_level.store(0.0f32.to_bits(), Ordering::Relaxed);
    }

    fn reset_shared_state(&self) {
        self.shared.lock().unwrap().reset();
    }

    fn play_test_note(&self, midi: PNote) {
        // Restrict to the audible test-tone bucket range; both bounds are in
        // 0..=127, so rebuilding the validated `PNote` can't fail.
        let clamped = (midi.as_u8() as usize).clamp(NOTE_BUCKET_MIN_MIDI, NOTE_BUCKET_MAX_MIDI);
        let midi = PNote::new(clamped as u8).unwrap();
        self.reset_shared_for_test_tone();
        let shared = self.shared.clone();
        let settings = self.settings.clone();
        let input_level = self.input_level.clone();

        thread::spawn(move || {
            if let Err(message) = play_test_note_thread(midi, shared.clone(), settings, input_level) {
                set_shared_error(&shared, &message);
            }
        });
    }

    /// Поднять непрерывный дрон-output. Колбэк синтезирует в реалтайме из
    /// [`DroneState`] (try_lock каждый блок), голоса держат фазу между
    /// блоками и сглаживаются по амплитуде → щелчков нет даже на пульсе/арпе.
    fn build_drone_stream(&self) -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device for drone".to_owned())?;
        let mut supported = device
            .supported_output_configs()
            .map_err(|e| format!("Drone output configs error: {e}"))?;
        let output_config = supported
            .find(|config| config.sample_format() == cpal::SampleFormat::F32)
            .ok_or_else(|| "No f32 output config for drone".to_owned())?;
        let output_rate = 48_000_u32.clamp(output_config.min_sample_rate(), output_config.max_sample_rate());
        let mut config = output_config.with_sample_rate(output_rate).config();
        config.buffer_size = preferred_low_latency_buffer(output_config.buffer_size());
        let sample_rate = SampleRate(config.sample_rate);
        let channels = usize::from(config.channels);

        let drone = self.drone.clone();
        let mut synth = DroneSynth::new(sample_rate);

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _| {
                    // try_lock: если UI прямо сейчас пишет состояние, держим
                    // прошлый снимок — реалтайм-колбэк никогда не блокируется.
                    // Отравленный замок — не «занято»: писавший поток уже упал.
                    match drone.try_lock() {
                        Ok(guard) => synth.adopt(&guard),
                        Err(TryLockError::WouldBlock) => {}
                        Err(TryLockError::Poisoned(e)) => panic!("{e}"),
                    }
                    for frame in data.chunks_mut(channels) {
                        let sample = synth.next_sample();
                        for out in frame {
                            *out = sample;
                        }
                    }
                },
                |err| report_stream_error("Drone output", &err),
                None,
            )
            .map_err(|e| format!("Failed to build drone output: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start drone output: {e}"))?;
        Ok(stream)
    }

    // Поднимает входной stream, кольцевые буферы, анализ-воркер и
    // (опционально) монитор-выход. Возвращает собранный ActiveCapture.
    fn build_capture(&self, id: Option<String>) -> Result<ActiveCapture, String> {
        // Единственная точка развилки pulse/cpal ⇒ здесь же отсекаем маршрут,
        // которого на этой платформе быть не может (персистнутый pulse-id на
        // Android). Подробности — в `route_id_for_this_platform`.
        let id = route_id_for_this_platform(id);

        if let Some(requested) = id.as_deref()
            && requested.starts_with(PULSE_INPUT_ID_PREFIX)
        {
            return self.build_pulse_capture(requested);
        }

        self.build_cpal_capture(id)
    }

    fn build_cpal_capture(&self, id: Option<String>) -> Result<ActiveCapture, String> {
        let host = cpal::default_host();
        let capture = select_cpal_capture(&host, id.as_deref())?;
        let device = capture.device;
        let selected_id = capture.selected_id;
        let config = capture.config;
        let sample_rate = SampleRate(config.sample_rate());
        let channels = usize::from(config.channels());
        let sample_format = config.sample_format();
        let input_buffer_size = preferred_low_latency_buffer(config.buffer_size());
        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.buffer_size = input_buffer_size;

        let (analysis_prod, analysis_cons) = analysis_ring(sample_rate);
        let (resonator_prod, resonator_cons) = analysis_ring(sample_rate);
        let (monitor_prod, monitor_cons) = self.monitor_ring(sample_rate);
        let (recorder_prod, recorder_cons) = recorder_ring(sample_rate);
        let fanout = InputFanout {
            analysis:  analysis_prod,
            resonator: resonator_prod,
            monitor:   monitor_prod,
            recorder:  Some(self.recorder.tap(recorder_prod)),
        };

        // Входной stream: callback тупо пушит в кольца, без блокировок и паник.
        let input_stream = match sample_format {
            cpal::SampleFormat::F32 => build_input::<f32>(&device, &stream_config, channels, fanout)?,
            cpal::SampleFormat::I16 => build_input::<i16>(&device, &stream_config, channels, fanout)?,
            cpal::SampleFormat::U16 => build_input::<u16>(&device, &stream_config, channels, fanout)?,
            other => return Err(format!("Unsupported sample format: {other:?}")),
        };
        input_stream
            .play()
            .map_err(|e| format!("Failed to start input stream: {e}"))?;

        let (output_stream, output_rate) = self.start_monitor_output(sample_rate, monitor_cons);
        let analysis = self.start_analysis_worker(sample_rate, analysis_cons);
        let resonator = self.start_resonator_worker(sample_rate, resonator_cons);
        let recorder = Some(start_recorder_worker(
            sample_rate,
            recorder_cons,
            self.recorder.clone(),
        ));
        self.finish_capture_start(sample_rate, output_rate, &selected_id);

        Ok(ActiveCapture {
            input: ActiveInput::Cpal(input_stream),
            output_stream,
            analysis,
            resonator,
            recorder,
            selected_id,
        })
    }

    fn build_pulse_capture(&self, id: &str) -> Result<ActiveCapture, String> {
        let sample_rate = PULSE_CAPTURE_RATE;
        let selected_id = id.to_owned();

        let (analysis_prod, analysis_cons) = analysis_ring(sample_rate);
        let (resonator_prod, resonator_cons) = analysis_ring(sample_rate);
        let (monitor_prod, monitor_cons) = self.monitor_ring(sample_rate);
        let (recorder_prod, recorder_cons) = recorder_ring(sample_rate);
        let fanout = InputFanout {
            analysis:  analysis_prod,
            resonator: resonator_prod,
            monitor:   monitor_prod,
            recorder:  Some(self.recorder.tap(recorder_prod)),
        };

        let input = ActiveInput::Pulse(build_pulse_input(id, sample_rate, fanout, self.shared.clone())?);

        let (output_stream, output_rate) = self.start_monitor_output(sample_rate, monitor_cons);
        let analysis = self.start_analysis_worker(sample_rate, analysis_cons);
        let resonator = self.start_resonator_worker(sample_rate, resonator_cons);
        let recorder = Some(start_recorder_worker(
            sample_rate,
            recorder_cons,
            self.recorder.clone(),
        ));
        self.finish_capture_start(sample_rate, output_rate, &selected_id);

        Ok(ActiveCapture {
            input,
            output_stream,
            analysis,
            resonator,
            recorder,
            selected_id,
        })
    }

    // Кольцевой буфер для монитора-вывода. Создаём только если monitor on.
    // Держим запас небольшим, чтобы монитор не копил лишнюю задержку.
    fn monitor_ring(&self, sample_rate: SampleRate) -> (Option<SampleProducer>, Option<SampleConsumer>) {
        if self.monitor_enabled.load(Ordering::Relaxed) {
            let (prod, cons) = HeapRb::<f32>::new(low_latency_monitor_ring_len(sample_rate)).split();
            (Some(prod), Some(cons))
        } else {
            (None, None)
        }
    }

    // Ошибку запуска монитора не считаем фатальной: запись и анализ
    // должны продолжать работать без playback monitoring.
    //
    // `None` = монитор не поднят (выключен или не собрался), и частоты у него нет.
    fn start_monitor_output(
        &self,
        sample_rate: SampleRate,
        monitor_cons: Option<SampleConsumer>,
    ) -> (Option<cpal::Stream>, Option<SampleRate>) {
        monitor_cons
            .and_then(|cons| build_monitor_output(sample_rate, cons, self.monitor_gain.clone()).ok())
            .unzip()
    }

    fn start_analysis_worker(
        &self,
        sample_rate: SampleRate,
        analysis_cons: SampleConsumer,
    ) -> AnalysisWorker {
        start_worker(
            analysis_cons,
            WorkerPipeline::Analysis(AnalysisPipeline::new(sample_rate)),
            self.shared.clone(),
            self.settings.clone(),
            self.input_gain.clone(),
            self.input_level.clone(),
        )
    }

    fn start_resonator_worker(
        &self,
        sample_rate: SampleRate,
        resonator_cons: SampleConsumer,
    ) -> AnalysisWorker {
        start_worker(
            resonator_cons,
            WorkerPipeline::Resonator {
                pipeline: ResonatorPipeline::new(sample_rate),
                wanted:   self.resonator_wanted.clone(),
            },
            self.shared.clone(),
            self.settings.clone(),
            self.input_gain.clone(),
            self.input_level.clone(),
        )
    }

    /// Общий хвост всех трёх путей захвата ⇒ единственная точка, где виден факт
    /// «вход поднялся, и вот на чём». Без этой строки в логе «звука нет» неотличимо
    /// от «звук идёт, но тишина в микрофоне» — а это разные баги.
    fn finish_capture_start(
        &self,
        sample_rate: SampleRate,
        output_rate: Option<SampleRate>,
        selected_id: &str,
    ) {
        audio_alog(&format!(
            "capture started: id={selected_id} rate={} monitor_out={}",
            sample_rate.0,
            output_rate.map_or(0, |rate| rate.0)
        ));
        self.store_capture_rates(sample_rate, output_rate);
        self.reset_shared_state();
        *self.selected_input_id.lock().unwrap() = Some(selected_id.to_owned());
    }

    /// Опубликовать частоты поднятого capture для UI. Атомик держит голый `u32`, и
    /// «монитора нет» кодируется в нём нулём — это кодировка канала, а не частота:
    /// `AudioEngine::monitor_output_sample_rate` раскодирует 0 обратно в `None`.
    fn store_capture_rates(&self, sample_rate: SampleRate, output_rate: Option<SampleRate>) {
        self.input_sample_rate.store(sample_rate.0, Ordering::Relaxed);
        self.monitor_output_rate
            .store(output_rate.map_or(0, |rate| rate.0), Ordering::Relaxed);
    }

    /// Поднять capture, играющий дубль с диска вместо устройства.
    ///
    /// Форма — ровно как у двух остальных путей (кольца → фанаут → источник →
    /// воркеры), и это не совпадение: линия реплея обязана быть боевой линией,
    /// а она такая ровно постольку, поскольку её рисует тот же код, которому
    /// всё равно, откуда приехал сэмпл.
    ///
    /// Отличий от живых путей три, и все три — следствия того, что источник
    /// файл, а не устройство:
    ///
    /// - **Частота — файла.** Пайплайны строятся под capture, так что дубль
    ///   44.1 кГц играет как 44.1 (см. `replay::load_take`). Ресемпла нет
    ///   нигде: он был бы обработкой сигнала, который весь смысл иметь сырым.
    /// - **Рекордера нет** (`InputFanout::recorder = None`), см. `replay`.
    /// - **`selected_input_id` не трогаем.** Это выбор УСТРОЙСТВА, к которому
    ///   реплей вернётся по стопу; реплей не устройство и не выбор. Отсюда же
    ///   `selected_id` капчура — ID живого входа, а не путь дубля: смена
    ///   монитора посреди реплея пересоберёт живой вход, что и правильно.
    /// - **Монитор — есть.** Дубль надо СЛЫШАТЬ: разметка границ нот идёт по
    ///   звуку, а не по картинке (это же и есть защита от зеркала).
    fn build_replay_capture(&self, path: PathBuf) -> Result<ActiveCapture, String> {
        let take = load_take(&path)?;
        let sample_rate = take.sample_rate;

        let (analysis_prod, analysis_cons) = analysis_ring(sample_rate);
        let (resonator_prod, resonator_cons) = analysis_ring(sample_rate);
        let (monitor_prod, monitor_cons) = self.monitor_ring(sample_rate);
        let fanout = InputFanout {
            analysis:  analysis_prod,
            resonator: resonator_prod,
            monitor:   monitor_prod,
            recorder:  None,
        };

        let (output_stream, output_rate) = self.start_monitor_output(sample_rate, monitor_cons);
        let analysis = self.start_analysis_worker(sample_rate, analysis_cons);
        let resonator = self.start_resonator_worker(sample_rate, resonator_cons);

        // Чистим состояние ДО первого сэмпла дубля: иначе линия дубля начнётся
        // с хвоста того, что микрофон слышал секунду назад, и первые кадры
        // реплея были бы про другой звук.
        self.store_capture_rates(sample_rate, output_rate);
        self.reset_shared_state();

        let selected_id = self.selected_input_id.lock().unwrap().clone().unwrap_or_default();

        self.replay.publish(ReplayStatus::Playing {
            path,
            seconds: 0.0,
            total_seconds: take.seconds(),
        });
        let source = start_replay_source(take, fanout, self.replay.clone());

        Ok(ActiveCapture {
            input: ActiveInput::Replay(source),
            output_stream,
            analysis,
            resonator,
            // Рекордер на этом пути не поднимается: писать нечего и незачем.
            recorder: None,
            selected_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DIAGNOSTIC: does a take, played back through the **public engine API**, put a
    /// line on the melody history?
    ///
    /// The module's other tests each prove one link — `replay` proves the source hands
    /// over the take in real time, `core`'s rig proves the pipelines draw a line when
    /// fed. Nothing proves the links are *joined*: that Start actually tears down the
    /// live capture, builds a replay one, wires it to the same rings the analysis
    /// workers drain, and that frames come out the other end. Every one of those is a
    /// place where replay could fail silently — the app would look idle and the roll
    /// would stay empty, with no error anywhere.
    ///
    /// Needs no audio device: the monitor is off by default, so the replay capture
    /// opens no output, and the live capture failing to build on a headless box is
    /// exactly the state a user hits when they replay with no mic plugged in.
    ///
    /// Not an assertion of a number — the counts depend on the bow. It fails on zero,
    /// which is the reading that means replay is broken rather than the take quiet.
    #[test]
    #[ignore = "needs testdata/*.wav — run with --ignored --nocapture"]
    fn a_take_replayed_through_the_engine_draws_a_line() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("g_open_slow_strokes.wav");
        let engine = AudioEngine::new();

        engine.start_replay(path.clone());

        // The bank is gated on a UI-driven deadline: nothing publishes melody frames
        // unless a panel keeps asking. The panel that hosts replay does exactly this
        // every frame; a test that forgot would measure the gate, not replay.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut frames = Vec::new();
        let mut cursor = None;
        let mut playing_seen = false;
        loop {
            engine.request_resonator();
            for frame in engine.melody_since(cursor) {
                cursor = Some(frame.seq);
                frames.push(frame);
            }
            match engine.replay_status() {
                ReplayStatus::Playing { .. } => playing_seen = true,
                ReplayStatus::Finished { .. } => break,
                ReplayStatus::Failed(msg) => panic!("replay refused the take: {msg}"),
                other => {
                    assert!(
                        Instant::now() < deadline,
                        "replay never started; stuck at {other:?}"
                    );
                }
            }
            assert!(Instant::now() < deadline, "replay never finished");
            thread::sleep(Duration::from_millis(16)); // ~a UI frame
        }
        // Drain what landed between the last poll and the end of the take.
        for frame in engine.melody_since(cursor) {
            frames.push(frame);
        }

        let voiced = frames.iter().filter(|f| f.pitch.is_some()).count();
        let scored = frames.iter().filter(|f| f.salience.is_some()).count();
        println!("\n=== g_open_slow_strokes replayed through the engine ===");
        println!("  frames      : {}", frames.len());
        println!("  ...scored   : {scored}");
        println!("  ...voiced   : {voiced}");

        assert!(
            playing_seen,
            "replay went straight to Finished — it played nothing"
        );
        assert!(
            !frames.is_empty(),
            "replay finished the take but the engine published no frames — the source is \
                 not wired to the analysis rings"
        );
        assert!(
            voiced > 0,
            "the engine replayed {} frames of a bowed G and decided no pitch, ever",
            frames.len()
        );
    }
}
