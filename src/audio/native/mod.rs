pub(super) mod imp {
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
        FromSample,
        Sample,
    };
    use resonators::midi_to_hz;
    use ringbuf::HeapRb;
    use ringbuf::traits::{
        Consumer,
        Producer,
        Split,
    };
    use rustfft::FftPlanner;

    use super::super::types::{
        AnalysisSettings,
        AudioInputOption,
        AudioStatus,
        ResonatorReading,
        TunerReading,
    };
    use crate::core_types::pitch::PNote;

    mod analysis_math;
    mod devices;
    mod pitch;
    mod resonator;
    mod spectrum;

    use analysis_math::{
        NOTE_BUCKET_MAX_MIDI,
        NOTE_BUCKET_MIN_MIDI,
        frequency_to_note,
        note_bucket_labels,
        smooth_frequency,
    };
    use devices::{
        cpal_device_display_name,
        enumerate_input_options,
        low_latency_monitor_ring_len,
        preferred_low_latency_buffer,
        select_cpal_capture,
    };
    use pitch::{
        LOWEST_TRACKED_FREQUENCY,
        detect_pitch_yin,
    };
    use resonator::{
        ResonatorAnalyzer,
        ResonatorSnapshot,
        ResonatorViewSettings,
    };
    use spectrum::spectrum_bars_for_window;

    const CPAL_INPUT_ID_PREFIX: &str = "cpal::";
    const CPAL_DEFAULT_OUTPUT_LOOPBACK_ID: &str = "cpal-loopback::@DEFAULT_OUTPUT@";
    const PULSE_INPUT_ID_PREFIX: &str = "pulse::";
    const PULSE_DEFAULT_SOURCE_ID: &str = "pulse::@DEFAULT_SOURCE@";
    const PULSE_DEFAULT_MONITOR_ID: &str = "pulse::@DEFAULT_MONITOR@";
    const PULSE_CAPTURE_RATE: u32 = 48_000;
    const PULSE_CAPTURE_LATENCY_MS: u32 = 20;
    const PULSE_CAPTURE_PROCESS_MS: u32 = 10;
    const LOW_LATENCY_TARGET_FRAMES: u32 = 256;

    // ------------------------------------------------------------------
    // Конфигурация анализа
    // ------------------------------------------------------------------
    const MAX_WINDOW_SIZE: usize = 16384;
    const WATERFALL_HISTORY: usize = 52;
    const ANALYSIS_INTERVAL: Duration = Duration::from_millis(40);
    const SILENCE_RMS_THRESHOLD: f32 = 0.0;
    const INPUT_WAVEFORM_HISTORY: usize = 2048;

    // Gain
    const DEFAULT_INPUT_GAIN: f32 = 1.0;
    const MIN_INPUT_GAIN: f32 = 0.1;
    const MAX_INPUT_GAIN: f32 = 12.0;
    const MONITOR_DEFAULT_GAIN: f32 = 0.35;
    const TEST_TONE_GAIN: f32 = 0.28;
    const TEST_TONE_DURATION: Duration = Duration::from_millis(1_600);

    // Время паузы воркера, когда в кольце нет свежих сэмплов
    const ANALYSIS_IDLE_SLEEP: Duration = Duration::from_millis(5);
    /// Сколько резонаторный банк ещё молотит после последнего запроса от UI.
    /// UI двигает дедлайн каждый кадр, пока панель-потребитель видна; когда панель
    /// закрылась и запросы прекратились, через этот грейс воркер паркуется.
    const RESONATOR_PARK_GRACE: Duration = Duration::from_millis(300);
    /// Сон запаркованного резонаторного воркера между сливами кольца.
    const RESONATOR_PARK_SLEEP: Duration = Duration::from_millis(20);

    type SampleProducer = <HeapRb<f32> as Split>::Prod;
    type SampleConsumer = <HeapRb<f32> as Split>::Cons;

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
        resonator_spectrum:  Vec<f32>,
        resonator_waterfall: VecDeque<Vec<f32>>,
        resonator_labels:    Vec<String>,
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
        resonator_wanted:    Arc<Mutex<Instant>>, // дедлайн «банк нужен до» (гейт)
        command_tx:          Option<Sender<Command>>,
        audio_thread:        Option<JoinHandle<()>>,
    }

    enum Command {
        SwitchInput(Option<String>),
        SetMonitorEnabled(bool),
        PlayTestNote(PNote),
    }

    impl Default for AudioEngine {
        fn default() -> Self {
            Self::new()
        }
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
                resonator_spectrum:  Vec::new(),
                resonator_waterfall: VecDeque::with_capacity(WATERFALL_HISTORY),
                resonator_labels:    Vec::new(),
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
            // Гейт резонатора: дедлайн в прошлом → пока никто не просит, банк не молотит.
            let resonator_wanted = Arc::new(Mutex::new(Instant::now()));

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

        pub fn resonator_reading(&self) -> Option<ResonatorReading> {
            self.shared.lock().ok().and_then(|g| {
                (!g.resonator_spectrum.is_empty()).then(|| {
                    ResonatorReading {
                        spectrum:    g.resonator_spectrum.clone(),
                        waterfall:   g.resonator_waterfall.iter().cloned().collect(),
                        note_labels: g.resonator_labels.clone(),
                    }
                })
            })
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
            if let Some(tx) = self.command_tx.as_ref() {
                let _ = tx.send(Command::SwitchInput(input_id));
            }
        }

        pub fn play_test_note(&self, midi: PNote) {
            if let Some(tx) = self.command_tx.as_ref() {
                let _ = tx.send(Command::PlayTestNote(midi));
            }
        }

        /// Запросить резонаторный банк на ближайший грейс. Панели-потребители
        /// (Scale Finder, Resonator *) зовут это каждый кадр, пока видимы; пока
        /// зовут — воркер молотит, перестали (панель закрылась) — паркуется.
        pub fn request_resonator(&self) {
            if let Ok(mut until) = self.resonator_wanted.lock() {
                *until = Instant::now() + RESONATOR_PARK_GRACE;
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
    fn audio_thread_main(rx: Receiver<Command>, ctx: AudioContext) {
        // Стартовый capture: берём дефолтный input.
        // Если не поднялся — оставляем в состоянии Error, UI покажет.
        let mut current = ctx.build_capture(None).ok();
        if current.is_none() {
            ctx.set_error("Could not open default audio input");
        }

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
        resonator_wanted:    Arc<Mutex<Instant>>,
    }

    impl AudioContext {
        fn set_error(&self, msg: &str) {
            if let Ok(mut s) = self.shared.lock() {
                s.status = AudioStatus::Error(msg.to_owned());
            }
        }

        fn reset_shared_for_test_tone(&self) {
            self.reset_shared_state();
            self.input_level.store(0.0f32.to_bits(), Ordering::Relaxed);
        }

        fn reset_shared_state(&self) {
            if let Ok(mut s) = self.shared.lock() {
                s.reading = None;
                s.input_waveform.clear();
                s.waterfall.clear();
                s.note_waterfall.clear();
                s.spiral_waterfall.clear();
                s.resonator_spectrum.clear();
                s.resonator_waterfall.clear();
                s.resonator_labels.clear();
                s.smoothed_frequency = None;
                s.status = AudioStatus::Listening;
            }
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

        // Поднимает входной stream, кольцевые буферы, анализ-воркер и
        // (опционально) монитор-выход. Возвращает собранный ActiveCapture.
        fn build_capture(&self, id: Option<String>) -> Result<ActiveCapture, String> {
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
            let sample_rate = config.sample_rate();
            let channels = usize::from(config.channels());
            let sample_format = config.sample_format();
            let input_buffer_size = preferred_low_latency_buffer(config.buffer_size());
            let mut stream_config: cpal::StreamConfig = config.into();
            stream_config.buffer_size = input_buffer_size;

            let (analysis_prod, analysis_cons) = analysis_ring(sample_rate);
            let (resonator_prod, resonator_cons) = analysis_ring(sample_rate);
            let (monitor_prod, monitor_cons) = self.monitor_ring(sample_rate);

            // Входной stream: callback тупо пушит в кольца, без блокировок и паник.
            let input_stream = match sample_format {
                cpal::SampleFormat::F32 => {
                    build_input::<f32>(
                        &device,
                        &stream_config,
                        channels,
                        analysis_prod,
                        resonator_prod,
                        monitor_prod,
                    )?
                }
                cpal::SampleFormat::I16 => {
                    build_input::<i16>(
                        &device,
                        &stream_config,
                        channels,
                        analysis_prod,
                        resonator_prod,
                        monitor_prod,
                    )?
                }
                cpal::SampleFormat::U16 => {
                    build_input::<u16>(
                        &device,
                        &stream_config,
                        channels,
                        analysis_prod,
                        resonator_prod,
                        monitor_prod,
                    )?
                }
                other => return Err(format!("Unsupported sample format: {other:?}")),
            };
            input_stream
                .play()
                .map_err(|e| format!("Failed to start input stream: {e}"))?;

            let (output_stream, output_rate) = self.start_monitor_output(sample_rate, monitor_cons);
            let analysis = self.start_analysis_worker(sample_rate, analysis_cons);
            let resonator = self.start_resonator_worker(sample_rate, resonator_cons);
            self.finish_capture_start(sample_rate, output_rate, &selected_id);

            Ok(ActiveCapture {
                input: ActiveInput::Cpal(input_stream),
                output_stream,
                analysis,
                resonator,
                selected_id,
            })
        }

        fn build_pulse_capture(&self, id: &str) -> Result<ActiveCapture, String> {
            let sample_rate = PULSE_CAPTURE_RATE;
            let selected_id = id.to_owned();

            let (analysis_prod, analysis_cons) = analysis_ring(sample_rate);
            let (resonator_prod, resonator_cons) = analysis_ring(sample_rate);
            let (monitor_prod, monitor_cons) = self.monitor_ring(sample_rate);

            let input = ActiveInput::Pulse(build_pulse_input(
                id,
                sample_rate,
                analysis_prod,
                resonator_prod,
                monitor_prod,
                self.shared.clone(),
            )?);

            let (output_stream, output_rate) = self.start_monitor_output(sample_rate, monitor_cons);
            let analysis = self.start_analysis_worker(sample_rate, analysis_cons);
            let resonator = self.start_resonator_worker(sample_rate, resonator_cons);
            self.finish_capture_start(sample_rate, output_rate, &selected_id);

            Ok(ActiveCapture {
                input,
                output_stream,
                analysis,
                resonator,
                selected_id,
            })
        }

        // Кольцевой буфер для монитора-вывода. Создаём только если monitor on.
        // Держим запас небольшим, чтобы монитор не копил лишнюю задержку.
        fn monitor_ring(&self, sample_rate: u32) -> (Option<SampleProducer>, Option<SampleConsumer>) {
            if self.monitor_enabled.load(Ordering::Relaxed) {
                let (prod, cons) = HeapRb::<f32>::new(low_latency_monitor_ring_len(sample_rate)).split();
                (Some(prod), Some(cons))
            } else {
                (None, None)
            }
        }

        // Ошибку запуска монитора не считаем фатальной: запись и анализ
        // должны продолжать работать без playback monitoring.
        fn start_monitor_output(
            &self,
            sample_rate: u32,
            monitor_cons: Option<SampleConsumer>,
        ) -> (Option<cpal::Stream>, u32) {
            match monitor_cons {
                Some(cons) => {
                    match build_monitor_output(sample_rate, cons, self.monitor_gain.clone()) {
                        Ok((stream, rate)) => (Some(stream), rate),
                        Err(_) => (None, 0),
                    }
                }
                None => (None, 0),
            }
        }

        fn start_analysis_worker(&self, sample_rate: u32, analysis_cons: SampleConsumer) -> AnalysisWorker {
            start_analysis_worker(
                sample_rate as f32,
                analysis_cons,
                self.shared.clone(),
                self.settings.clone(),
                self.input_gain.clone(),
                self.input_level.clone(),
            )
        }

        fn start_resonator_worker(&self, sample_rate: u32, resonator_cons: SampleConsumer) -> AnalysisWorker {
            start_resonator_worker(
                sample_rate as f32,
                resonator_cons,
                self.shared.clone(),
                self.settings.clone(),
                self.input_gain.clone(),
                self.resonator_wanted.clone(),
            )
        }

        fn finish_capture_start(&self, sample_rate: u32, output_rate: u32, selected_id: &str) {
            self.input_sample_rate.store(sample_rate, Ordering::Relaxed);
            self.monitor_output_rate.store(output_rate, Ordering::Relaxed);
            self.reset_shared_state();
            if let Ok(mut sel) = self.selected_input_id.lock() {
                *sel = Some(selected_id.to_owned());
            }
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
        resonator:     AnalysisWorker,
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
            self.resonator.stop();
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
        mut cons: SampleConsumer,
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

    fn start_resonator_worker(
        sample_rate: f32,
        mut cons: SampleConsumer,
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        resonator_wanted: Arc<Mutex<Instant>>,
    ) -> AnalysisWorker {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();

        let thread = thread::spawn(move || {
            let mut pipeline = ResonatorPipeline::new(sample_rate);
            let mut batch: Vec<f32> = Vec::with_capacity(4096);

            while !stop_flag.load(Ordering::Relaxed) {
                // Гейт: банк нужен, только пока UI двигает дедлайн. Замок не
                // отравлен/в прошлом → считаем active=false (паркуемся). Если замок
                // отравлен — на стороне безопасности молотим (lock().is_ok() == false
                // ⇒ active=false здесь; но это практически недостижимо).
                let active = resonator_wanted
                    .lock()
                    .map(|until| Instant::now() < *until)
                    .unwrap_or(false);

                batch.clear();
                for _ in 0..4096 {
                    match cons.try_pop() {
                        Some(s) => batch.push(s),
                        None => break,
                    }
                }

                if !active {
                    // Запаркованы: кольцо ВСЁ РАВНО дренируем (иначе переполнится и
                    // при пробуждении выльется пачкой старого звука), но дорогой банк
                    // не считаем — это и есть экономия CPU.
                    thread::sleep(RESONATOR_PARK_SLEEP);
                    continue;
                }
                if batch.is_empty() {
                    thread::sleep(ANALYSIS_IDLE_SLEEP);
                    continue;
                }
                pipeline.push_samples(batch.drain(..), &shared, &settings, &input_gain);
            }
        });

        AnalysisWorker { stop, thread }
    }

    // ------------------------------------------------------------------
    // Построение cpal streams
    // ------------------------------------------------------------------
    // Кольцевой буфер для анализа. Размер — 0.5с при данном rate,
    // с большим запасом на подёргивания планировщика.
    fn analysis_ring(sample_rate: u32) -> (SampleProducer, SampleConsumer) {
        HeapRb::<f32>::new((sample_rate as usize) / 2).split()
    }

    fn build_input<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        mut an_prod: SampleProducer,
        mut res_prod: SampleProducer,
        mut mon_prod: Option<SampleProducer>,
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
                            let sample = f32::from_sample(*raw);
                            // try_push: если анализ отстал и кольцо забито,
                            // теряем сэмпл — не блокируем аудио-callback.
                            let _ = an_prod.try_push(sample);
                            let _ = res_prod.try_push(sample);
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
        mut an_prod: SampleProducer,
        mut res_prod: SampleProducer,
        mut mon_prod: Option<SampleProducer>,
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
                                let _ = res_prod.try_push(sample);
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
                            let _ = res_prod.try_push(sample);
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
        mut cons: SampleConsumer,
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
                config,
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

    fn play_test_note_thread(
        midi: PNote,
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_level: Arc<AtomicU32>,
    ) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device".to_owned())?;
        let mut supported = device
            .supported_output_configs()
            .map_err(|e| format!("Output configs error: {e}"))?;
        let output_config = supported
            .find(|config| config.sample_format() == cpal::SampleFormat::F32)
            .ok_or_else(|| "No f32 output config for test note".to_owned())?;
        let output_rate = 48_000_u32.clamp(output_config.min_sample_rate(), output_config.max_sample_rate());
        let mut config = output_config.with_sample_rate(output_rate).config();
        config.buffer_size = preferred_low_latency_buffer(output_config.buffer_size());
        let sample_rate = config.sample_rate as f32;
        let channels = usize::from(config.channels);
        let frequency = midi_to_hz(midi.as_u8() as f32, 440.0);
        let total_samples = (sample_rate * TEST_TONE_DURATION.as_secs_f32()) as usize;
        let samples = Arc::new(test_tone_samples(frequency, sample_rate, total_samples));
        let playback_samples = samples.clone();
        let playback_index = Arc::new(AtomicU32::new(0));
        let playback_position = playback_index.clone();

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _| {
                    for frame in data.chunks_mut(channels) {
                        let index = playback_position.fetch_add(1, Ordering::Relaxed) as usize;
                        let sample = playback_samples.get(index).copied().unwrap_or(0.0);
                        for out in frame {
                            *out = sample;
                        }
                    }
                },
                |err| eprintln!("Test note output error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build test note output: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start test note output: {e}"))?;

        let input_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let mut pipeline = AnalysisPipeline::new(sample_rate);
        let chunk_len = (sample_rate / 50.0).max(1.0) as usize;
        for chunk in samples.chunks(chunk_len) {
            pipeline.push_samples(
                chunk.iter().copied(),
                &shared,
                &settings,
                &input_gain,
                &input_level,
            );
            thread::sleep(Duration::from_secs_f32(chunk.len() as f32 / sample_rate));
        }

        thread::sleep(Duration::from_millis(120));
        drop(stream);
        Ok(())
    }

    fn test_tone_samples(frequency: f32, sample_rate: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                let attack = (i as f32 / (sample_rate * 0.025)).clamp(0.0, 1.0);
                let release = ((len.saturating_sub(i) as f32) / (sample_rate * 0.08)).clamp(0.0, 1.0);
                let envelope = attack.min(release);
                let phase = std::f32::consts::TAU * frequency * t;
                let sample = 0.55 * phase.sin() + 0.18 * (phase * 2.0).sin() + 0.07 * (phase * 3.0).sin();
                sample * envelope * TEST_TONE_GAIN
            })
            .collect()
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
    struct ResonatorPipeline {
        analyzer:     ResonatorAnalyzer,
        last_publish: Instant,
    }

    struct AnalysisPipeline {
        buffer:        VecDeque<f32>,
        last_analysis: Instant,
        planner:       FftPlanner<f32>,
        sample_rate:   f32,
    }

    #[derive(Clone, Copy, Debug)]
    struct PitchEstimate {
        frequency_hz: f32,
        clarity:      f32,
    }

    #[derive(Clone, Debug)]
    struct AnalysisFrame {
        pitch:           Option<PitchEstimate>,
        spectrum:        Vec<f32>,
        note_spectrum:   Vec<f32>,
        spiral_spectrum: Vec<f32>,
    }

    impl ResonatorPipeline {
        fn new(sample_rate: f32) -> Self {
            Self {
                analyzer:     ResonatorAnalyzer::new(sample_rate),
                last_publish: Instant::now() - Duration::from_millis(16),
            }
        }

        fn push_samples(
            &mut self,
            samples: impl IntoIterator<Item = f32>,
            shared: &Arc<Mutex<SharedState>>,
            settings: &Arc<Mutex<AnalysisSettings>>,
            input_gain: &Arc<AtomicU32>,
        ) {
            let analysis_settings = settings.lock().map(|g| g.clone()).unwrap_or_default().sanitized();
            self.sync_settings(&analysis_settings, shared);

            let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
            let samples: Vec<f32> = samples.into_iter().map(|sample| sample * gain).collect();
            self.analyzer.process_samples(&samples);

            let publish_interval = Duration::from_millis(analysis_settings.resonator.update_ms);
            if self.last_publish.elapsed() < publish_interval {
                return;
            }
            self.last_publish = Instant::now();
            publish_resonator_snapshot(
                shared,
                self.analyzer.snapshot(),
                analysis_settings.resonator.history,
            );
        }

        fn sync_settings(&mut self, settings: &AnalysisSettings, shared: &Arc<Mutex<SharedState>>) {
            let requested = ResonatorViewSettings::from(settings);
            if !self.analyzer.sync_settings(requested) {
                return;
            }
            if let Ok(mut state) = shared.lock() {
                state.resonator_spectrum.clear();
                state.resonator_waterfall.clear();
                state.resonator_labels = self.analyzer.note_labels();
                let resonator_labels = state.resonator_labels.clone();
                if let Some(reading) = state.reading.as_mut() {
                    reading.resonator_spectrum.clear();
                    reading.resonator_waterfall.clear();
                    reading.resonator_note_labels = resonator_labels;
                }
            }
        }
    }

    impl AnalysisPipeline {
        fn new(sample_rate: f32) -> Self {
            Self {
                buffer: VecDeque::with_capacity(MAX_WINDOW_SIZE * 2),
                last_analysis: Instant::now() - ANALYSIS_INTERVAL,
                planner: FftPlanner::new(),
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
            let frame = analyze_window(&window, self.sample_rate, &analysis_settings, &mut self.planner);
            publish_analysis_reading(shared, frame);
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
    ) -> AnalysisFrame {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        let (spectrum, note_spectrum, spiral_spectrum) =
            spectrum_bars_for_window(window, sample_rate, settings, planner);
        let pitch = if rms < SILENCE_RMS_THRESHOLD {
            None
        } else {
            detect_pitch_yin(window, sample_rate).and_then(|(f, c)| {
                (LOWEST_TRACKED_FREQUENCY..=1200.0)
                    .contains(&f)
                    .then_some(PitchEstimate {
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
        }
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

    fn push_limited_history<T>(history: &mut VecDeque<T>, item: T, max_len: usize) {
        history.push_back(item);
        while history.len() > max_len {
            history.pop_front();
        }
    }

    fn publish_analysis_reading(shared: &Arc<Mutex<SharedState>>, frame: AnalysisFrame) {
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
            push_limited_history(&mut state.waterfall, frame.spectrum.clone(), WATERFALL_HISTORY);
            push_limited_history(
                &mut state.note_waterfall,
                frame.note_spectrum.clone(),
                WATERFALL_HISTORY,
            );
            push_limited_history(
                &mut state.spiral_waterfall,
                frame.spiral_spectrum.clone(),
                WATERFALL_HISTORY,
            );
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
                resonator_spectrum: state.resonator_spectrum.clone(),
                resonator_waterfall: state.resonator_waterfall.iter().cloned().collect(),
                resonator_note_labels: state.resonator_labels.clone(),
                note_labels: note_bucket_labels(),
            });
            state.status = AudioStatus::Listening;
        }
    }

    fn publish_resonator_snapshot(
        shared: &Arc<Mutex<SharedState>>,
        snapshot: ResonatorSnapshot,
        history_len: usize,
    ) {
        if let Ok(mut state) = shared.lock() {
            state.resonator_spectrum = snapshot.spectrum;
            state.resonator_labels = snapshot.note_labels;
            let resonator_spectrum = state.resonator_spectrum.clone();
            let resonator_labels = state.resonator_labels.clone();
            push_limited_history(
                &mut state.resonator_waterfall,
                resonator_spectrum.clone(),
                history_len,
            );
            let resonator_waterfall: Vec<Vec<f32>> = state.resonator_waterfall.iter().cloned().collect();

            if let Some(reading) = state.reading.as_mut() {
                reading.resonator_spectrum = resonator_spectrum;
                reading.resonator_waterfall = resonator_waterfall;
                reading.resonator_note_labels = resonator_labels;
            }
        }
    }
}
