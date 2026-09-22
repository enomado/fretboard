//! Capture: the input side of the engine — where samples come from and where each
//! captured sample fans out to.
//!
//! Three sources (cpal device, Pulse/PipeWire `parec`, replayed take) share one
//! [`InputFanout`] and one [`ActiveCapture`] bundle, so a consumer cannot be wired to
//! one path and forgotten on another.

use std::io::Read;
use std::process::{
    Child,
    Command as ProcessCommand,
    Stdio,
};
use std::sync::atomic::{
    AtomicBool,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
};
use std::thread::{
    self,
    JoinHandle,
};

use cpal::traits::{
    DeviceTrait,
    StreamTrait,
};
use cpal::{
    FromSample,
    Sample,
};
use ringbuf::traits::Producer;

use super::recorder::RecorderTap;
use super::workers::AnalysisWorker;
use super::{
    PULSE_INPUT_ID_PREFIX,
    SampleProducer,
    report_stream_error,
};
use crate::audio::core::{
    SharedState,
    set_shared_error,
};
use crate::audio::sample_rate::SampleRate;

const PULSE_CAPTURE_LATENCY_MS: u32 = 20;
const PULSE_CAPTURE_PROCESS_MS: u32 = 10;

// ------------------------------------------------------------------
// ActiveCapture: текущая активная пара stream'ов + воркер.
// При смене устройства или монитора весь объект дропается целиком;
// все потоки останавливаются, кольца исчезают.
// ------------------------------------------------------------------
pub(super) struct PulseInputCapture {
    stop:   Arc<AtomicBool>,
    child:  Child,
    thread: JoinHandle<()>,
}

impl PulseInputCapture {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.thread.join();
    }
}

pub(super) enum ActiveInput {
    Cpal(cpal::Stream),
    Pulse(PulseInputCapture),
    /// Дубль, проигрываемый с диска в те же кольца. Как и у рекордера,
    /// `AnalysisWorker` тут — просто «поток со стоп-флагом».
    Replay(AnalysisWorker),
}

pub(super) struct ActiveCapture {
    pub(super) input:         ActiveInput,
    pub(super) output_stream: Option<cpal::Stream>,
    pub(super) analysis:      AnalysisWorker,
    pub(super) resonator:     AnalysisWorker,
    /// Писатель дублей. `AnalysisWorker` здесь не про анализ — это просто
    /// «поток со стоп-флагом», и рекордеру нужен ровно он (см. тип).
    ///
    /// `None` на replay-пути: писать нечего (см. `build_replay_capture`).
    pub(super) recorder:      Option<AnalysisWorker>,
    /// The input this capture is rebuilt onto (a monitor toggle re-creates it), in
    /// `build_capture`'s own terms: the live paths hold the device they opened; the
    /// replay path holds the user's device choice (`selected_input_id`), which is
    /// `None` until a live input has ever come up — and `None` there means what it means
    /// to `build_capture`: the default input, as at startup. (It used to be a `String`
    /// with `""` for that case, and the rebuild then asked for a device named `""`.)
    pub(super) selected_id:   Option<String>,
}

impl ActiveCapture {
    pub(super) fn shutdown(self) {
        match self.input {
            ActiveInput::Cpal(input_stream) => {
                // ALSA backend cpal может паниковать при drop'е, если callback
                // успел паникнуть — наш callback не паникует (try_push, без unwrap).
                // pause() перед drop корректно слайдит трекер-отправитель.
                let _ = input_stream.pause();
                drop(input_stream);
            }
            ActiveInput::Pulse(pulse) => pulse.shutdown(),
            // Тред реплея проверяет стоп-флаг раз в чанк, так что join
            // возвращается в пределах 10 мс — стоп посреди 35-секундного дубля
            // не заставляет ждать его конца.
            ActiveInput::Replay(source) => source.stop(),
        }
        if let Some(out) = &self.output_stream {
            let _ = out.pause();
        }
        drop(self.output_stream);
        // Анализ останавливаем после stream'а: callback больше не пишет
        // в кольцо, воркер додренит остатки и выйдет.
        self.analysis.stop();
        self.resonator.stop();
        // Рекордер последним: он закрывает WAV (дописывает длины в RIFF-хедер)
        // и обязан успеть это сделать до того, как мы вернём управление.
        if let Some(recorder) = self.recorder {
            recorder.stop();
        }
    }
}

