#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::mem::ManuallyDrop;
    use std::sync::atomic::{
        AtomicU32,
        Ordering,
    };
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
    const SILENCE_RMS_THRESHOLD: f32 = 0.003;
    const YIN_THRESHOLD: f32 = 0.12;
    const SPECTRUM_MIN_FREQUENCY: f32 = 20.0;
    const SPECTRUM_MAX_FREQUENCY: f32 = 2_000.0;
    const NOTE_BUCKET_SPREAD: f32 = 0.35;

    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz:     f32,
        pub note_name:        String,
        pub cents:            f32,
        pub clarity:          f32,
        pub spectrum:         Vec<f32>,
        pub waterfall:        Vec<Vec<f32>>,
        pub note_spectrum:    Vec<f32>,
        pub note_waterfall:   Vec<Vec<f32>>,
        pub spiral_spectrum:  Vec<f32>,
        pub spiral_waterfall: Vec<Vec<f32>>,
        pub note_labels:      Vec<String>,
    }

    #[derive(Clone, Debug)]
    pub enum AudioStatus {
        Idle,
        Listening,
        Error(String),
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
            self
        }
    }

    struct SharedState {
        status:             AudioStatus,
        reading:            Option<TunerReading>,
        waterfall:          VecDeque<Vec<f32>>,
        note_waterfall:     VecDeque<Vec<f32>>,
        spiral_waterfall:   VecDeque<Vec<f32>>,
        smoothed_frequency: Option<f32>,
    }

    pub struct AudioEngine {
        shared:      Arc<Mutex<SharedState>>,
        settings:    Arc<Mutex<AnalysisSettings>>,
        input_gain:  Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
        _stream:     ManuallyDrop<Option<cpal::Stream>>,
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let shared = Arc::new(Mutex::new(SharedState {
                status:             AudioStatus::Idle,
                reading:            None,
                waterfall:          VecDeque::with_capacity(WATERFALL_HISTORY),
                note_waterfall:     VecDeque::with_capacity(WATERFALL_HISTORY),
                spiral_waterfall:   VecDeque::with_capacity(WATERFALL_HISTORY),
                smoothed_frequency: None,
            }));
            let settings = Arc::new(Mutex::new(AnalysisSettings::default()));
            let input_gain = Arc::new(AtomicU32::new(DEFAULT_INPUT_GAIN.to_bits()));
            let input_level = Arc::new(AtomicU32::new(0.0f32.to_bits()));

            let stream = start_stream(
                shared.clone(),
                settings.clone(),
                input_gain.clone(),
                input_level.clone(),
            );

            Self {
                shared,
                settings,
                input_gain,
                input_level,
                // CPAL 0.15's ALSA backend can panic while dropping an input stream on shutdown.
                // Keep the stream alive for the process lifetime instead of tearing it down through
                // the buggy drop path.
                _stream: ManuallyDrop::new(stream.ok()),
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
    }

    fn start_stream(
        shared: Arc<Mutex<SharedState>>,
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
    ) -> Result<cpal::Stream, ()> {
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
                    settings.clone(),
                    input_gain.clone(),
                    input_level.clone(),
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
                    settings.clone(),
                    input_gain.clone(),
                    input_level.clone(),
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
                    settings.clone(),
                    input_gain.clone(),
                    input_level.clone(),
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
        settings: Arc<Mutex<AnalysisSettings>>,
        input_gain: Arc<AtomicU32>,
        input_level: Arc<AtomicU32>,
        err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let mut buffer: VecDeque<f32> = VecDeque::with_capacity(MAX_WINDOW_SIZE * 2);
        let mut last_analysis = Instant::now() - ANALYSIS_INTERVAL;
        let mut planner = FftPlanner::new();

        device.build_input_stream(
            config,
            move |data: &[T], _| {
                let analysis_settings = settings
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default()
                    .sanitized();
                let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
                for frame in data.chunks(channels) {
                    let sample = (f32::from_sample(frame[0]) * gain).clamp(-1.0, 1.0);
                    buffer.push_back(sample);
                }

                while buffer.len() > MAX_WINDOW_SIZE * 2 {
                    buffer.pop_front();
                }

                if buffer.len() < analysis_settings.window_size || last_analysis.elapsed() < ANALYSIS_INTERVAL
                {
                    return;
                }
                last_analysis = Instant::now();

                let start = buffer.len().saturating_sub(analysis_settings.window_size);
                let window: Vec<f32> = buffer.iter().skip(start).copied().collect();
                let level = normalized_level(&window);
                input_level.store(level.to_bits(), Ordering::Relaxed);

                if let Some(reading) = analyze_window(&window, sample_rate, &analysis_settings, &mut planner)
                {
                    if let Ok(mut state) = shared.lock() {
                        let smoothed_frequency =
                            smooth_frequency(state.smoothed_frequency, reading.frequency_hz);
                        state.smoothed_frequency = Some(smoothed_frequency);

                        let (note_name, cents) = frequency_to_note(smoothed_frequency);
                        state.waterfall.push_back(reading.spectrum.clone());
                        state.note_waterfall.push_back(reading.note_spectrum.clone());
                        state.spiral_waterfall.push_back(reading.spiral_spectrum.clone());
                        while state.waterfall.len() > WATERFALL_HISTORY {
                            state.waterfall.pop_front();
                        }
                        while state.note_waterfall.len() > WATERFALL_HISTORY {
                            state.note_waterfall.pop_front();
                        }
                        while state.spiral_waterfall.len() > WATERFALL_HISTORY {
                            state.spiral_waterfall.pop_front();
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
                            spiral_spectrum: reading.spiral_spectrum,
                            spiral_waterfall: state.spiral_waterfall.iter().cloned().collect(),
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

    fn normalized_level(window: &[f32]) -> f32 {
        let rms = (window.iter().map(|sample| sample * sample).sum::<f32>() / window.len() as f32).sqrt();
        (rms / 0.1).clamp(0.0, 1.0)
    }

    fn analyze_window(
        window: &[f32],
        sample_rate: f32,
        settings: &AnalysisSettings,
        planner: &mut FftPlanner<f32>,
    ) -> Option<TunerReading> {
        let rms = (window.iter().map(|sample| sample * sample).sum::<f32>() / window.len() as f32).sqrt();
        if rms < SILENCE_RMS_THRESHOLD {
            return None;
        }

        let mut normalized = window.to_vec();
        normalized = apply_hann_window(&normalized);

        let (frequency_hz, clarity) = detect_pitch_yin(&normalized, sample_rate)?;
        if !(45.0..=1200.0).contains(&frequency_hz) {
            return None;
        }

        let (note_name, cents) = frequency_to_note(frequency_hz);
        let (spectrum, note_spectrum, spiral_spectrum) =
            spectrum_bars(&normalized, sample_rate, settings, planner);

        Some(TunerReading {
            frequency_hz,
            note_name,
            cents,
            clarity,
            spectrum,
            waterfall: Vec::new(),
            note_spectrum,
            note_waterfall: Vec::new(),
            spiral_spectrum,
            spiral_waterfall: Vec::new(),
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
        if !tau.is_finite() || tau <= 0.0 {
            return None;
        }

        let tau = tau.clamp(min_lag as f32, search_end as f32);
        let tau_index = tau.round().clamp(min_lag as f32, search_end as f32) as usize;
        let clarity = (1.0 - cumulative[tau_index].clamp(0.0, 1.0)).clamp(0.0, 1.0);
        if clarity < 0.35 {
            return None;
        }

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
            }
            .sanitized();

            assert!(settings.window_size >= MIN_WINDOW_SIZE);
            assert!(settings.fft_size >= settings.window_size.next_power_of_two());
            assert!(settings.max_frequency > settings.min_frequency);
            assert!(settings.spectrum_smoothing <= 4);
            assert!((0.15..=0.8).contains(&settings.note_spread));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod native {
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
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct TunerReading {
        pub frequency_hz:     f32,
        pub note_name:        String,
        pub cents:            f32,
        pub clarity:          f32,
        pub spectrum:         Vec<f32>,
        pub waterfall:        Vec<Vec<f32>>,
        pub note_spectrum:    Vec<f32>,
        pub note_waterfall:   Vec<Vec<f32>>,
        pub spiral_spectrum:  Vec<f32>,
        pub spiral_waterfall: Vec<Vec<f32>>,
        pub note_labels:      Vec<String>,
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
    }
}

pub use native::{
    AnalysisSettings,
    AudioEngine,
    AudioStatus,
    TunerReading,
};
