#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::io::Read;
    use std::process::{
        Child,
        Command as ProcessCommand,
        Stdio,
    };
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
    use cpal::{
        BufferSize,
        FromSample,
        Sample,
        SupportedBufferSize,
    };
    use resonators::{
        ResonatorBank,
        ResonatorConfig,
        heuristic_alpha,
        midi_to_hz,
    };
    use ringbuf::HeapRb;
    use ringbuf::traits::{
        Consumer,
        Producer,
        Split,
    };
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex32;

    const CPAL_INPUT_ID_PREFIX: &str = "cpal::";
    const PULSE_INPUT_ID_PREFIX: &str = "pulse::";
    const PULSE_DEFAULT_MONITOR_ID: &str = "pulse::@DEFAULT_MONITOR@";
    const PULSE_CAPTURE_RATE: u32 = 48_000;
    const PULSE_CAPTURE_LATENCY_MS: u32 = 20;
    const PULSE_CAPTURE_PROCESS_MS: u32 = 10;
    const LOW_LATENCY_TARGET_FRAMES: u32 = 256;

    // ------------------------------------------------------------------
    // Конфигурация анализа
    // ------------------------------------------------------------------
    const DEFAULT_WINDOW_SIZE: usize = 6144;
    const MIN_WINDOW_SIZE: usize = 2048;
    const MAX_WINDOW_SIZE: usize = 16384;
    const DEFAULT_FFT_SIZE: usize = 16384;
    const MIN_FFT_SIZE: usize = 4096;
    const MAX_FFT_SIZE: usize = 32768;
    const SPECTRUM_BINS: usize = 72;
    const WATERFALL_HISTORY: usize = 52;
    const NOTE_BUCKET_MIN_MIDI: usize = 36;
    const NOTE_BUCKET_MAX_MIDI: usize = 84;
    const SPIRAL_BINS_PER_SEMITONE: usize = 8;
    const SPIRAL_BIN_COUNT: usize =
        (NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI) * SPIRAL_BINS_PER_SEMITONE + 1;
    const ANALYSIS_INTERVAL: Duration = Duration::from_millis(40);
    const SPECTRUM_MIN_FREQUENCY: f32 = 20.0;
    const SPECTRUM_MAX_FREQUENCY: f32 = 2_000.0;
    const NOTE_BUCKET_SPREAD: f32 = 0.35;
    const RESONATOR_MIN_MIDI: usize = NOTE_BUCKET_MIN_MIDI;
    const RESONATOR_MAX_MIDI: usize = NOTE_BUCKET_MAX_MIDI;
    const RESONATOR_DEFAULT_BINS_PER_SEMITONE: usize = 5;
    const YIN_THRESHOLD: f32 = 0.12;
    const SILENCE_RMS_THRESHOLD: f32 = 0.0;
    const INPUT_WAVEFORM_HISTORY: usize = 2048;

    // Gain
    const DEFAULT_INPUT_GAIN: f32 = 1.0;
    const MIN_INPUT_GAIN: f32 = 0.1;
    const MAX_INPUT_GAIN: f32 = 12.0;
    const MONITOR_DEFAULT_GAIN: f32 = 0.35;

    // Время паузы воркера, когда в кольце нет свежих сэмплов
    const ANALYSIS_IDLE_SLEEP: Duration = Duration::from_millis(5);

    // ------------------------------------------------------------------
    // Публичные типы (стабильный API для UI)
    // ------------------------------------------------------------------
    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz:          f32,
        pub note_name:             String,
        pub cents:                 f32,
        pub clarity:               f32,
        pub spectrum:              Vec<f32>,
        pub waterfall:             Vec<Vec<f32>>,
        pub note_spectrum:         Vec<f32>,
        pub note_waterfall:        Vec<Vec<f32>>,
        pub spiral_spectrum:       Vec<f32>,
        pub spiral_waterfall:      Vec<Vec<f32>>,
        pub resonator_spectrum:    Vec<f32>,
        pub resonator_waterfall:   Vec<Vec<f32>>,
        pub resonator_note_labels: Vec<String>,
        pub note_labels:           Vec<String>,
    }

    #[derive(Clone, Debug)]
    pub enum AudioStatus {
        Idle,
        Listening,
        Error(String),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AudioInputKind {
        Microphone,
        System,
        Other,
    }

    #[derive(Clone, Debug)]
    pub struct AudioInputOption {
        pub id:    String,
        pub label: String,
        pub kind:  AudioInputKind,
    }

    #[derive(Clone, Debug)]
    pub struct AnalysisSettings {
        pub window_size:        usize,
        pub fft_size:           usize,
        pub min_frequency:      f32,
        pub max_frequency:      f32,
        pub spectrum_smoothing: usize,
        pub note_spread:        f32,
        pub spectrum_gamma:     f32,
        pub note_gamma:         f32,
        pub resonator_min_midi: usize,
        pub resonator_max_midi: usize,
        pub resonator_bins:     usize,
        pub resonator_alpha:    f32,
        pub resonator_beta:     f32,
        pub resonator_gamma:    f32,
    }

    impl Default for AnalysisSettings {
        fn default() -> Self {
            Self {
                window_size:        DEFAULT_WINDOW_SIZE,
                fft_size:           DEFAULT_FFT_SIZE,
                min_frequency:      SPECTRUM_MIN_FREQUENCY,
                max_frequency:      SPECTRUM_MAX_FREQUENCY,
                spectrum_smoothing: 1,
                note_spread:        NOTE_BUCKET_SPREAD,
                spectrum_gamma:     0.58,
                note_gamma:         0.72,
                resonator_min_midi: RESONATOR_MIN_MIDI,
                resonator_max_midi: RESONATOR_MAX_MIDI,
                resonator_bins:     RESONATOR_DEFAULT_BINS_PER_SEMITONE,
                resonator_alpha:    1.0,
                resonator_beta:     1.0,
                resonator_gamma:    0.72,
            }
        }
    }

    impl AnalysisSettings {
        fn sanitized(mut self) -> Self {
            self.window_size = self.window_size.clamp(MIN_WINDOW_SIZE, MAX_WINDOW_SIZE);
            let min_fft_for_window = self
                .window_size
                .next_power_of_two()
                .clamp(MIN_FFT_SIZE, MAX_FFT_SIZE);
            self.fft_size = self
                .fft_size
                .max(MIN_FFT_SIZE)
                .next_power_of_two()
                .clamp(min_fft_for_window, MAX_FFT_SIZE);
            self.min_frequency = self.min_frequency.clamp(20.0, 1_200.0);
            self.max_frequency = self.max_frequency.clamp(120.0, 4_000.0);
            if self.max_frequency <= self.min_frequency + 40.0 {
                self.max_frequency = (self.min_frequency + 40.0).clamp(120.0, 4_000.0);
            }
            self.spectrum_smoothing = self.spectrum_smoothing.min(4);
            self.note_spread = self.note_spread.clamp(0.15, 0.8);
            self.spectrum_gamma = self.spectrum_gamma.clamp(0.35, 1.2);
            self.note_gamma = self.note_gamma.clamp(0.35, 1.2);
            self.resonator_min_midi = self.resonator_min_midi.clamp(24, 84);
            self.resonator_max_midi = self.resonator_max_midi.clamp(36, 108);
            if self.resonator_max_midi <= self.resonator_min_midi + 6 {
                self.resonator_max_midi = (self.resonator_min_midi + 6).clamp(36, 108);
            }
            self.resonator_bins = self.resonator_bins.clamp(1, 12);
            self.resonator_alpha = self.resonator_alpha.clamp(0.2, 4.0);
            self.resonator_beta = self.resonator_beta.clamp(0.2, 4.0);
            self.resonator_gamma = self.resonator_gamma.clamp(0.35, 1.2);
            self
        }
    }

    // ------------------------------------------------------------------
    // Данные, которые UI читает через AudioEngine
    // ------------------------------------------------------------------
    struct SharedState {
        status:              AudioStatus,
        reading:             Option<TunerReading>,
        input_waveform:      VecDeque<f32>,
        waterfall:           VecDeque<Vec<f32>>,
        note_waterfall:      VecDeque<Vec<f32>>,
        spiral_waterfall:    VecDeque<Vec<f32>>,
        resonator_waterfall: VecDeque<Vec<f32>>,
        smoothed_frequency:  Option<f32>,
    }

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
        command_tx:          Option<Sender<Command>>,
        audio_thread:        Option<JoinHandle<()>>,
    }

    enum Command {
        SwitchInput(Option<String>),
        SetMonitorEnabled(bool),
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let shared = Arc::new(Mutex::new(SharedState {
                status:              AudioStatus::Idle,
                reading:             None,
                input_waveform:      VecDeque::with_capacity(INPUT_WAVEFORM_HISTORY),
                waterfall:           VecDeque::with_capacity(WATERFALL_HISTORY),
                note_waterfall:      VecDeque::with_capacity(WATERFALL_HISTORY),
                spiral_waterfall:    VecDeque::with_capacity(WATERFALL_HISTORY),
                resonator_waterfall: VecDeque::with_capacity(WATERFALL_HISTORY),
                smoothed_frequency:  None,
            }));
            let settings = Arc::new(Mutex::new(AnalysisSettings::default()));
            let input_gain = Arc::new(AtomicU32::new(DEFAULT_INPUT_GAIN.to_bits()));
            let input_level = Arc::new(AtomicU32::new(0.0f32.to_bits()));
            let monitor_enabled = Arc::new(AtomicBool::new(false));
            let monitor_gain = Arc::new(AtomicU32::new(MONITOR_DEFAULT_GAIN.to_bits()));
            let input_sample_rate = Arc::new(AtomicU32::new(0));
            let monitor_output_rate = Arc::new(AtomicU32::new(0));
            let selected_input_id = Arc::new(Mutex::new(None));

            let (command_tx, command_rx) = mpsc::channel::<Command>();

            // Запускаем audio-тред: он единственный владеет cpal::Stream.
            // UI шлёт команды через канал и мгновенно возвращается.
            let audio_thread = thread::spawn({
                let shared = shared.clone();
                let settings = settings.clone();
                let input_gain = input_gain.clone();
                let input_level = input_level.clone();
                let monitor_enabled = monitor_enabled.clone();
                let monitor_gain = monitor_gain.clone();
                let input_sample_rate = input_sample_rate.clone();
                let monitor_output_rate = monitor_output_rate.clone();
                let selected_input_id = selected_input_id.clone();
                move || {
                    audio_thread_main(
                        command_rx,
                        shared,
                        settings,
                        input_gain,
                        input_level,
                        monitor_enabled,
                        monitor_gain,
                        input_sample_rate,
                        monitor_output_rate,
                        selected_input_id,
                    );
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
                command_tx: Some(command_tx),
                audio_thread: Some(audio_thread),
            }
        }

        pub fn status(&self) -> AudioStatus {
            self.shared
                .lock()
                .map(|g| g.status.clone())
                .unwrap_or_else(|_| AudioStatus::Error("Audio state lock poisoned".to_owned()))
        }

        pub fn reading(&self) -> Option<TunerReading> {
            self.shared.lock().ok().and_then(|g| g.reading.clone())
        }

        pub fn analysis_settings(&self) -> AnalysisSettings {
            self.settings.lock().map(|g| g.clone()).unwrap_or_default()
        }

        pub fn set_analysis_settings(&self, settings: AnalysisSettings) {
            if let Ok(mut guard) = self.settings.lock() {
                *guard = settings.sanitized();
            }
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
                .map(|g| g.input_waveform.iter().copied().collect())
                .unwrap_or_default()
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
            self.selected_input_id.lock().ok().and_then(|g| g.clone())
        }

        pub fn set_selected_input_id(&self, input_id: Option<String>) {
            if self.selected_input_id() == input_id {
                return;
            }
            if let Some(tx) = self.command_tx.as_ref() {
                let _ = tx.send(Command::SwitchInput(input_id));
            }
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
    #[allow(clippy::too_many_arguments)]
    fn audio_thread_main(
        rx: Receiver<Command>,
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
        monitor_enabled: Arc<AtomicBool>,
        monitor_gain: Arc<AtomicU32>,
        input_sample_rate: Arc<AtomicU32>,
        monitor_output_rate: Arc<AtomicU32>,
        selected_input_id: Arc<Mutex<Option<String>>>,
    ) {
        let ctx = AudioContext {
            shared,
            settings,
            input_gain,
            input_level,
            monitor_enabled,
            monitor_gain,
            input_sample_rate,
            monitor_output_rate,
            selected_input_id,
        };

        // Стартовый capture: берём дефолтный input.
        // Если не поднялся — оставляем в состоянии Error, UI покажет.
        let mut current = ctx.build_capture(None).ok();
        if current.is_none() {
            ctx.set_error("Could not open default audio input");
        }

        loop {
            let cmd = match rx.recv() {
                Ok(c) => c,
                Err(_) => break, // Engine дропнулся
            };
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
            }
        }

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
    }

    impl AudioContext {
        fn set_error(&self, msg: &str) {
            if let Ok(mut s) = self.shared.lock() {
                s.status = AudioStatus::Error(msg.to_owned());
            }
        }

        fn reset_shared_for_new_capture(&self) {
            if let Ok(mut s) = self.shared.lock() {
                s.reading = None;
                s.input_waveform.clear();
                s.waterfall.clear();
                s.note_waterfall.clear();
                s.spiral_waterfall.clear();
                s.resonator_waterfall.clear();
                s.smoothed_frequency = None;
                s.status = AudioStatus::Listening;
            }
        }

        // Поднимает входной stream, кольцевые буферы, анализ-воркер и
        // (опционально) монитор-выход. Возвращает собранный ActiveCapture.
        fn build_capture(&self, id: Option<String>) -> Result<ActiveCapture, String> {
            if let Some(requested) = id.as_deref() {
                if requested.starts_with(PULSE_INPUT_ID_PREFIX) {
                    return self.build_pulse_capture(requested);
                }
            }

            self.build_cpal_capture(id)
        }

        fn build_cpal_capture(&self, id: Option<String>) -> Result<ActiveCapture, String> {
            let host = cpal::default_host();
            let device = select_input_device(&host, id.as_deref())?;
            let selected_id = cpal_device_route_id(&device);
            let config = device
                .default_input_config()
                .map_err(|e| format!("Input config error: {e}"))?;
            let sample_rate = config.sample_rate();
            let channels = usize::from(config.channels());
            let sample_format = config.sample_format();
            let input_buffer_size = preferred_low_latency_buffer(config.buffer_size());
            let mut stream_config: cpal::StreamConfig = config.into();
            stream_config.buffer_size = input_buffer_size;

            // Кольцевой буфер для анализа. Размер — 0.5с при данном rate,
            // с большим запасом на подёргивания планировщика.
            let (analysis_prod, analysis_cons) = HeapRb::<f32>::new((sample_rate as usize) / 2).split();

            // Кольцевой буфер для монитора-вывода. Создаём только если monitor on.
            // Держим запас небольшим, чтобы монитор не копил лишнюю задержку.
            let (monitor_prod, monitor_cons) = if self.monitor_enabled.load(Ordering::Relaxed) {
                let (p, c) = HeapRb::<f32>::new(low_latency_monitor_ring_len(sample_rate)).split();
                (Some(p), Some(c))
            } else {
                (None, None)
            };

            // Входной stream: callback тупо пушит в кольца, без блокировок и паник.
            let input_stream = match sample_format {
                cpal::SampleFormat::F32 => {
                    build_input::<f32>(&device, &stream_config, channels, analysis_prod, monitor_prod)?
                }
                cpal::SampleFormat::I16 => {
                    build_input::<i16>(&device, &stream_config, channels, analysis_prod, monitor_prod)?
                }
                cpal::SampleFormat::U16 => {
                    build_input::<u16>(&device, &stream_config, channels, analysis_prod, monitor_prod)?
                }
                other => return Err(format!("Unsupported sample format: {other:?}")),
            };
            input_stream
                .play()
                .map_err(|e| format!("Failed to start input stream: {e}"))?;

            // Монитор-выход, если нужен. Ошибку запуска не считаем фатальной —
            // без монитора запись и анализ всё равно работают.
            let (output_stream, output_rate) = match monitor_cons {
                Some(cons) => {
                    match build_monitor_output(sample_rate, cons, self.monitor_gain.clone()) {
                        Ok((stream, rate)) => (Some(stream), rate),
                        Err(_) => (None, 0),
                    }
                }
                None => (None, 0),
            };

            // Анализ-воркер читает из кольца, крутит FFT/YIN, кладёт в shared.
            let analysis = start_analysis_worker(
                sample_rate as f32,
                analysis_cons,
                self.shared.clone(),
                self.settings.clone(),
                self.input_gain.clone(),
                self.input_level.clone(),
            );

            self.input_sample_rate.store(sample_rate, Ordering::Relaxed);
            self.monitor_output_rate.store(output_rate, Ordering::Relaxed);
            self.reset_shared_for_new_capture();
            if let Ok(mut sel) = self.selected_input_id.lock() {
                *sel = Some(selected_id.clone());
            }

            Ok(ActiveCapture {
                input: ActiveInput::Cpal(input_stream),
                output_stream,
                analysis,
                selected_id,
            })
        }

        fn build_pulse_capture(&self, id: &str) -> Result<ActiveCapture, String> {
            let sample_rate = PULSE_CAPTURE_RATE;
            let selected_id = id.to_owned();

            let (analysis_prod, analysis_cons) = HeapRb::<f32>::new((sample_rate as usize) / 2).split();
            let (monitor_prod, monitor_cons) = if self.monitor_enabled.load(Ordering::Relaxed) {
                let (p, c) = HeapRb::<f32>::new(low_latency_monitor_ring_len(sample_rate)).split();
                (Some(p), Some(c))
            } else {
                (None, None)
            };

            let input = ActiveInput::Pulse(build_pulse_input(
                id,
                sample_rate,
                analysis_prod,
                monitor_prod,
                self.shared.clone(),
            )?);

            let (output_stream, output_rate) = match monitor_cons {
                Some(cons) => {
                    match build_monitor_output(sample_rate, cons, self.monitor_gain.clone()) {
                        Ok((stream, rate)) => (Some(stream), rate),
                        Err(_) => (None, 0),
                    }
                }
                None => (None, 0),
            };

            let analysis = start_analysis_worker(
                sample_rate as f32,
                analysis_cons,
                self.shared.clone(),
                self.settings.clone(),
                self.input_gain.clone(),
                self.input_level.clone(),
            );

            self.input_sample_rate.store(sample_rate, Ordering::Relaxed);
            self.monitor_output_rate.store(output_rate, Ordering::Relaxed);
            self.reset_shared_for_new_capture();
            if let Ok(mut sel) = self.selected_input_id.lock() {
                *sel = Some(selected_id.clone());
            }

            Ok(ActiveCapture {
                input,
                output_stream,
                analysis,
                selected_id,
            })
        }
    }

    // ------------------------------------------------------------------
    // ActiveCapture: текущая активная пара stream'ов + воркер.
    // При смене устройства или монитора весь объект дропается целиком;
    // все потоки останавливаются, кольца исчезают.
    // ------------------------------------------------------------------
    struct PulseInputCapture {
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

    enum ActiveInput {
        Cpal(cpal::Stream),
        Pulse(PulseInputCapture),
    }

    struct ActiveCapture {
        input:         ActiveInput,
        output_stream: Option<cpal::Stream>,
        analysis:      AnalysisWorker,
        selected_id:   String,
    }

    impl ActiveCapture {
        fn shutdown(self) {
            match self.input {
                ActiveInput::Cpal(input_stream) => {
                    // ALSA backend cpal может паниковать при drop'е, если callback
                    // успел паникнуть — наш callback не паникует (try_push, без unwrap).
                    // pause() перед drop корректно слайдит трекер-отправитель.
                    let _ = input_stream.pause();
                    drop(input_stream);
                }
                ActiveInput::Pulse(pulse) => pulse.shutdown(),
            }
            if let Some(out) = &self.output_stream {
                let _ = out.pause();
            }
            drop(self.output_stream);
            // Анализ останавливаем после stream'а: callback больше не пишет
            // в кольцо, воркер додренит остатки и выйдет.
            self.analysis.stop();
        }
    }

    // ------------------------------------------------------------------
    // Анализ-воркер
    // ------------------------------------------------------------------
    struct AnalysisWorker {
        stop:   Arc<AtomicBool>,
        thread: JoinHandle<()>,
    }

    impl AnalysisWorker {
        fn stop(self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = self.thread.join();
        }
    }

    fn start_analysis_worker(
        sample_rate: f32,
        mut cons: <HeapRb<f32> as Split>::Cons,
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
    ) -> AnalysisWorker {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();

        let thread = thread::spawn(move || {
            let mut pipeline = AnalysisPipeline::new(sample_rate);
            let mut batch: Vec<f32> = Vec::with_capacity(4096);

            while !stop_flag.load(Ordering::Relaxed) {
                batch.clear();
                // Дренируем сколько есть в кольце, не больше 4096 за раз,
                // чтобы FFT-пауза не превышала одного сэмпл-окна.
                for _ in 0..4096 {
                    match cons.try_pop() {
                        Some(s) => batch.push(s),
                        None => break,
                    }
                }
                if batch.is_empty() {
                    thread::sleep(ANALYSIS_IDLE_SLEEP);
                    continue;
                }
                pipeline.push_samples(batch.drain(..), &shared, &settings, &input_gain, &input_level);
            }
        });

        AnalysisWorker { stop, thread }
    }

    // ------------------------------------------------------------------
    // Построение cpal streams
    // ------------------------------------------------------------------
    fn build_input<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        mut an_prod: <HeapRb<f32> as Split>::Prod,
        mut mon_prod: Option<<HeapRb<f32> as Split>::Prod>,
    ) -> Result<cpal::Stream, String>
    where
        T: Sample + cpal::SizedSample,
        f32: FromSample<T>,
    {
        device
            .build_input_stream(
                config,
                move |data: &[T], _| {
                    // Даункаст в моно: первый канал каждого фрейма.
                    // Нет unwrap/panic — при пустом фрейме просто пропускаем.
                    for frame in data.chunks(channels) {
                        if let Some(raw) = frame.first() {
                            let sample = f32::from_sample(*raw);
                            // try_push: если анализ отстал и кольцо забито,
                            // теряем сэмпл — не блокируем аудио-callback.
                            let _ = an_prod.try_push(sample);
                            if let Some(p) = mon_prod.as_mut() {
                                let _ = p.try_push(sample);
                            }
                        }
                    }
                },
                |err| eprintln!("Input stream error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {e}"))
    }

    fn build_pulse_input(
        input_id: &str,
        sample_rate: u32,
        mut an_prod: <HeapRb<f32> as Split>::Prod,
        mut mon_prod: Option<<HeapRb<f32> as Split>::Prod>,
        shared: Arc<Mutex<SharedState>>,
    ) -> Result<PulseInputCapture, String> {
        let pulse_device = input_id.strip_prefix(PULSE_INPUT_ID_PREFIX).unwrap_or(input_id);
        let rate = sample_rate.to_string();
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
                                let sample = pulse_i16_to_f32([lo, hi]);
                                let _ = an_prod.try_push(sample);
                                if let Some(p) = mon_prod.as_mut() {
                                    let _ = p.try_push(sample);
                                }
                                idx = 1;
                            } else {
                                carry = Some(lo);
                                continue;
                            }
                        }

                        while idx + 1 < n {
                            let sample = pulse_i16_to_f32([buf[idx], buf[idx + 1]]);
                            let _ = an_prod.try_push(sample);
                            if let Some(p) = mon_prod.as_mut() {
                                let _ = p.try_push(sample);
                            }
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

    fn build_monitor_output(
        input_rate: u32,
        mut cons: <HeapRb<f32> as Split>::Cons,
        monitor_gain: Arc<AtomicU32>,
    ) -> Result<(cpal::Stream, u32), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device".to_owned())?;

        // Ищем output-config, поддерживающий ровно наш input rate — тогда
        // никакого ресемпла: step = 1.0, линейная интерполяция вырождается.
        let matching = device
            .supported_output_configs()
            .map_err(|e| format!("Output configs error: {e}"))?
            .find(|c| {
                c.sample_format() == cpal::SampleFormat::F32
                    && c.min_sample_rate() <= input_rate
                    && c.max_sample_rate() >= input_rate
            });

        let (config, actual_rate) = match matching {
            Some(c) => {
                let mut config = c.with_sample_rate(input_rate).config();
                config.buffer_size = preferred_low_latency_buffer(c.buffer_size());
                (config, input_rate)
            }
            None => {
                let default = device
                    .default_output_config()
                    .map_err(|e| format!("Default output config: {e}"))?;
                let mut config = default.config();
                config.buffer_size = preferred_low_latency_buffer(default.buffer_size());
                (config, default.sample_rate())
            }
        };

        let channels = usize::from(config.channels);
        // Линейная интерполяция: если input_rate == actual_rate, step = 1.0 и
        // мы читаем ровно по одному сэмплу на фрейм, без сглаживания.
        let step = input_rate as f32 / actual_rate.max(1) as f32;
        let mut a: f32 = 0.0;
        let mut b: f32 = 0.0;
        let mut phase: f32 = 0.0;

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let gain = f32::from_bits(monitor_gain.load(Ordering::Relaxed)).clamp(0.0, 1.0);
                    for frame in data.chunks_mut(channels) {
                        while phase >= 1.0 {
                            a = b;
                            b = cons.try_pop().unwrap_or(a);
                            phase -= 1.0;
                        }
                        let t = phase.clamp(0.0, 1.0);
                        let sample = (a + (b - a) * t) * gain;
                        phase += step;
                        for out in frame {
                            *out = sample;
                        }
                    }
                },
                |err| eprintln!("Monitor output error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build monitor output: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start monitor output: {e}"))?;
        Ok((stream, actual_rate))
    }

    // ------------------------------------------------------------------
    // Енумерация устройств (только cpal — на Linux это ALSA-устройства,
    // включая pulse-обёрнутые default/pulse). Pulse-мониторы пока не
    // видны — их добавим в step 2 через libpulse-simple для Linux.
    // ------------------------------------------------------------------
    fn enumerate_input_options() -> Vec<AudioInputOption> {
        let host = cpal::default_host();
        let default_device = host.default_input_device();
        let default_name = default_device.as_ref().map(cpal_device_display_name);
        let default_id = default_device.as_ref().map(cpal_device_route_id);
        let Ok(devices) = host.input_devices() else {
            return Vec::new();
        };

        let mut entries: Vec<(String, String, AudioInputKind, bool)> = devices
            .map(|device| {
                let id = cpal_device_route_id(&device);
                let name = cpal_device_display_name(&device);
                let kind = classify_input_kind(&name, default_name.as_deref());
                let is_default = default_id.as_deref() == Some(id.as_str());
                (id, name, kind, is_default)
            })
            .collect();

        // Если ALSA не отметил ни одного устройства как Microphone —
        // помечаем им дефолтное (либо первое не-System), чтобы UI его
        // не прятал.
        if !entries
            .iter()
            .any(|(_, _, kind, _)| *kind == AudioInputKind::Microphone)
        {
            let fallback = entries
                .iter()
                .position(|(_, _, kind, is_default)| *kind != AudioInputKind::System && *is_default)
                .or_else(|| {
                    entries
                        .iter()
                        .position(|(_, _, kind, _)| *kind != AudioInputKind::System)
                });
            if let Some(i) = fallback {
                entries[i].2 = AudioInputKind::Microphone;
            }
        }

        let mut options: Vec<AudioInputOption> = entries
            .into_iter()
            .map(|(id, name, kind, is_default)| {
                AudioInputOption {
                    id,
                    label: format_input_label(&name, kind, is_default),
                    kind,
                }
            })
            .collect();

        if pulse_monitor_input_available() {
            options.push(AudioInputOption {
                id:    PULSE_DEFAULT_MONITOR_ID.to_owned(),
                label: "System • Default monitor (Pulse/PipeWire)".to_owned(),
                kind:  AudioInputKind::System,
            });
        }

        options.sort_by_key(|o| {
            match o.kind {
                AudioInputKind::Microphone => 0,
                AudioInputKind::System => 1,
                AudioInputKind::Other => 2,
            }
        });
        options
    }

    fn select_input_device(host: &cpal::Host, requested: Option<&str>) -> Result<cpal::Device, String> {
        if let Some(requested) = requested {
            if let Some(device_id) = parse_cpal_device_id(requested) {
                if let Some(device) = host.device_by_id(&device_id) {
                    return Ok(device);
                }
            }
            let devices = host
                .input_devices()
                .map_err(|e| format!("Failed to enumerate input devices: {e}"))?;
            for device in devices {
                let device_name = cpal_device_display_name(&device);
                let device_id = cpal_device_route_id(&device);
                if device_id == requested || device_name == requested {
                    return Ok(device);
                }
            }
            return Err(format!("Input device not found: {requested}"));
        }
        host.default_input_device()
            .ok_or_else(|| "No input device found".to_owned())
    }

    fn classify_input_kind(name: &str, default_name: Option<&str>) -> AudioInputKind {
        let lowered = name.to_lowercase();
        let system_markers = [
            "monitor",
            "loopback",
            "stereo mix",
            "what u hear",
            "blackhole",
            "soundflower",
        ];
        if system_markers.iter().any(|m| lowered.contains(m)) {
            AudioInputKind::System
        } else if default_name == Some(name) {
            AudioInputKind::Microphone
        } else {
            AudioInputKind::Other
        }
    }

    fn format_input_label(name: &str, kind: AudioInputKind, is_default: bool) -> String {
        let tag = match kind {
            AudioInputKind::Microphone => "Mic",
            AudioInputKind::System => "System",
            AudioInputKind::Other => "Input",
        };
        if is_default {
            format!("{tag} • {name} (Default)")
        } else {
            format!("{tag} • {name}")
        }
    }

    fn cpal_device_display_name(device: &cpal::Device) -> String {
        device
            .description()
            .map(|desc| desc.name().to_owned())
            .unwrap_or_else(|_| "Unknown input".to_owned())
    }

    fn cpal_device_route_id(device: &cpal::Device) -> String {
        match device.id() {
            Ok(id) => format!("{CPAL_INPUT_ID_PREFIX}{id}"),
            Err(_) => {
                format!(
                    "{CPAL_INPUT_ID_PREFIX}compat::{}",
                    cpal_device_display_name(device)
                )
            }
        }
    }

    fn parse_cpal_device_id(requested: &str) -> Option<cpal::DeviceId> {
        requested
            .strip_prefix(CPAL_INPUT_ID_PREFIX)?
            .parse::<cpal::DeviceId>()
            .ok()
    }

    fn preferred_low_latency_buffer(range: &SupportedBufferSize) -> BufferSize {
        match range {
            SupportedBufferSize::Range { min, max } => {
                let requested = LOW_LATENCY_TARGET_FRAMES.clamp(*min, *max);
                BufferSize::Fixed(requested)
            }
            SupportedBufferSize::Unknown => BufferSize::Default,
        }
    }

    fn low_latency_monitor_ring_len(sample_rate: u32) -> usize {
        ((sample_rate as usize) * 3 / 100).max(256)
    }

    fn pulse_monitor_input_available() -> bool {
        ProcessCommand::new("parec")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    fn pulse_i16_to_f32(bytes: [u8; 2]) -> f32 {
        f32::from(i16::from_le_bytes(bytes)) / 32768.0
    }

    fn set_shared_error(shared: &Arc<Mutex<SharedState>>, msg: &str) {
        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Error(msg.to_owned());
        }
    }

    // ------------------------------------------------------------------
    // AnalysisPipeline: чистые функции, ничего аудио-специфичного.
    // ------------------------------------------------------------------
    struct AnalysisPipeline {
        buffer:         VecDeque<f32>,
        last_analysis:  Instant,
        planner:        FftPlanner<f32>,
        resonator_view: ResonatorViewSettings,
        sample_rate:    f32,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct ResonatorViewSettings {
        min_midi:          usize,
        max_midi:          usize,
        bins_per_semitone: usize,
        alpha_scale:       f32,
        beta_scale:        f32,
        gamma:             f32,
    }

    #[derive(Clone, Debug)]
    struct ResonatorSnapshot {
        spectrum:    Vec<f32>,
        note_labels: Vec<String>,
    }

    #[derive(Clone, Copy, Debug)]
    struct PitchEstimate {
        frequency_hz: f32,
        clarity:      f32,
    }

    #[derive(Clone, Debug)]
    struct AnalysisFrame {
        pitch:                 Option<PitchEstimate>,
        spectrum:              Vec<f32>,
        note_spectrum:         Vec<f32>,
        spiral_spectrum:       Vec<f32>,
        resonator_spectrum:    Vec<f32>,
        resonator_note_labels: Vec<String>,
    }

    impl AnalysisPipeline {
        fn new(sample_rate: f32) -> Self {
            Self {
                buffer: VecDeque::with_capacity(MAX_WINDOW_SIZE * 2),
                last_analysis: Instant::now() - ANALYSIS_INTERVAL,
                planner: FftPlanner::new(),
                resonator_view: ResonatorViewSettings::default(),
                sample_rate,
            }
        }

        fn push_samples(
            &mut self,
            samples: impl IntoIterator<Item = f32>,
            shared: &Arc<Mutex<SharedState>>,
            settings: &Arc<Mutex<AnalysisSettings>>,
            input_gain: &Arc<AtomicU32>,
            input_level: &Arc<AtomicU32>,
        ) {
            let analysis_settings = settings.lock().map(|g| g.clone()).unwrap_or_default().sanitized();
            self.sync_resonator_view(&analysis_settings, shared);
            let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
            let mut recent: Vec<f32> = Vec::new();

            // Применяем гейн без хард-клипа: обрезка в signal-path рождает
            // гармоники, сбивает YIN/FFT. Отрисовка сама клипит на ±1 при ренде.
            for s in samples {
                let scaled = s * gain;
                self.buffer.push_back(scaled);
                recent.push(scaled);
            }

            append_input_waveform(shared, &recent);

            while self.buffer.len() > MAX_WINDOW_SIZE * 2 {
                self.buffer.pop_front();
            }

            if self.buffer.len() < analysis_settings.window_size
                || self.last_analysis.elapsed() < ANALYSIS_INTERVAL
            {
                return;
            }
            self.last_analysis = Instant::now();

            let start = self.buffer.len().saturating_sub(analysis_settings.window_size);
            let window: Vec<f32> = self.buffer.iter().skip(start).copied().collect();
            let level = normalized_level(&window);
            let previous_level = f32::from_bits(input_level.load(Ordering::Relaxed));
            let smoothed_level_value = smoothed_level(previous_level, level);
            input_level.store(smoothed_level_value.to_bits(), Ordering::Relaxed);
            let resonator_snap =
                resonator_snapshot_for_window(&window, self.sample_rate, &self.resonator_view);

            let frame = analyze_window(
                &window,
                self.sample_rate,
                &analysis_settings,
                &mut self.planner,
                resonator_snap,
            );
            publish_reading(shared, frame);
        }

        fn sync_resonator_view(&mut self, settings: &AnalysisSettings, shared: &Arc<Mutex<SharedState>>) {
            let requested = ResonatorViewSettings::from(settings);
            if requested == self.resonator_view {
                return;
            }
            self.resonator_view = requested;
            if let Ok(mut state) = shared.lock() {
                state.resonator_waterfall.clear();
                if let Some(reading) = state.reading.as_mut() {
                    reading.resonator_spectrum.clear();
                    reading.resonator_waterfall.clear();
                    reading.resonator_note_labels =
                        resonator_note_labels(self.resonator_view.min_midi, self.resonator_view.max_midi);
                }
            }
        }
    }

    impl Default for ResonatorViewSettings {
        fn default() -> Self {
            Self {
                min_midi:          RESONATOR_MIN_MIDI,
                max_midi:          RESONATOR_MAX_MIDI,
                bins_per_semitone: RESONATOR_DEFAULT_BINS_PER_SEMITONE,
                alpha_scale:       1.0,
                beta_scale:        1.0,
                gamma:             0.72,
            }
        }
    }

    impl From<&AnalysisSettings> for ResonatorViewSettings {
        fn from(s: &AnalysisSettings) -> Self {
            Self {
                min_midi:          s.resonator_min_midi,
                max_midi:          s.resonator_max_midi,
                bins_per_semitone: s.resonator_bins,
                alpha_scale:       s.resonator_alpha,
                beta_scale:        s.resonator_beta,
                gamma:             s.resonator_gamma,
            }
        }
    }

    // ------------------------------------------------------------------
    // Чистые функции анализа (FFT / YIN / резонаторы / метки)
    // ------------------------------------------------------------------
    fn normalized_level(window: &[f32]) -> f32 {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        if rms <= f32::EPSILON {
            return 0.0;
        }
        let db = 20.0 * rms.log10();
        ((db + 54.0) / 48.0).clamp(0.0, 1.0)
    }

    fn smoothed_level(previous: f32, current: f32) -> f32 {
        let alpha = if current > previous { 0.32 } else { 0.12 };
        previous + (current - previous) * alpha
    }

    fn analyze_window(
        window: &[f32],
        sample_rate: f32,
        settings: &AnalysisSettings,
        planner: &mut FftPlanner<f32>,
        resonator_snapshot: ResonatorSnapshot,
    ) -> AnalysisFrame {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        let windowed = apply_hann_window(window);

        let (spectrum, note_spectrum, spiral_spectrum) =
            spectrum_bars(&windowed, sample_rate, settings, planner);
        let pitch = if rms < SILENCE_RMS_THRESHOLD {
            None
        } else {
            detect_pitch_yin(window, sample_rate).and_then(|(f, c)| {
                (45.0..=1200.0).contains(&f).then_some(PitchEstimate {
                    frequency_hz: f,
                    clarity:      c,
                })
            })
        };

        AnalysisFrame {
            pitch,
            spectrum,
            note_spectrum,
            spiral_spectrum,
            resonator_spectrum: resonator_snapshot.spectrum,
            resonator_note_labels: resonator_snapshot.note_labels,
        }
    }

    fn apply_hann_window(input: &[f32]) -> Vec<f32> {
        let len = input.len() as f32;
        input
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let phase = (2.0 * std::f32::consts::PI * i as f32) / (len - 1.0);
                let mult = 0.5 * (1.0 - phase.cos());
                s * mult
            })
            .collect()
    }

    fn detect_pitch_yin(window: &[f32], sample_rate: f32) -> Option<(f32, f32)> {
        let min_lag = (sample_rate / 1000.0).max(1.0) as usize;
        let max_lag = (sample_rate / 45.0) as usize;
        let search_end = max_lag.min(window.len().saturating_sub(1));
        if min_lag >= search_end {
            return None;
        }

        let mut difference = vec![0.0f32; search_end + 1];
        let mut cumulative = vec![0.0f32; search_end + 1];

        for tau in 1..=search_end {
            let limit = window.len().saturating_sub(tau);
            let mut sum = 0.0;
            for i in 0..limit {
                let d = window[i] - window[i + tau];
                sum += d * d;
            }
            difference[tau] = sum;
        }

        cumulative[0] = 1.0;
        let mut running_sum = 0.0;
        for tau in 1..=search_end {
            running_sum += difference[tau];
            cumulative[tau] = if running_sum > 0.0 {
                difference[tau] * tau as f32 / running_sum
            } else {
                1.0
            };
        }

        let mut best_tau = None;
        for tau in min_lag..search_end {
            if cumulative[tau] < YIN_THRESHOLD && cumulative[tau] <= cumulative[tau + 1] {
                best_tau = Some(tau);
                break;
            }
        }

        let tau = best_tau.unwrap_or_else(|| {
            (min_lag..=search_end)
                .min_by(|l, r| cumulative[*l].total_cmp(&cumulative[*r]))
                .unwrap_or(min_lag)
        });

        let tau = parabolic_tau(&cumulative, tau);
        if !tau.is_finite() || tau <= 0.0 {
            return None;
        }
        let tau = tau.clamp(min_lag as f32, search_end as f32);
        let tau_index = tau.round().clamp(min_lag as f32, search_end as f32) as usize;
        let clarity = (1.0 - cumulative[tau_index].clamp(0.0, 1.0)).clamp(0.0, 1.0);

        Some((sample_rate / tau, clarity))
    }

    fn spectrum_bars(
        window: &[f32],
        sample_rate: f32,
        settings: &AnalysisSettings,
        planner: &mut FftPlanner<f32>,
    ) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let fft_size = settings.fft_size.max(window.len().next_power_of_two());
        let mut input = vec![Complex32::new(0.0, 0.0); fft_size];
        for (slot, sample) in input.iter_mut().zip(window.iter().copied()) {
            slot.re = sample;
        }
        let fft = planner.plan_fft_forward(input.len());
        fft.process(&mut input);

        let magnitudes: Vec<f32> = input.iter().take(input.len() / 2).map(|v| v.norm_sqr()).collect();

        let hz_per_bin = sample_rate / input.len() as f32;
        let mut bars: Vec<f32> = vec![0.0; SPECTRUM_BINS];
        let mut note_bars: Vec<f32> = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
        let mut spiral_bars: Vec<f32> = vec![0.0; SPIRAL_BIN_COUNT];

        for (i, magnitude) in magnitudes.iter().enumerate() {
            let frequency = i as f32 * hz_per_bin;
            if !(settings.min_frequency..=settings.max_frequency).contains(&frequency) {
                continue;
            }
            if let Some(bucket) =
                spectrum_bucket_index(frequency, settings.min_frequency, settings.max_frequency)
            {
                bars[bucket] += *magnitude;
            }
            accumulate_note_energy(&mut note_bars, frequency, *magnitude, settings.note_spread);
            accumulate_spiral_energy(&mut spiral_bars, frequency, *magnitude);
        }

        normalize_bars(&mut bars, settings.spectrum_gamma);
        normalize_bars(&mut note_bars, settings.note_gamma);
        normalize_bars(&mut spiral_bars, 1.0);
        smooth_bars(&mut bars, settings.spectrum_smoothing);

        (bars, note_bars, spiral_bars)
    }

    fn build_resonator_bank(sample_rate: f32, settings: &ResonatorViewSettings) -> ResonatorBank {
        let bin_count = (settings.max_midi - settings.min_midi) * settings.bins_per_semitone + 1;
        let configs: Vec<ResonatorConfig> = (0..bin_count)
            .map(|i| {
                let midi = settings.min_midi as f32 + i as f32 / settings.bins_per_semitone as f32;
                let frequency = midi_to_hz(midi, 440.0);
                let alpha =
                    (heuristic_alpha(frequency, sample_rate) * settings.alpha_scale).clamp(0.0001, 1.0);
                let beta = (heuristic_alpha(frequency, sample_rate) * settings.beta_scale).clamp(0.0001, 1.0);
                ResonatorConfig::new(frequency, alpha, beta)
            })
            .collect();
        ResonatorBank::new(&configs, sample_rate)
    }

    fn resonator_snapshot(bank: &ResonatorBank, settings: &ResonatorViewSettings) -> ResonatorSnapshot {
        let mut spectrum = bank.magnitudes();
        normalize_bars(&mut spectrum, settings.gamma);
        ResonatorSnapshot {
            spectrum,
            note_labels: resonator_note_labels(settings.min_midi, settings.max_midi),
        }
    }

    fn resonator_snapshot_for_window(
        window: &[f32],
        sample_rate: f32,
        settings: &ResonatorViewSettings,
    ) -> ResonatorSnapshot {
        let mut bank = build_resonator_bank(sample_rate, settings);
        for sample in window.iter().copied() {
            bank.process_sample(sample);
        }
        resonator_snapshot(&bank, settings)
    }

    fn frequency_to_note(frequency_hz: f32) -> (String, f32) {
        const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let midi = 69.0 + 12.0 * (frequency_hz / 440.0).log2();
        let nearest = midi.round();
        let cents = (midi - nearest) * 100.0;
        let note_index = ((nearest as i32).rem_euclid(12)) as usize;
        let octave = (nearest as i32 / 12) - 1;
        (format!("{}{}", NOTE_NAMES[note_index], octave), cents)
    }

    fn parabolic_tau(values: &[f32], tau: usize) -> f32 {
        if tau == 0 || tau + 1 >= values.len() {
            return tau as f32;
        }
        let left = values[tau - 1];
        let center = values[tau];
        let right = values[tau + 1];
        let denom = left - 2.0 * center + right;
        if denom.abs() < f32::EPSILON {
            tau as f32
        } else {
            tau as f32 + 0.5 * (left - right) / denom
        }
    }

    fn smooth_frequency(previous: Option<f32>, next: f32) -> f32 {
        match previous {
            Some(prev) => {
                let corrected = correct_octave_jump(prev, next);
                let ratio = (corrected / prev).max(prev / corrected);
                let alpha = if ratio > 1.04 { 0.18 } else { 0.10 };
                prev + (corrected - prev) * alpha
            }
            None => next,
        }
    }

    fn correct_octave_jump(previous: f32, next: f32) -> f32 {
        let ratio = next / previous;
        if (1.85..=2.15).contains(&ratio) {
            next * 0.5
        } else if (0.46..=0.54).contains(&ratio) {
            next * 2.0
        } else {
            next
        }
    }

    fn normalize_bars(values: &mut [f32], gamma: f32) {
        let max = values.iter().copied().fold(0.0, f32::max);
        if max > 0.0 {
            for v in values {
                *v = (*v / max).clamp(0.0, 1.0).powf(gamma);
            }
        }
    }

    fn smooth_bars(values: &mut [f32], passes: usize) {
        if values.len() < 3 || passes == 0 {
            return;
        }
        let mut scratch = values.to_vec();
        for _ in 0..passes {
            scratch.copy_from_slice(values);
            for i in 0..values.len() {
                let l = scratch[i.saturating_sub(1)];
                let c = scratch[i];
                let r = scratch[(i + 1).min(scratch.len() - 1)];
                values[i] = l * 0.2 + c * 0.6 + r * 0.2;
            }
        }
    }

    fn spectrum_bucket_index(frequency: f32, min_frequency: f32, max_frequency: f32) -> Option<usize> {
        if !(min_frequency..=max_frequency).contains(&frequency) {
            return None;
        }
        let min_log = min_frequency.log2();
        let max_log = max_frequency.log2();
        let normalized = ((frequency.log2() - min_log) / (max_log - min_log)).clamp(0.0, 1.0);
        Some((normalized * (SPECTRUM_BINS - 1) as f32).round() as usize)
    }

    fn accumulate_note_energy(note_bars: &mut [f32], frequency: f32, energy: f32, note_spread: f32) {
        if frequency <= 0.0 || note_bars.is_empty() {
            return;
        }
        let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
        let note_position = midi - NOTE_BUCKET_MIN_MIDI as f32;
        let center = note_position.round() as isize;
        for index in (center - 2)..=(center + 2) {
            if !(0..note_bars.len() as isize).contains(&index) {
                continue;
            }
            let distance = (index as f32 - note_position).abs();
            if distance > 1.25 {
                continue;
            }
            let weight = (-0.5 * (distance / note_spread).powi(2)).exp();
            note_bars[index as usize] += energy * weight;
        }
    }

    fn accumulate_spiral_energy(spiral_bars: &mut [f32], frequency: f32, energy: f32) {
        if frequency <= 0.0 || spiral_bars.is_empty() {
            return;
        }
        let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
        if !(NOTE_BUCKET_MIN_MIDI as f32..=NOTE_BUCKET_MAX_MIDI as f32).contains(&midi) {
            return;
        }
        let position = (midi - NOTE_BUCKET_MIN_MIDI as f32) * SPIRAL_BINS_PER_SEMITONE as f32;
        let left_index = position.floor() as usize;
        let frac = position - left_index as f32;
        if left_index < spiral_bars.len() {
            spiral_bars[left_index] += energy * (1.0 - frac);
        }
        if left_index + 1 < spiral_bars.len() {
            spiral_bars[left_index + 1] += energy * frac;
        }
    }

    fn note_bucket_labels() -> Vec<String> {
        (NOTE_BUCKET_MIN_MIDI..=NOTE_BUCKET_MAX_MIDI)
            .map(|m| midi_to_note_label(m as i32))
            .collect()
    }

    fn resonator_note_labels(min_midi: usize, max_midi: usize) -> Vec<String> {
        (min_midi..=max_midi)
            .map(|m| midi_to_note_label(m as i32))
            .collect()
    }

    fn midi_to_note_label(midi: i32) -> String {
        const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let note_index = midi.rem_euclid(12) as usize;
        let octave = midi / 12 - 1;
        format!("{}{}", NOTE_NAMES[note_index], octave)
    }

    fn append_input_waveform(shared: &Arc<Mutex<SharedState>>, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        if let Ok(mut state) = shared.lock() {
            state.input_waveform.extend(samples.iter().copied());
            while state.input_waveform.len() > INPUT_WAVEFORM_HISTORY {
                state.input_waveform.pop_front();
            }
        }
    }

    fn publish_reading(shared: &Arc<Mutex<SharedState>>, frame: AnalysisFrame) {
        if let Ok(mut state) = shared.lock() {
            let (smoothed_frequency, clarity) = match frame.pitch {
                Some(pitch) => {
                    let sf = smooth_frequency(state.smoothed_frequency, pitch.frequency_hz);
                    state.smoothed_frequency = Some(sf);
                    (sf, pitch.clarity)
                }
                None => {
                    let Some(sf) = state.smoothed_frequency else {
                        return;
                    };
                    (sf, 0.0)
                }
            };

            let (note_name, cents) = frequency_to_note(smoothed_frequency);
            state.waterfall.push_back(frame.spectrum.clone());
            state.note_waterfall.push_back(frame.note_spectrum.clone());
            state.spiral_waterfall.push_back(frame.spiral_spectrum.clone());
            state
                .resonator_waterfall
                .push_back(frame.resonator_spectrum.clone());
            while state.waterfall.len() > WATERFALL_HISTORY {
                state.waterfall.pop_front();
            }
            while state.note_waterfall.len() > WATERFALL_HISTORY {
                state.note_waterfall.pop_front();
            }
            while state.spiral_waterfall.len() > WATERFALL_HISTORY {
                state.spiral_waterfall.pop_front();
            }
            while state.resonator_waterfall.len() > WATERFALL_HISTORY {
                state.resonator_waterfall.pop_front();
            }

            state.reading = Some(TunerReading {
                frequency_hz: smoothed_frequency,
                note_name,
                cents,
                clarity,
                spectrum: frame.spectrum,
                waterfall: state.waterfall.iter().cloned().collect(),
                note_spectrum: frame.note_spectrum,
                note_waterfall: state.note_waterfall.iter().cloned().collect(),
                spiral_spectrum: frame.spiral_spectrum,
                spiral_waterfall: state.spiral_waterfall.iter().cloned().collect(),
                resonator_spectrum: frame.resonator_spectrum,
                resonator_waterfall: state.resonator_waterfall.iter().cloned().collect(),
                resonator_note_labels: frame.resonator_note_labels,
                note_labels: note_bucket_labels(),
            });
            state.status = AudioStatus::Listening;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            AnalysisSettings,
            MIN_WINDOW_SIZE,
            NOTE_BUCKET_MAX_MIDI,
            NOTE_BUCKET_MIN_MIDI,
            NOTE_BUCKET_SPREAD,
            accumulate_note_energy,
            detect_pitch_yin,
            parabolic_tau,
            spectrum_bucket_index,
        };

        fn sine_wave(frequency_hz: f32, sample_rate: f32, len: usize) -> Vec<f32> {
            (0..len)
                .map(|i| {
                    let phase = i as f32 * frequency_hz * std::f32::consts::TAU / sample_rate;
                    phase.sin()
                })
                .collect()
        }

        #[test]
        fn parabolic_tau_can_overshoot_without_producing_invalid_index() {
            let values = vec![0.0, 0.5, 0.0, -0.499];
            let refined = parabolic_tau(&values, 2);
            assert!(refined > values.len() as f32);

            let window = vec![1.0; 981];
            let result = std::panic::catch_unwind(|| detect_pitch_yin(&window, 44_100.0));
            assert!(result.is_ok());
        }

        #[test]
        fn spectrum_bucket_index_is_monotonic_in_log_space() {
            let low = spectrum_bucket_index(40.0, 20.0, 2_000.0).unwrap();
            let mid = spectrum_bucket_index(160.0, 20.0, 2_000.0).unwrap();
            let high = spectrum_bucket_index(640.0, 20.0, 2_000.0).unwrap();
            assert!(low < mid);
            assert!(mid < high);
        }

        #[test]
        fn note_energy_prefers_the_closest_semitone() {
            let mut bars = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
            accumulate_note_energy(&mut bars, 440.0, 1.0, NOTE_BUCKET_SPREAD);
            let a4_index = 69 - NOTE_BUCKET_MIN_MIDI;

            let strongest = bars
                .iter()
                .enumerate()
                .max_by(|(_, l), (_, r)| l.total_cmp(r))
                .map(|(i, _)| i)
                .unwrap();
            assert_eq!(strongest, a4_index);
            assert!(bars[a4_index] > bars[a4_index - 1]);
            assert!(bars[a4_index] > bars[a4_index + 1]);
        }

        #[test]
        fn yin_detects_c2_on_raw_signal() {
            let sample_rate = 44_100.0;
            let expected = 65.40639;
            let window = sine_wave(expected, sample_rate, 6144);
            let (detected, _clarity) = detect_pitch_yin(&window, sample_rate).unwrap();
            assert!(
                (detected - expected).abs() < 1.0,
                "detected {detected} expected {expected}"
            );
        }

        #[test]
        fn yin_detects_c3_on_raw_signal() {
            let sample_rate = 44_100.0;
            let expected = 130.81278;
            let window = sine_wave(expected, sample_rate, 6144);
            let (detected, _clarity) = detect_pitch_yin(&window, sample_rate).unwrap();
            assert!(
                (detected - expected).abs() < 1.0,
                "detected {detected} expected {expected}"
            );
        }

        #[test]
        fn analysis_settings_are_sanitized() {
            let settings = AnalysisSettings {
                window_size:        500,
                fft_size:           1_000,
                min_frequency:      900.0,
                max_frequency:      920.0,
                spectrum_smoothing: 12,
                note_spread:        0.01,
                spectrum_gamma:     0.01,
                note_gamma:         9.0,
                resonator_min_midi: 10,
                resonator_max_midi: 11,
                resonator_bins:     99,
                resonator_alpha:    0.01,
                resonator_beta:     9.0,
                resonator_gamma:    9.0,
            }
            .sanitized();

            assert!(settings.window_size >= MIN_WINDOW_SIZE);
            assert!(settings.fft_size >= settings.window_size.next_power_of_two());
            assert!(settings.max_frequency > settings.min_frequency);
            assert!(settings.spectrum_smoothing <= 4);
            assert!((0.15..=0.8).contains(&settings.note_spread));
            assert!(settings.resonator_max_midi > settings.resonator_min_midi);
            assert!((1..=12).contains(&settings.resonator_bins));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod native {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AudioInputKind {
        Microphone,
        System,
        Other,
    }

    #[derive(Clone, Debug)]
    pub struct AudioInputOption {
        pub id:    String,
        pub label: String,
        pub kind:  AudioInputKind,
    }

    #[derive(Clone, Debug)]
    pub struct AnalysisSettings {
        pub window_size:        usize,
        pub fft_size:           usize,
        pub min_frequency:      f32,
        pub max_frequency:      f32,
        pub spectrum_smoothing: usize,
        pub note_spread:        f32,
        pub spectrum_gamma:     f32,
        pub note_gamma:         f32,
        pub resonator_min_midi: usize,
        pub resonator_max_midi: usize,
        pub resonator_bins:     usize,
        pub resonator_alpha:    f32,
        pub resonator_beta:     f32,
        pub resonator_gamma:    f32,
    }

    impl Default for AnalysisSettings {
        fn default() -> Self {
            Self {
                window_size:        6144,
                fft_size:           16384,
                min_frequency:      20.0,
                max_frequency:      2_000.0,
                spectrum_smoothing: 1,
                note_spread:        0.35,
                spectrum_gamma:     0.58,
                note_gamma:         0.72,
                resonator_min_midi: 36,
                resonator_max_midi: 84,
                resonator_bins:     5,
                resonator_alpha:    1.0,
                resonator_beta:     1.0,
                resonator_gamma:    0.72,
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz:          f32,
        pub note_name:             String,
        pub cents:                 f32,
        pub clarity:               f32,
        pub spectrum:              Vec<f32>,
        pub waterfall:             Vec<Vec<f32>>,
        pub note_spectrum:         Vec<f32>,
        pub note_waterfall:        Vec<Vec<f32>>,
        pub spiral_spectrum:       Vec<f32>,
        pub spiral_waterfall:      Vec<Vec<f32>>,
        pub resonator_spectrum:    Vec<f32>,
        pub resonator_waterfall:   Vec<Vec<f32>>,
        pub resonator_note_labels: Vec<String>,
        pub note_labels:           Vec<String>,
    }

    #[derive(Clone, Debug)]
    pub enum AudioStatus {
        Idle,
        Listening,
        Error(String),
    }

    pub struct AudioEngine;

    impl AudioEngine {
        pub fn new() -> Self {
            Self
        }
        pub fn status(&self) -> AudioStatus {
            AudioStatus::Error("Microphone tuner is not implemented for wasm yet".to_owned())
        }
        pub fn reading(&self) -> Option<TunerReading> {
            None
        }
        pub fn analysis_settings(&self) -> AnalysisSettings {
            AnalysisSettings::default()
        }
        pub fn set_analysis_settings(&self, _settings: AnalysisSettings) {
        }
        pub fn input_gain(&self) -> f32 {
            1.0
        }
        pub fn set_input_gain(&self, _gain: f32) {
        }
        pub fn input_gain_range(&self) -> (f32, f32) {
            (0.1, 12.0)
        }
        pub fn input_level(&self) -> f32 {
            0.0
        }
        pub fn input_waveform(&self) -> Vec<f32> {
            Vec::new()
        }
        pub fn monitor_enabled(&self) -> bool {
            false
        }
        pub fn set_monitor_enabled(&self, _enabled: bool) {
        }
        pub fn monitor_gain(&self) -> f32 {
            0.0
        }
        pub fn set_monitor_gain(&self, _gain: f32) {
        }
        pub fn current_input_sample_rate(&self) -> u32 {
            0
        }
        pub fn monitor_output_sample_rate(&self) -> Option<u32> {
            None
        }
        pub fn default_output_device_name(&self) -> Option<String> {
            None
        }
        pub fn available_inputs(&self) -> Vec<AudioInputOption> {
            Vec::new()
        }
        pub fn selected_input_id(&self) -> Option<String> {
            None
        }
        pub fn set_selected_input_id(&self, _input_id: Option<String>) {
        }
    }
}

pub use native::{
    AnalysisSettings,
    AudioEngine,
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
    TunerReading,
};