// ------------------------------------------------------------------
// InputFanout: куда расходится каждый захваченный сэмпл.
//
// Один тип на все пути захвата (cpal и Pulse). Раньше три строки try_push
// повторялись в каждом колбэке; теперь правило живёт в одном месте — и нового
// потребителя нельзя подключить к одному пути, забыв про другой.
// ------------------------------------------------------------------
pub(super) struct InputFanout {
    pub(super) analysis:  SampleProducer,
    pub(super) resonator: SampleProducer,
    pub(super) monitor:   Option<SampleProducer>,
    /// `None` на replay-пути: дубль — это то, что сыграла скрипка, а реплей
    /// проигрывает уже записанный файл. Записать его значило бы сделать копию
    /// с наклейкой «улика». Тапа тут нет физически, а не по договорённости —
    /// и кнопка Record при реплее мертва (`take_panel`).
    pub(super) recorder:  Option<RecorderTap>,
}

impl InputFanout {
    /// Реалтайм: вызывается из аудио-колбэка на каждый сэмпл. Не блокирует и
    /// не аллоцирует.
    ///
    /// **Сэмпл здесь сырой.** `input_gain` применяется позже и в другом месте
    /// — в воркерах, которые дренят эти кольца (`audio::core`). Поэтому дубль
    /// рекордера физически не может увидеть ползунок: умножения ещё не было.
    /// Это свойство конструкции, а не договорённость; не переносить обработку
    /// сюда и не переносить отвод ниже по потоку.
    pub(super) fn push(&mut self, sample: f32) {
        // try_push: если анализ отстал и кольцо забито, теряем сэмпл — не
        // блокируем аудио-callback. Для анализа потеря безобидна: кадр устарел,
        // следующий приедет через миллисекунды.
        let _ = self.analysis.try_push(sample);
        let _ = self.resonator.try_push(sample);
        if let Some(monitor) = self.monitor.as_mut() {
            let _ = monitor.try_push(sample);
        }
        // А здесь потеря НЕ безобидна: это дыра в улике. Считается — см.
        // `RecorderTap::push`. При реплее тапа нет (см. поле).
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.push(sample);
        }
    }
}

pub(super) fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut fanout: InputFanout,
) -> Result<cpal::Stream, String>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                // Даункаст в моно: первый канал каждого фрейма.
                // Нет unwrap/panic — при пустом фрейме просто пропускаем.
                for frame in data.chunks(channels) {
                    if let Some(raw) = frame.first() {
                        fanout.push(f32::from_sample(*raw));
                    }
                }
            },
            |err| report_stream_error("Input stream", &err),
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {e}"))
}

pub(super) fn build_pulse_input(
    input_id: &str,
    sample_rate: SampleRate,
    mut fanout: InputFanout,
    shared: Arc<Mutex<SharedState>>,
) -> Result<PulseInputCapture, String> {
    let pulse_device = input_id.strip_prefix(PULSE_INPUT_ID_PREFIX).unwrap_or(input_id);
    let rate = sample_rate.0.to_string();
    let latency_ms = PULSE_CAPTURE_LATENCY_MS.to_string();
    let process_ms = PULSE_CAPTURE_PROCESS_MS.to_string();

    let mut child = ProcessCommand::new("parec")
        .args([
            "--record",
            "--raw",
            "--format=s16le",
            "--channels=1",
            "--rate",
            &rate,
            "--latency-msec",
            &latency_ms,
            "--process-time-msec",
            &process_ms,
            "--client-name=fretboard",
            "--stream-name=fretboard-input",
            "--device",
            pulse_device,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start PulseAudio capture via parec: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "parec did not provide a readable stdout stream".to_owned())?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    let thread = thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut buf = [0u8; 4096];
        let mut carry: Option<u8> = None;

        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    if !stop_flag.load(Ordering::Relaxed) {
                        set_shared_error(&shared, "PulseAudio capture stopped");
                    }
                    break;
                }
                Ok(n) => {
                    let mut idx = 0usize;

                    if let Some(lo) = carry.take() {
                        if let Some(&hi) = buf.first() {
                            fanout.push(pulse_i16_to_f32([lo, hi]));
                            idx = 1;
                        } else {
                            carry = Some(lo);
                            continue;
                        }
                    }

                    while idx + 1 < n {
                        fanout.push(pulse_i16_to_f32([buf[idx], buf[idx + 1]]));
                        idx += 2;
                    }

                    if idx < n {
                        carry = Some(buf[idx]);
                    }
                }
                Err(err) => {
                    if !stop_flag.load(Ordering::Relaxed) {
                        set_shared_error(&shared, &format!("PulseAudio read error: {err}"));
                    }
                    break;
                }
            }
        }
    });

    Ok(PulseInputCapture { stop, child, thread })
}

pub(super) fn pulse_i16_to_f32(bytes: [u8; 2]) -> f32 {
    f32::from(i16::from_le_bytes(bytes)) / 32768.0
}
