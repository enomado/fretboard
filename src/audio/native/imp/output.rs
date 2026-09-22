//! Output: the monitor stream (hear what is being captured) and the one-shot test
//! note.

use std::sync::atomic::{
    AtomicU32,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
};
use std::thread;
use std::time::Duration;

use cpal::traits::{
    DeviceTrait,
    HostTrait,
    StreamTrait,
};
use resonators::midi_to_hz;
use ringbuf::traits::Consumer;

use super::devices::preferred_low_latency_buffer;
use super::{
    SampleConsumer,
    report_stream_error,
};
use crate::audio::core::{
    AnalysisPipeline,
    SharedState,
};
use crate::audio::types::AnalysisSettings;
use crate::core_types::pitch::PNote;

const TEST_TONE_GAIN: f32 = 0.28;
const TEST_TONE_DURATION: Duration = Duration::from_millis(1_600);

pub(super) fn build_monitor_output(
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
            |err| report_stream_error("Monitor output", &err),
            None,
        )
        .map_err(|e| format!("Failed to build monitor output: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start monitor output: {e}"))?;
    Ok((stream, actual_rate))
}

pub(super) fn play_test_note_thread(
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
    // Тест-нота звучит по текущему камертону, чтобы совпадать с анализом.
    let reference_hz = settings.lock().unwrap().concert_pitch_hz;
    let frequency = midi_to_hz(midi.as_u8() as f32, reference_hz);
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
            |err| report_stream_error("Test note output", &err),
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
