#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::io::Read;
    use std::mem::ManuallyDrop;
    use std::process::{
        Child,
        Command,
        Stdio,
    };
    use std::sync::atomic::{
        AtomicBool,
        AtomicU32,
        Ordering,
    };
    use std::sync::{
        Arc,
        Condvar,
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

    use cpal::Sample;
    use cpal::traits::{
        DeviceTrait,
        HostTrait,
        StreamTrait,
    };
    use resonators::{
        ResonatorBank,
        ResonatorConfig,
        heuristic_alpha,
        midi_to_hz,
    };
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex32;

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
    const DEFAULT_INPUT_GAIN: f32 = 4.0;
    const SILENCE_RMS_THRESHOLD: f32 = 0.0;
    const YIN_THRESHOLD: f32 = 0.12;
    const SPECTRUM_MIN_FREQUENCY: f32 = 20.0;
    const SPECTRUM_MAX_FREQUENCY: f32 = 2_000.0;
    const NOTE_BUCKET_SPREAD: f32 = 0.35;
    const RESONATOR_MIN_MIDI: usize = NOTE_BUCKET_MIN_MIDI;
    const RESONATOR_MAX_MIDI: usize = NOTE_BUCKET_MAX_MIDI;
    const RESONATOR_DEFAULT_BINS_PER_SEMITONE: usize = 5;
    const CPAL_INPUT_PREFIX: &str = "cpal::";
    const PULSE_INPUT_PREFIX: &str = "pulse::";
    const PULSE_SAMPLE_RATE: u32 = 44_100;
    const PULSE_DEFAULT_SOURCE: &str = "@DEFAULT_SOURCE@";
    const PULSE_DEFAULT_MONITOR: &str = "@DEFAULT_MONITOR@";
    const INPUT_WAVEFORM_HISTORY: usize = 2048;
    const MONITOR_DEFAULT_GAIN: f32 = 0.35;
    const MONITOR_BUFFER_MAX_FRAMES: usize = 4096;
    struct PulseCapture {
        child:  Child,
        thread: JoinHandle<()>,
    }

    impl PulseCapture {
        fn stop(self) {
            let mut child = self.child;
            let _ = child.kill();
            let _ = child.wait();
            let _ = self.thread.join();
        }
    }

    struct MonitorOutput {
        stream:      cpal::Stream,
        sample_rate: u32,
    }

    impl MonitorOutput {
        fn stop(self) {
            let _ = self.stream.pause();
        }
    }

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

    struct AnalysisMailbox {
        latest: Mutex<Option<Vec<f32>>>,
        wake:   Condvar,
    }

    impl AnalysisMailbox {
        fn new() -> Self {
            Self {
                latest: Mutex::new(None),
                wake:   Condvar::new(),
            }
        }
    }

    struct MonitorBuffer {
        samples:        VecDeque<f32>,
        resample_phase: f32,
    }

    impl MonitorBuffer {
        fn with_capacity(capacity: usize) -> Self {
            Self {
                samples:        VecDeque::with_capacity(capacity),
                resample_phase: 0.0,
            }
        }

        fn clear(&mut self) {
            self.samples.clear();
            self.resample_phase = 0.0;
        }
    }

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
            let resonator_view = ResonatorViewSettings::default();
            Self {
                buffer: VecDeque::with_capacity(MAX_WINDOW_SIZE * 2),
                last_analysis: Instant::now() - ANALYSIS_INTERVAL,
                planner: FftPlanner::new(),
                resonator_view,
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
            let analysis_settings = settings
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
                .sanitized();
            self.sync_resonator_view(&analysis_settings, shared);
            let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
            let mut recent_samples = Vec::new();

            for sample in samples {
                let scaled = (sample * gain).clamp(-1.0, 1.0);
                self.buffer.push_back(scaled);
                recent_samples.push(scaled);
            }

            append_input_waveform(shared, &recent_samples);

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
            let smoothed_level = smoothed_level(previous_level, level);
            input_level.store(smoothed_level.to_bits(), Ordering::Relaxed);
            let resonator_snapshot =
                resonator_snapshot_for_window(&window, self.sample_rate, &self.resonator_view);

            let frame = analyze_window(
                &window,
                self.sample_rate,
                &analysis_settings,
                &mut self.planner,
                resonator_snapshot,
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

    enum StartedInput {
        Cpal {
            stream:          cpal::Stream,
            analysis_worker: AnalysisWorker,
            selected_id:     String,
            sample_rate:     u32,
        },
        Pulse {
            capture:         PulseCapture,
            analysis_worker: AnalysisWorker,
            selected_id:     String,
            sample_rate:     u32,
        },
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

    pub struct AudioEngine {
        shared:              Arc<Mutex<SharedState>>,
        settings:            Arc<Mutex<AnalysisSettings>>,
        input_gain:          Arc<AtomicU32>,
        input_level:         Arc<AtomicU32>,
        monitor_enabled:     Arc<AtomicBool>,
        monitor_gain:        Arc<AtomicU32>,
        monitor_sample_rate: Arc<AtomicU32>,
        monitor_buffer:      Arc<Mutex<MonitorBuffer>>,
        selected_input_id:   Arc<Mutex<Option<String>>>,
        streams:             Arc<Mutex<Vec<ManuallyDrop<cpal::Stream>>>>,
        pulse_capture:       Arc<Mutex<Option<PulseCapture>>>,
        analysis_worker:     Arc<Mutex<Option<AnalysisWorker>>>,
        monitor_output:      Arc<Mutex<Option<MonitorOutput>>>,
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
            let monitor_sample_rate = Arc::new(AtomicU32::new(PULSE_SAMPLE_RATE));
            let monitor_buffer = Arc::new(Mutex::new(MonitorBuffer::with_capacity(
                MONITOR_BUFFER_MAX_FRAMES,
            )));
            let selected_input_id = Arc::new(Mutex::new(None));
            let streams = Arc::new(Mutex::new(Vec::new()));
            let pulse_capture = Arc::new(Mutex::new(None));
            let analysis_worker = Arc::new(Mutex::new(None));
            let monitor_output = Arc::new(Mutex::new(None));
            let preferred_input_id = preferred_initial_input_id();

            let input = start_input(
                shared.clone(),
                settings.clone(),
                input_gain.clone(),
                input_level.clone(),
                monitor_enabled.clone(),
                monitor_gain.clone(),
                monitor_sample_rate.clone(),
                monitor_buffer.clone(),
                monitor_output.clone(),
                preferred_input_id.as_deref(),
            )
            .or_else(|preferred_error| {
                let should_fallback = preferred_input_id
                    .as_deref()
                    .is_none_or(|id| !id.starts_with(PULSE_INPUT_PREFIX));

                if should_fallback {
                    start_input(
                        shared.clone(),
                        settings.clone(),
                        input_gain.clone(),
                        input_level.clone(),
                        monitor_enabled.clone(),
                        monitor_gain.clone(),
                        monitor_sample_rate.clone(),
                        monitor_buffer.clone(),
                        monitor_output.clone(),
                        None,
                    )
                    .map_err(|fallback_error| {
                        format!(
                            "Preferred input failed: {preferred_error}. Fallback failed: {fallback_error}"
                        )
                    })
                } else {
                    Err(preferred_error)
                }
            });

            match input {
                Ok(StartedInput::Cpal {
                    stream,
                    analysis_worker: worker,
                    selected_id,
                    sample_rate,
                }) => {
                    monitor_sample_rate.store(sample_rate, Ordering::Relaxed);
                    if let Ok(mut current) = selected_input_id.lock() {
                        *current = Some(selected_id);
                    }
                    if let Ok(mut stream_guard) = streams.lock() {
                        stream_guard.push(ManuallyDrop::new(stream));
                    }
                    if let Ok(mut guard) = analysis_worker.lock() {
                        *guard = Some(worker);
                    }
                }
                Ok(StartedInput::Pulse {
                    capture,
                    analysis_worker: worker,
                    selected_id,
                    sample_rate,
                }) => {
                    monitor_sample_rate.store(sample_rate, Ordering::Relaxed);
                    if let Ok(mut current) = selected_input_id.lock() {
                        *current = Some(selected_id);
                    }
                    if let Ok(mut guard) = pulse_capture.lock() {
                        *guard = Some(capture);
                    }
                    if let Ok(mut guard) = analysis_worker.lock() {
                        *guard = Some(worker);
                    }
                }
                Err(message) => update_error(&shared, &message),
            }

            Self {
                shared,
                settings,
                input_gain,
                input_level,
                monitor_enabled,
                monitor_gain,
                monitor_sample_rate,
                monitor_buffer,
                selected_input_id,
                streams,
                pulse_capture,
                analysis_worker,
                monitor_output,
            }
        }

        pub fn status(&self) -> AudioStatus {
            self.shared
                .lock()
                .map(|guard| guard.status.clone())
                .unwrap_or_else(|_| AudioStatus::Error("Audio state lock poisoned".to_owned()))
        }

        pub fn reading(&self) -> Option<TunerReading> {
            self.shared.lock().ok().and_then(|guard| guard.reading.clone())
        }

        pub fn analysis_settings(&self) -> AnalysisSettings {
            self.settings
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
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
            self.input_gain
                .store(gain.clamp(1.0, 12.0).to_bits(), Ordering::Relaxed);
        }

        pub fn input_level(&self) -> f32 {
            f32::from_bits(self.input_level.load(Ordering::Relaxed))
        }

        pub fn input_waveform(&self) -> Vec<f32> {
            self.shared
                .lock()
                .map(|guard| guard.input_waveform.iter().copied().collect())
                .unwrap_or_default()
        }

        pub fn monitor_enabled(&self) -> bool {
            self.monitor_enabled.load(Ordering::Relaxed)
        }

        pub fn set_monitor_enabled(&self, enabled: bool) {
            self.monitor_enabled.store(enabled, Ordering::Relaxed);
            let selected_input_id = self.selected_input_id();
            refresh_monitor_playback(
                selected_input_id.as_deref(),
                &self.monitor_enabled,
                &self.monitor_sample_rate,
                &self.monitor_buffer,
                &self.monitor_output,
            );
        }

        pub fn monitor_gain(&self) -> f32 {
            f32::from_bits(self.monitor_gain.load(Ordering::Relaxed))
        }

        pub fn set_monitor_gain(&self, gain: f32) {
            self.monitor_gain
                .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }

        pub fn current_input_sample_rate(&self) -> u32 {
            self.monitor_sample_rate.load(Ordering::Relaxed)
        }

        pub fn monitor_output_sample_rate(&self) -> Option<u32> {
            self.monitor_output
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(|output| output.sample_rate))
        }

        pub fn default_output_device_name(&self) -> Option<String> {
            cpal::default_host()
                .default_output_device()
                .and_then(|device| device.name().ok())
        }

        pub fn available_inputs(&self) -> Vec<AudioInputOption> {
            enumerate_input_options()
        }

        pub fn selected_input_id(&self) -> Option<String> {
            self.selected_input_id.lock().ok().and_then(|guard| guard.clone())
        }

        pub fn set_selected_input_id(&self, input_id: Option<String>) {
            let current = self.selected_input_id();
            if current == input_id {
                return;
            }

            match start_input(
                self.shared.clone(),
                self.settings.clone(),
                self.input_gain.clone(),
                self.input_level.clone(),
                self.monitor_enabled.clone(),
                self.monitor_gain.clone(),
                self.monitor_sample_rate.clone(),
                self.monitor_buffer.clone(),
                self.monitor_output.clone(),
                input_id.as_deref(),
            ) {
                Ok(started_input) => {
                    if let Ok(mut state) = self.shared.lock() {
                        state.reading = None;
                        state.input_waveform.clear();
                        state.waterfall.clear();
                        state.note_waterfall.clear();
                        state.spiral_waterfall.clear();
                        state.resonator_waterfall.clear();
                        state.smoothed_frequency = None;
                        state.status = AudioStatus::Listening;
                    }
                    if let Ok(mut buffer) = self.monitor_buffer.lock() {
                        buffer.clear();
                    }

                    if let Ok(streams) = self.streams.lock() {
                        if let Some(active) = streams.last() {
                            let _ = active.pause();
                        }
                    }
                    if let Ok(mut guard) = self.pulse_capture.lock()
                        && let Some(capture) = guard.take()
                    {
                        capture.stop();
                    }
                    if let Ok(mut guard) = self.analysis_worker.lock()
                        && let Some(worker) = guard.take()
                    {
                        worker.stop();
                    }

                    let resolved_id = match started_input {
                        StartedInput::Cpal {
                            stream,
                            analysis_worker: worker,
                            selected_id,
                            sample_rate,
                        } => {
                            self.monitor_sample_rate.store(sample_rate, Ordering::Relaxed);
                            if let Ok(mut streams) = self.streams.lock() {
                                // CPAL 0.15's ALSA backend can panic while dropping an input stream on shutdown.
                                // Keep old streams paused and alive instead of dropping through the buggy path.
                                streams.push(ManuallyDrop::new(stream));
                            }
                            if let Ok(mut guard) = self.analysis_worker.lock() {
                                *guard = Some(worker);
                            }
                            selected_id
                        }
                        StartedInput::Pulse {
                            capture,
                            analysis_worker: worker,
                            selected_id,
                            sample_rate,
                        } => {
                            self.monitor_sample_rate.store(sample_rate, Ordering::Relaxed);
                            if let Ok(mut guard) = self.pulse_capture.lock() {
                                *guard = Some(capture);
                            }
                            if let Ok(mut guard) = self.analysis_worker.lock() {
                                *guard = Some(worker);
                            }
                            selected_id
                        }
                    };

                    if let Ok(mut selected) = self.selected_input_id.lock() {
                        *selected = Some(resolved_id);
                    }

                    let selected_input_id = self.selected_input_id();
                    refresh_monitor_playback(
                        selected_input_id.as_deref(),
                        &self.monitor_enabled,
                        &self.monitor_sample_rate,
                        &self.monitor_buffer,
                        &self.monitor_output,
                    );
                }
                Err(message) => update_error(&self.shared, &message),
            }
        }
    }

    impl Drop for AudioEngine {
        fn drop(&mut self) {
            if let Ok(mut guard) = self.pulse_capture.lock()
                && let Some(capture) = guard.take()
            {
                capture.stop();
            }
            if let Ok(mut guard) = self.analysis_worker.lock()
                && let Some(worker) = guard.take()
            {
                worker.stop();
            }

            if let Ok(mut guard) = self.monitor_output.lock()
                && let Some(playback) = guard.take()
            {
                playback.stop();
            }
        }
    }

    fn start_input(
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
        monitor_enabled: Arc<AtomicBool>,
        monitor_gain: Arc<AtomicU32>,
        monitor_sample_rate: Arc<AtomicU32>,
        monitor_buffer: Arc<Mutex<MonitorBuffer>>,
        monitor_output: Arc<Mutex<Option<MonitorOutput>>>,
        requested_input_id: Option<&str>,
    ) -> Result<StartedInput, String> {
        if let Some(source_name) = requested_input_id.and_then(strip_pulse_input_id) {
            let (analysis_sender, analysis_worker) = start_analysis_worker(
                PULSE_SAMPLE_RATE as f32,
                shared.clone(),
                settings.clone(),
                input_gain.clone(),
                input_level.clone(),
            );
            let capture = start_pulse_capture(
                shared,
                monitor_enabled,
                monitor_gain,
                monitor_buffer,
                analysis_sender,
                source_name,
            )?;
            return Ok(StartedInput::Pulse {
                capture,
                analysis_worker,
                selected_id: format!("{PULSE_INPUT_PREFIX}{source_name}"),
                sample_rate: PULSE_SAMPLE_RATE,
            });
        }

        let requested_cpal_id = requested_input_id.and_then(strip_cpal_input_id);
        let host = cpal::default_host();
        let device = select_input_device(&host, requested_cpal_id)?;
        let selected_input_id = device
            .name()
            .map(|name| format!("{CPAL_INPUT_PREFIX}{name}"))
            .unwrap_or_else(|_| format!("{CPAL_INPUT_PREFIX}Unknown input"));

        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(error) => {
                return Err(format!("Input config error: {error}"));
            }
        };

        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Listening;
        }

        let sample_rate = config.sample_rate().0 as f32;
        let channels = usize::from(config.channels());
        let stream_config: cpal::StreamConfig = config.clone().into();
        let (analysis_sender, analysis_worker) = start_analysis_worker(
            sample_rate,
            shared.clone(),
            settings.clone(),
            input_gain.clone(),
            input_level.clone(),
        );

        let err_state = shared.clone();
        let err_fn = move |error| update_error(&err_state, &format!("Audio stream error: {error}"));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    monitor_enabled.clone(),
                    monitor_gain.clone(),
                    monitor_buffer.clone(),
                    analysis_sender.clone(),
                    err_fn,
                )
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    monitor_enabled.clone(),
                    monitor_gain.clone(),
                    monitor_buffer.clone(),
                    analysis_sender.clone(),
                    err_fn,
                )
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    monitor_enabled.clone(),
                    monitor_gain.clone(),
                    monitor_buffer.clone(),
                    analysis_sender,
                    err_fn,
                )
            }
            sample_format => {
                return Err(format!("Unsupported sample format: {sample_format:?}"));
            }
        };

        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => return Err(format!("Failed to build input stream: {error}")),
        };

        if stream.play().is_err() {
            return Err("Failed to start input stream".to_owned());
        }

        refresh_monitor_playback(
            Some(selected_input_id.as_str()),
            &monitor_enabled,
            &monitor_sample_rate,
            &monitor_buffer,
            &monitor_output,
        );

        Ok(StartedInput::Cpal {
            stream,
            analysis_worker,
            selected_id: selected_input_id,
            sample_rate: sample_rate.round() as u32,
        })
    }

    fn start_analysis_worker(
        sample_rate: f32,
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
    ) -> (Arc<AnalysisMailbox>, AnalysisWorker) {
        let mailbox = Arc::new(AnalysisMailbox::new());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let worker_mailbox = mailbox.clone();

        let thread = thread::spawn(move || {
            let mut pipeline = AnalysisPipeline::new(sample_rate);
            while !stop_flag.load(Ordering::Relaxed) {
                let mut latest = match worker_mailbox.latest.lock() {
                    Ok(guard) => guard,
                    Err(_) => break,
                };

                while latest.is_none() && !stop_flag.load(Ordering::Relaxed) {
                    match worker_mailbox
                        .wake
                        .wait_timeout(latest, Duration::from_millis(25))
                    {
                        Ok((guard, _)) => latest = guard,
                        Err(_) => return,
                    }
                }

                if let Some(samples) = latest.take() {
                    drop(latest);
                    pipeline.push_samples(samples, &shared, &settings, &input_gain, &input_level);
                }
            }
        });

        (mailbox, AnalysisWorker { stop, thread })
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        sample_rate: f32,
        monitor_enabled: Arc<AtomicBool>,
        monitor_gain: Arc<AtomicU32>,
        monitor_buffer: Arc<Mutex<MonitorBuffer>>,
        analysis_mailbox: Arc<AnalysisMailbox>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        device.build_input_stream(
            config,
            move |data: &[T], _| {
                let _ = sample_rate;
                let samples: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| f32::from_sample(frame[0]))
                    .collect();

                if monitor_enabled.load(Ordering::Relaxed) {
                    push_monitor_samples(&samples, &monitor_gain, &monitor_buffer);
                }

                push_analysis_samples(&analysis_mailbox, samples);
            },
            err_fn,
            None,
        )
    }

    fn start_pulse_capture(
        shared: Arc<Mutex<SharedState>>,
        monitor_enabled: Arc<AtomicBool>,
        monitor_gain: Arc<AtomicU32>,
        monitor_buffer: Arc<Mutex<MonitorBuffer>>,
        analysis_mailbox: Arc<AnalysisMailbox>,
        source_name: &str,
    ) -> Result<PulseCapture, String> {
        let mut command = Command::new("parec");
        command
            .arg("--device")
            .arg(source_name)
            .arg("--format=float32le")
            .arg(format!("--rate={PULSE_SAMPLE_RATE}"))
            .arg("--channels=1")
            .arg("--raw")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|error| format!("Failed to start system audio capture: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "System audio capture has no stdout pipe".to_owned())?;

        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Listening;
        }

        let thread = thread::spawn(move || {
            let mut reader = stdout;
            let mut byte_buffer = [0_u8; 4096];
            let mut remainder = Vec::new();

            loop {
                match reader.read(&mut byte_buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        remainder.extend_from_slice(&byte_buffer[..bytes_read]);
                        let complete_len = remainder.len() - (remainder.len() % 4);
                        if complete_len == 0 {
                            continue;
                        }

                        if monitor_enabled.load(Ordering::Relaxed) {
                            push_monitor_bytes(&remainder[..complete_len], &monitor_gain, &monitor_buffer);
                        }

                        let samples: Vec<f32> = remainder[..complete_len]
                            .chunks_exact(4)
                            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                            .collect();
                        push_analysis_samples(&analysis_mailbox, samples);
                        remainder.drain(..complete_len);
                    }
                    Err(error) => {
                        update_error(&shared, &format!("System audio capture error: {error}"));
                        break;
                    }
                }
            }
        });

        Ok(PulseCapture { child, thread })
    }

    fn push_analysis_samples(mailbox: &Arc<AnalysisMailbox>, samples: Vec<f32>) {
        if let Ok(mut latest) = mailbox.latest.lock() {
            *latest = Some(samples);
            mailbox.wake.notify_one();
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

    fn refresh_monitor_playback(
        selected_input_id: Option<&str>,
        monitor_enabled: &Arc<AtomicBool>,
        monitor_sample_rate: &Arc<AtomicU32>,
        monitor_buffer: &Arc<Mutex<MonitorBuffer>>,
        monitor_output: &Arc<Mutex<Option<MonitorOutput>>>,
    ) {
        let should_run = monitor_enabled.load(Ordering::Relaxed)
            && selected_input_id.is_some_and(monitor_supported_for_input);
        let desired_sample_rate = monitor_sample_rate.load(Ordering::Relaxed);

        if should_run {
            if let Ok(mut output) = monitor_output.lock() {
                let needs_restart = output
                    .as_ref()
                    .is_none_or(|active| active.sample_rate != desired_sample_rate);
                if needs_restart {
                    if let Some(active) = output.take() {
                        active.stop();
                    }
                    if let Ok(mut buffer) = monitor_buffer.lock() {
                        buffer.clear();
                    }
                    if let Ok(started) = start_monitor_output(
                        desired_sample_rate,
                        monitor_buffer.clone(),
                        monitor_output.clone(),
                    ) {
                        *output = Some(started);
                    }
                }
            }
        } else if let Ok(mut output) = monitor_output.lock()
            && let Some(active) = output.take()
        {
            active.stop();
        }
    }

    fn start_monitor_output(
        desired_sample_rate: u32,
        monitor_buffer: Arc<Mutex<MonitorBuffer>>,
        _monitor_output: Arc<Mutex<Option<MonitorOutput>>>,
    ) -> Result<MonitorOutput, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device found for monitor playback".to_owned())?;
        let (config, sample_format, actual_sample_rate) =
            select_monitor_output_config(&device, desired_sample_rate)?;

        let channels = usize::from(config.channels);
        let err_fn = move |_error| {};

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                build_output_stream::<f32>(
                    &device,
                    &config,
                    channels,
                    desired_sample_rate,
                    actual_sample_rate,
                    monitor_buffer.clone(),
                    err_fn,
                )
                .map_err(|error| format!("Failed to build monitor output stream: {error}"))?
            }
            cpal::SampleFormat::I16 => {
                build_output_stream::<i16>(
                    &device,
                    &config,
                    channels,
                    desired_sample_rate,
                    actual_sample_rate,
                    monitor_buffer.clone(),
                    err_fn,
                )
                .map_err(|error| format!("Failed to build monitor output stream: {error}"))?
            }
            cpal::SampleFormat::U16 => {
                build_output_stream::<u16>(
                    &device,
                    &config,
                    channels,
                    desired_sample_rate,
                    actual_sample_rate,
                    monitor_buffer.clone(),
                    err_fn,
                )
                .map_err(|error| format!("Failed to build monitor output stream: {error}"))?
            }
            other => return Err(format!("Unsupported monitor output format: {other:?}")),
        };

        stream
            .play()
            .map_err(|error| format!("Failed to start monitor output stream: {error}"))?;

        Ok(MonitorOutput {
            stream,
            sample_rate: actual_sample_rate,
        })
    }

    fn push_monitor_bytes(
        input_bytes: &[u8],
        monitor_gain: &Arc<AtomicU32>,
        monitor_buffer: &Arc<Mutex<MonitorBuffer>>,
    ) {
        let gain = f32::from_bits(monitor_gain.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mut samples = Vec::with_capacity(input_bytes.len() / 4);

        for chunk in input_bytes.chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            samples.push((sample * gain).clamp(-1.0, 1.0));
        }
        append_monitor_buffer(monitor_buffer, &samples);
    }

    fn push_monitor_samples(
        samples: &[f32],
        monitor_gain: &Arc<AtomicU32>,
        monitor_buffer: &Arc<Mutex<MonitorBuffer>>,
    ) {
        let gain = f32::from_bits(monitor_gain.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let scaled: Vec<f32> = samples
            .iter()
            .map(|sample| (sample * gain).clamp(-1.0, 1.0))
            .collect();
        append_monitor_buffer(monitor_buffer, &scaled);
    }

    fn append_monitor_buffer(monitor_buffer: &Arc<Mutex<MonitorBuffer>>, samples: &[f32]) {
        if let Ok(mut buffer) = monitor_buffer.lock() {
            buffer.samples.extend(samples.iter().copied());
            while buffer.samples.len() > MONITOR_BUFFER_MAX_FRAMES {
                buffer.samples.pop_front();
            }
        }
    }

    fn select_monitor_output_config(
        device: &cpal::Device,
        desired_sample_rate: u32,
    ) -> Result<(cpal::StreamConfig, cpal::SampleFormat, u32), String> {
        let preferred_formats = [
            cpal::SampleFormat::F32,
            cpal::SampleFormat::I16,
            cpal::SampleFormat::U16,
        ];

        if let Ok(configs) = device.supported_output_configs() {
            let configs: Vec<_> = configs.collect();
            for preferred_format in preferred_formats {
                if let Some(config) = configs.iter().find(|config| {
                    config.sample_format() == preferred_format
                        && config.min_sample_rate().0 <= desired_sample_rate
                        && config.max_sample_rate().0 >= desired_sample_rate
                }) {
                    return Ok((
                        config
                            .with_sample_rate(cpal::SampleRate(desired_sample_rate))
                            .config(),
                        config.sample_format(),
                        desired_sample_rate,
                    ));
                }
            }
        }

        let default = device
            .default_output_config()
            .map_err(|error| format!("Monitor output config error: {error}"))?;
        let actual_sample_rate = default.sample_rate().0;
        Ok((default.config(), default.sample_format(), actual_sample_rate))
    }

    fn build_output_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        input_sample_rate: u32,
        output_sample_rate: u32,
        monitor_buffer: Arc<Mutex<MonitorBuffer>>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
    {
        device.build_output_stream(
            config,
            move |data: &mut [T], _| {
                fill_output_buffer(
                    data,
                    channels,
                    input_sample_rate,
                    output_sample_rate,
                    &monitor_buffer,
                );
            },
            err_fn,
            None,
        )
    }

    fn fill_output_buffer<T>(
        data: &mut [T],
        channels: usize,
        input_sample_rate: u32,
        output_sample_rate: u32,
        monitor_buffer: &Arc<Mutex<MonitorBuffer>>,
    ) where
        T: cpal::Sample + cpal::FromSample<f32>,
    {
        if let Ok(mut buffer) = monitor_buffer.lock() {
            let step = if input_sample_rate == 0 || output_sample_rate == 0 {
                1.0
            } else {
                input_sample_rate as f32 / output_sample_rate as f32
            };
            for frame in data.chunks_mut(channels) {
                while buffer.resample_phase >= 1.0 && buffer.samples.len() > 1 {
                    buffer.samples.pop_front();
                    buffer.resample_phase -= 1.0;
                }

                let sample = match buffer.samples.len() {
                    0 => 0.0,
                    1 => buffer.samples[0],
                    _ => {
                        let a = buffer.samples[0];
                        let b = buffer.samples[1];
                        a + (b - a) * buffer.resample_phase.clamp(0.0, 1.0)
                    }
                };

                buffer.resample_phase += step;
                for channel in frame {
                    *channel = T::from_sample(sample);
                }
            }
        } else {
            for sample in data.iter_mut() {
                *sample = T::from_sample(0.0);
            }
        }
    }

    fn monitor_supported_for_input(input_id: &str) -> bool {
        !input_id.ends_with(PULSE_DEFAULT_MONITOR) && !input_id.ends_with(".monitor")
    }

    fn normalized_level(window: &[f32]) -> f32 {
        let rms = (window.iter().map(|sample| sample * sample).sum::<f32>() / window.len() as f32).sqrt();
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
        let rms = (window.iter().map(|sample| sample * sample).sum::<f32>() / window.len() as f32).sqrt();
        let mut normalized = window.to_vec();
        normalized = apply_hann_window(&normalized);

        let (spectrum, note_spectrum, spiral_spectrum) =
            spectrum_bars(&normalized, sample_rate, settings, planner);
        let pitch = if rms < SILENCE_RMS_THRESHOLD {
            None
        } else {
            detect_pitch_yin(&normalized, sample_rate).and_then(|(frequency_hz, clarity)| {
                (45.0..=1200.0).contains(&frequency_hz).then_some(PitchEstimate {
                    frequency_hz,
                    clarity,
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
            .map(|(index, sample)| {
                let phase = (2.0 * std::f32::consts::PI * index as f32) / (len - 1.0);
                let multiplier = 0.5 * (1.0 - phase.cos());
                sample * multiplier
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
            for index in 0..limit {
                let delta = window[index] - window[index + tau];
                sum += delta * delta;
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
                .min_by(|left, right| cumulative[*left].total_cmp(&cumulative[*right]))
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

        let magnitudes: Vec<f32> = input
            .iter()
            .take(input.len() / 2)
            .map(|value| value.norm_sqr())
            .collect();

        let hz_per_bin = sample_rate / input.len() as f32;
        let mut bars: Vec<f32> = vec![0.0; SPECTRUM_BINS];
        let mut note_bars: Vec<f32> = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];
        let mut spiral_bars: Vec<f32> = vec![0.0; SPIRAL_BIN_COUNT];

        for (index, magnitude) in magnitudes.iter().enumerate() {
            let frequency = index as f32 * hz_per_bin;
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
        fn from(settings: &AnalysisSettings) -> Self {
            Self {
                min_midi:          settings.resonator_min_midi,
                max_midi:          settings.resonator_max_midi,
                bins_per_semitone: settings.resonator_bins,
                alpha_scale:       settings.resonator_alpha,
                beta_scale:        settings.resonator_beta,
                gamma:             settings.resonator_gamma,
            }
        }
    }

    fn build_resonator_bank(sample_rate: f32, settings: &ResonatorViewSettings) -> ResonatorBank {
        let bin_count = (settings.max_midi - settings.min_midi) * settings.bins_per_semitone + 1;
        let configs: Vec<ResonatorConfig> = (0..bin_count)
            .map(|index| {
                let midi = settings.min_midi as f32 + index as f32 / settings.bins_per_semitone as f32;
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
        let denominator = left - 2.0 * center + right;
        if denominator.abs() < f32::EPSILON {
            tau as f32
        } else {
            tau as f32 + 0.5 * (left - right) / denominator
        }
    }

    fn smooth_frequency(previous: Option<f32>, next: f32) -> f32 {
        match previous {
            Some(previous) => {
                let corrected = correct_octave_jump(previous, next);
                let ratio = (corrected / previous).max(previous / corrected);
                let alpha = if ratio > 1.04 { 0.18 } else { 0.10 };
                previous + (corrected - previous) * alpha
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
        let max_value = values.iter().copied().fold(0.0, f32::max);
        if max_value > 0.0 {
            for value in values {
                *value = (*value / max_value).clamp(0.0, 1.0).powf(gamma);
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
            for index in 0..values.len() {
                let left = scratch[index.saturating_sub(1)];
                let center = scratch[index];
                let right = scratch[(index + 1).min(scratch.len() - 1)];
                values[index] = left * 0.2 + center * 0.6 + right * 0.2;
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
            .map(|midi| midi_to_note_label(midi as i32))
            .collect()
    }

    fn resonator_note_labels(min_midi: usize, max_midi: usize) -> Vec<String> {
        (min_midi..=max_midi)
            .map(|midi| midi_to_note_label(midi as i32))
            .collect()
    }

    fn midi_to_note_label(midi: i32) -> String {
        const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let note_index = midi.rem_euclid(12) as usize;
        let octave = midi / 12 - 1;
        format!("{}{}", NOTE_NAMES[note_index], octave)
    }

    fn publish_reading(shared: &Arc<Mutex<SharedState>>, frame: AnalysisFrame) {
        if let Ok(mut state) = shared.lock() {
            let (smoothed_frequency, clarity) = match frame.pitch {
                Some(pitch) => {
                    let smoothed_frequency = smooth_frequency(state.smoothed_frequency, pitch.frequency_hz);
                    state.smoothed_frequency = Some(smoothed_frequency);
                    (smoothed_frequency, pitch.clarity)
                }
                None => {
                    let Some(smoothed_frequency) = state.smoothed_frequency else {
                        return;
                    };
                    (smoothed_frequency, 0.0)
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

    fn update_error(shared: &Arc<Mutex<SharedState>>, message: &str) {
        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Error(message.to_owned());
        }
    }

    fn enumerate_input_options() -> Vec<AudioInputOption> {
        let mut options = enumerate_cpal_input_options();
        options.extend(enumerate_pulse_input_options());
        let preferred_mic_id = format!("{PULSE_INPUT_PREFIX}{PULSE_DEFAULT_SOURCE}");
        if options.iter().any(|option| option.id == preferred_mic_id) {
            options
                .retain(|option| option.kind != AudioInputKind::Microphone || option.id == preferred_mic_id);
        }
        options.sort_by_key(|option| {
            match option.kind {
                AudioInputKind::Microphone => 0,
                AudioInputKind::System => 1,
                AudioInputKind::Other => 2,
            }
        });
        options
    }

    fn preferred_initial_input_id() -> Option<String> {
        let pulse_options = enumerate_pulse_input_options();
        pulse_options
            .iter()
            .find(|option| option.id == format!("{PULSE_INPUT_PREFIX}{PULSE_DEFAULT_SOURCE}"))
            .or_else(|| {
                pulse_options
                    .iter()
                    .find(|option| option.kind == AudioInputKind::Microphone)
            })
            .map(|option| option.id.clone())
    }

    fn enumerate_cpal_input_options() -> Vec<AudioInputOption> {
        let host = cpal::default_host();
        let default_name = host.default_input_device().and_then(|device| device.name().ok());
        let mut entries = Vec::new();

        let Ok(devices) = host.input_devices() else {
            return Vec::new();
        };

        for device in devices {
            let Ok(name) = device.name() else {
                continue;
            };

            let kind = classify_input_kind(&name, default_name.as_deref());
            let is_default = default_name.as_deref() == Some(name.as_str());
            entries.push((name, kind, is_default));
        }

        if !entries
            .iter()
            .any(|(_, kind, _)| *kind == AudioInputKind::Microphone)
            && let Some(index) = entries
                .iter()
                .position(|(_, kind, is_default)| *kind != AudioInputKind::System && *is_default)
                .or_else(|| {
                    entries
                        .iter()
                        .position(|(_, kind, _)| *kind != AudioInputKind::System)
                })
        {
            entries[index].1 = AudioInputKind::Microphone;
        }

        entries
            .into_iter()
            .map(|(name, kind, is_default)| {
                AudioInputOption {
                    id: format!("{CPAL_INPUT_PREFIX}{name}"),
                    label: format_input_label(&name, kind, is_default),
                    kind,
                }
            })
            .collect()
    }

    fn enumerate_pulse_input_options() -> Vec<AudioInputOption> {
        let output = Command::new("pactl").args(["list", "short", "sources"]).output();
        let Ok(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut monitor_names: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.split('\t').nth(1))
            .filter(|name| name.ends_with(".monitor"))
            .map(str::to_owned)
            .collect();
        let mut source_names: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.split('\t').nth(1))
            .filter(|name| !name.ends_with(".monitor"))
            .map(str::to_owned)
            .collect();

        if monitor_names.is_empty() && source_names.is_empty() {
            return Vec::new();
        }

        monitor_names.sort();
        monitor_names.dedup();
        source_names.sort();
        source_names.dedup();

        let mut options = Vec::new();

        if !source_names.is_empty() {
            options.push(AudioInputOption {
                id:    format!("{PULSE_INPUT_PREFIX}{PULSE_DEFAULT_SOURCE}"),
                label: "Mic • Pulse default source (Recommended)".to_owned(),
                kind:  AudioInputKind::Microphone,
            });
            options.extend(source_names.into_iter().map(|name| {
                AudioInputOption {
                    id:    format!("{PULSE_INPUT_PREFIX}{name}"),
                    label: format!("Mic • {name}"),
                    kind:  AudioInputKind::Microphone,
                }
            }));
        }

        if !monitor_names.is_empty() {
            options.push(AudioInputOption {
                id:    format!("{PULSE_INPUT_PREFIX}{PULSE_DEFAULT_MONITOR}"),
                label: "System • Default monitor".to_owned(),
                kind:  AudioInputKind::System,
            });

            options.extend(monitor_names.into_iter().map(|name| {
                AudioInputOption {
                    id:    format!("{PULSE_INPUT_PREFIX}{name}"),
                    label: format!("System • {name}"),
                    kind:  AudioInputKind::System,
                }
            }));
        }

        options
    }

    fn select_input_device(
        host: &cpal::Host,
        requested_input_id: Option<&str>,
    ) -> Result<cpal::Device, String> {
        if let Some(requested_input_id) = requested_input_id {
            let devices = host
                .input_devices()
                .map_err(|error| format!("Failed to enumerate input devices: {error}"))?;
            for device in devices {
                let Ok(name) = device.name() else {
                    continue;
                };
                if name == requested_input_id {
                    return Ok(device);
                }
            }

            return Err(format!("Input device not found: {requested_input_id}"));
        }

        host.default_input_device()
            .ok_or_else(|| "No input device found".to_owned())
    }

    fn strip_cpal_input_id(input_id: &str) -> Option<&str> {
        input_id
            .strip_prefix(CPAL_INPUT_PREFIX)
            .or(Some(input_id).filter(|id| !id.starts_with(PULSE_INPUT_PREFIX)))
    }

    fn strip_pulse_input_id(input_id: &str) -> Option<&str> {
        input_id.strip_prefix(PULSE_INPUT_PREFIX)
    }

    fn classify_input_kind(name: &str, default_name: Option<&str>) -> AudioInputKind {
        let lowered = name.to_lowercase();
        if [
            "monitor",
            "loopback",
            "stereo mix",
            "what u hear",
            "blackhole",
            "soundflower",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
        {
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
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(index, _)| index)
                .unwrap();

            assert_eq!(strongest, a4_index);
            assert!(bars[a4_index] > bars[a4_index - 1]);
            assert!(bars[a4_index] > bars[a4_index + 1]);
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
