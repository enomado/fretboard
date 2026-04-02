#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::sync::{
        Arc,
        Mutex,
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
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex32;

    const WINDOW_SIZE: usize = 4096;
    const SPECTRUM_BINS: usize = 48;
    const ANALYSIS_INTERVAL: Duration = Duration::from_millis(70);

    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz: f32,
        pub note_name:    String,
        pub cents:        f32,
        pub clarity:      f32,
        pub spectrum:     Vec<f32>,
    }

    #[derive(Clone, Debug)]
    pub enum AudioStatus {
        Idle,
        Listening,
        Error(String),
    }

    struct SharedState {
        status:  AudioStatus,
        reading: Option<TunerReading>,
    }

    pub struct AudioEngine {
        shared:  Arc<Mutex<SharedState>>,
        _stream: Option<cpal::Stream>,
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let shared = Arc::new(Mutex::new(SharedState {
                status:  AudioStatus::Idle,
                reading: None,
            }));

            let stream = start_stream(shared.clone());

            Self {
                shared,
                _stream: stream.ok(),
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
    }

    fn start_stream(shared: Arc<Mutex<SharedState>>) -> Result<cpal::Stream, ()> {
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            update_error(&shared, "No input device found");
            return Err(());
        };

        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(error) => {
                update_error(&shared, &format!("Input config error: {error}"));
                return Err(());
            }
        };

        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Listening;
        }

        let sample_rate = config.sample_rate().0 as f32;
        let channels = usize::from(config.channels());
        let stream_config: cpal::StreamConfig = config.clone().into();

        let err_state = shared.clone();
        let err_fn = move |error| update_error(&err_state, &format!("Audio stream error: {error}"));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    shared.clone(),
                    err_fn,
                )
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    shared.clone(),
                    err_fn,
                )
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(
                    &device,
                    &stream_config,
                    channels,
                    sample_rate,
                    shared.clone(),
                    err_fn,
                )
            }
            sample_format => {
                update_error(&shared, &format!("Unsupported sample format: {sample_format:?}"));
                return Err(());
            }
        };

        let Ok(stream) = stream else {
            return Err(());
        };

        if stream.play().is_err() {
            update_error(&shared, "Failed to start input stream");
            return Err(());
        }

        Ok(stream)
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        sample_rate: f32,
        shared: Arc<Mutex<SharedState>>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let mut buffer: VecDeque<f32> = VecDeque::with_capacity(WINDOW_SIZE * 2);
        let mut last_analysis = Instant::now() - ANALYSIS_INTERVAL;
        let mut planner = FftPlanner::new();

        device.build_input_stream(
            config,
            move |data: &[T], _| {
                for frame in data.chunks(channels) {
                    let sample: f32 = f32::from_sample(frame[0]);
                    buffer.push_back(sample);
                }

                while buffer.len() > WINDOW_SIZE * 2 {
                    buffer.pop_front();
                }

                if buffer.len() < WINDOW_SIZE || last_analysis.elapsed() < ANALYSIS_INTERVAL {
                    return;
                }
                last_analysis = Instant::now();

                let start = buffer.len().saturating_sub(WINDOW_SIZE);
                let window: Vec<f32> = buffer.iter().skip(start).copied().collect();

                if let Some(reading) = analyze_window(&window, sample_rate, &mut planner) {
                    if let Ok(mut state) = shared.lock() {
                        state.reading = Some(reading);
                        state.status = AudioStatus::Listening;
                    }
                }
            },
            err_fn,
            None,
        )
    }

    fn analyze_window(
        window: &[f32],
        sample_rate: f32,
        planner: &mut FftPlanner<f32>,
    ) -> Option<TunerReading> {
        let rms = (window.iter().map(|sample| sample * sample).sum::<f32>() / window.len() as f32).sqrt();
        if rms < 0.01 {
            return None;
        }

        let mut normalized = window.to_vec();
        normalized = apply_hann_window(&normalized);

        let (frequency_hz, clarity) = detect_pitch_autocorrelation(&normalized, sample_rate)?;
        if !(45.0..=1200.0).contains(&frequency_hz) {
            return None;
        }

        let (note_name, cents) = frequency_to_note(frequency_hz);
        let spectrum = spectrum_bars(&normalized, sample_rate, planner);

        Some(TunerReading {
            frequency_hz,
            note_name,
            cents,
            clarity,
            spectrum,
        })
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

    fn detect_pitch_autocorrelation(window: &[f32], sample_rate: f32) -> Option<(f32, f32)> {
        let min_lag = (sample_rate / 1000.0).max(1.0) as usize;
        let max_lag = (sample_rate / 45.0) as usize;
        let search_end = max_lag.min(window.len().saturating_sub(1));
        if min_lag >= search_end {
            return None;
        }

        let mut best_lag = 0usize;
        let mut best_score = 0.0f32;

        for lag in min_lag..=search_end {
            let mut sum = 0.0;
            let limit = window.len().saturating_sub(lag);
            for index in 0..limit {
                sum += window[index] * window[index + lag];
            }

            if sum > best_score {
                best_score = sum;
                best_lag = lag;
            }
        }

        if best_lag == 0 || best_score <= 0.0 {
            return None;
        }

        Some((sample_rate / best_lag as f32, best_score / window.len() as f32))
    }

    fn spectrum_bars(window: &[f32], sample_rate: f32, planner: &mut FftPlanner<f32>) -> Vec<f32> {
        let mut input: Vec<Complex32> = window.iter().map(|sample| Complex32::new(*sample, 0.0)).collect();
        let fft = planner.plan_fft_forward(input.len());
        fft.process(&mut input);

        let magnitudes: Vec<f32> = input
            .iter()
            .take(input.len() / 2)
            .map(|value| value.norm())
            .collect();

        let max_frequency = 2000.0f32;
        let hz_per_bin = sample_rate / window.len() as f32;
        let mut bars: Vec<f32> = vec![0.0; SPECTRUM_BINS];

        for (index, magnitude) in magnitudes.iter().enumerate() {
            let frequency = index as f32 * hz_per_bin;
            if !(20.0..=max_frequency).contains(&frequency) {
                continue;
            }

            let normalized = ((frequency / max_frequency) * SPECTRUM_BINS as f32).floor() as usize;
            let bucket = normalized.min(SPECTRUM_BINS - 1);
            bars[bucket] = bars[bucket].max(*magnitude);
        }

        let max_value = bars.iter().copied().fold(0.0, f32::max);
        if max_value > 0.0 {
            for value in &mut bars {
                *value = (*value / max_value).clamp(0.0, 1.0);
            }
        }

        bars
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

    fn update_error(shared: &Arc<Mutex<SharedState>>, message: &str) {
        if let Ok(mut state) = shared.lock() {
            state.status = AudioStatus::Error(message.to_owned());
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod native {
    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz: f32,
        pub note_name:    String,
        pub cents:        f32,
        pub clarity:      f32,
        pub spectrum:     Vec<f32>,
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
    }
}

pub use native::{
    AudioEngine,
    AudioStatus,
    TunerReading,
};
