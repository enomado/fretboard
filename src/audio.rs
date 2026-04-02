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
    const WATERFALL_HISTORY: usize = 36;
    const NOTE_BUCKET_MIN_MIDI: usize = 36;
    const NOTE_BUCKET_MAX_MIDI: usize = 84;
    const ANALYSIS_INTERVAL: Duration = Duration::from_millis(70);
    const YIN_THRESHOLD: f32 = 0.12;

    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz:   f32,
        pub note_name:      String,
        pub cents:          f32,
        pub clarity:        f32,
        pub spectrum:       Vec<f32>,
        pub waterfall:      Vec<Vec<f32>>,
        pub note_spectrum:  Vec<f32>,
        pub note_waterfall: Vec<Vec<f32>>,
        pub note_labels:    Vec<String>,
    }

    #[derive(Clone, Debug)]
    pub enum AudioStatus {
        Idle,
        Listening,
        Error(String),
    }

    struct SharedState {
        status:             AudioStatus,
        reading:            Option<TunerReading>,
        waterfall:          VecDeque<Vec<f32>>,
        note_waterfall:     VecDeque<Vec<f32>>,
        smoothed_frequency: Option<f32>,
    }

    pub struct AudioEngine {
        shared:  Arc<Mutex<SharedState>>,
        _stream: Option<cpal::Stream>,
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let shared = Arc::new(Mutex::new(SharedState {
                status:             AudioStatus::Idle,
                reading:            None,
                waterfall:          VecDeque::with_capacity(WATERFALL_HISTORY),
                note_waterfall:     VecDeque::with_capacity(WATERFALL_HISTORY),
                smoothed_frequency: None,
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
                        let smoothed_frequency =
                            smooth_frequency(state.smoothed_frequency, reading.frequency_hz);
                        state.smoothed_frequency = Some(smoothed_frequency);

                        let (note_name, cents) = frequency_to_note(smoothed_frequency);
                        state.waterfall.push_back(reading.spectrum.clone());
                        state.note_waterfall.push_back(reading.note_spectrum.clone());
                        while state.waterfall.len() > WATERFALL_HISTORY {
                            state.waterfall.pop_front();
                        }
                        while state.note_waterfall.len() > WATERFALL_HISTORY {
                            state.note_waterfall.pop_front();
                        }

                        state.reading = Some(TunerReading {
                            frequency_hz: smoothed_frequency,
                            note_name,
                            cents,
                            clarity: reading.clarity,
                            spectrum: reading.spectrum,
                            waterfall: state.waterfall.iter().cloned().collect(),
                            note_spectrum: reading.note_spectrum,
                            note_waterfall: state.note_waterfall.iter().cloned().collect(),
                            note_labels: note_bucket_labels(),
                        });
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

        let (frequency_hz, clarity) = detect_pitch_yin(&normalized, sample_rate)?;
        if !(45.0..=1200.0).contains(&frequency_hz) {
            return None;
        }

        let (note_name, cents) = frequency_to_note(frequency_hz);
        let (spectrum, note_spectrum) = spectrum_bars(&normalized, sample_rate, planner);

        Some(TunerReading {
            frequency_hz,
            note_name,
            cents,
            clarity,
            spectrum,
            waterfall: Vec::new(),
            note_spectrum,
            note_waterfall: Vec::new(),
            note_labels: note_bucket_labels(),
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
        if tau <= 0.0 {
            return None;
        }

        let clarity = (1.0 - cumulative[tau.round() as usize].clamp(0.0, 1.0)).clamp(0.0, 1.0);
        if clarity < 0.35 {
            return None;
        }

        Some((sample_rate / tau, clarity))
    }

    fn spectrum_bars(
        window: &[f32],
        sample_rate: f32,
        planner: &mut FftPlanner<f32>,
    ) -> (Vec<f32>, Vec<f32>) {
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
        let mut note_bars: Vec<f32> = vec![0.0; NOTE_BUCKET_MAX_MIDI - NOTE_BUCKET_MIN_MIDI + 1];

        for (index, magnitude) in magnitudes.iter().enumerate() {
            let frequency = index as f32 * hz_per_bin;
            if !(20.0..=max_frequency).contains(&frequency) {
                continue;
            }

            let normalized = ((frequency / max_frequency) * SPECTRUM_BINS as f32).floor() as usize;
            let bucket = normalized.min(SPECTRUM_BINS - 1);
            bars[bucket] = bars[bucket].max(*magnitude);

            if let Some(note_index) = note_bucket_index(frequency) {
                note_bars[note_index] = note_bars[note_index].max(*magnitude);
            }
        }

        normalize_bars(&mut bars);
        normalize_bars(&mut note_bars);

        (bars, note_bars)
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
                let ratio = (next / previous).max(previous / next);
                let alpha = if ratio > 1.08 { 0.45 } else { 0.22 };
                previous + (next - previous) * alpha
            }
            None => next,
        }
    }

    fn normalize_bars(values: &mut [f32]) {
        let max_value = values.iter().copied().fold(0.0, f32::max);
        if max_value > 0.0 {
            for value in values {
                *value = (*value / max_value).clamp(0.0, 1.0);
            }
        }
    }

    fn note_bucket_index(frequency: f32) -> Option<usize> {
        if frequency <= 0.0 {
            return None;
        }

        let midi = (69.0 + 12.0 * (frequency / 440.0).log2()).round() as isize;
        if midi < NOTE_BUCKET_MIN_MIDI as isize || midi > NOTE_BUCKET_MAX_MIDI as isize {
            return None;
        }

        Some((midi as usize) - NOTE_BUCKET_MIN_MIDI)
    }

    fn note_bucket_labels() -> Vec<String> {
        (NOTE_BUCKET_MIN_MIDI..=NOTE_BUCKET_MAX_MIDI)
            .map(|midi| midi_to_note_label(midi as i32))
            .collect()
    }

    fn midi_to_note_label(midi: i32) -> String {
        const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let note_index = midi.rem_euclid(12) as usize;
        let octave = midi / 12 - 1;
        format!("{}{}", NOTE_NAMES[note_index], octave)
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
        pub frequency_hz:   f32,
        pub note_name:      String,
        pub cents:          f32,
        pub clarity:        f32,
        pub spectrum:       Vec<f32>,
        pub waterfall:      Vec<Vec<f32>>,
        pub note_spectrum:  Vec<f32>,
        pub note_waterfall: Vec<Vec<f32>>,
        pub note_labels:    Vec<String>,
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
