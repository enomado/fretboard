#![cfg(not(target_arch = "wasm32"))]

use std::io::Read;
use std::process::{
    Command,
    Stdio,
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
use cpal::{
    FromSample,
    Sample,
};

const DEFAULT_DURATION: Duration = Duration::from_millis(900);
const PULSE_RATE: u32 = 48_000;

#[derive(Clone)]
struct Candidate {
    backend: String,
    route:   String,
    label:   String,
    device:  Option<cpal::Device>,
}

#[derive(Clone, Debug, Default)]
struct LevelStats {
    samples: u64,
    sum_sq:  f64,
    peak:    f32,
}

impl LevelStats {
    fn push(&mut self, sample: f32) {
        self.samples += 1;
        self.sum_sq += f64::from(sample * sample);
        self.peak = self.peak.max(sample.abs());
    }

    fn rms(&self) -> f32 {
        if self.samples == 0 {
            0.0
        } else {
            (self.sum_sq / self.samples as f64).sqrt() as f32
        }
    }
}

#[derive(Clone)]
struct ProbeResult {
    candidate: Candidate,
    status:    String,
    stats:     LevelStats,
    rate:      Option<u32>,
}

fn main() {
    let duration = probe_duration();
    println!("Probing audio inputs for {:.1}s each\n", duration.as_secs_f32());

    let candidates = collect_candidates();
    if candidates.is_empty() {
        println!("No input candidates found.");
        return;
    }

    let mut results = Vec::new();
    for candidate in candidates {
        println!("trying {:<5} {}", candidate.backend, candidate.label);
        results.push(probe_candidate(candidate, duration));
    }

    results.sort_by(|left, right| {
        right
            .stats
            .rms()
            .total_cmp(&left.stats.rms())
            .then_with(|| right.stats.peak.total_cmp(&left.stats.peak))
    });

    println!(
        "\n{:<7} {:<10} {:>9} {:>9} {:>10} route / label",
        "backend", "status", "rms", "peak", "samples"
    );
    for result in results {
        let rate = result.rate.map_or("-".to_owned(), |rate| format!("{rate}Hz"));
        println!(
            "{:<7} {:<10} {:>9.6} {:>9.6} {:>10} {}",
            result.candidate.backend,
            result.status,
            result.stats.rms(),
            result.stats.peak,
            result.stats.samples,
            result.candidate.route,
        );
        println!("                    {} ({rate})", result.candidate.label);
    }
}

fn probe_duration() -> Duration {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--duration-ms" {
            if let Some(value) = args.next().and_then(|value| value.parse::<u64>().ok()) {
                return Duration::from_millis(value.clamp(150, 5_000));
            }
        }
    }
    DEFAULT_DURATION
}

fn collect_candidates() -> Vec<Candidate> {
    let mut candidates = Vec::new();

    if command_ok("parec", "--version") {
        candidates.push(Candidate {
            backend: "pulse".to_owned(),
            route:   "@DEFAULT_SOURCE@".to_owned(),
            label:   "Default microphone (Pulse/PipeWire)".to_owned(),
            device:  None,
        });
        candidates.extend(pulse_source_candidates());
    }

    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .map(|device| cpal_device_display_name(&device));
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            let name = cpal_device_display_name(&device);
            let default_tag = if default_name.as_deref() == Some(name.as_str()) {
                " (default)"
            } else {
                ""
            };
            candidates.push(Candidate {
                backend: "cpal".to_owned(),
                route:   name.clone(),
                label:   format!("{name}{default_tag}"),
                device:  Some(device),
            });
        }
    }

    candidates
}

fn pulse_source_candidates() -> Vec<Candidate> {
    let Ok(output) = Command::new("pactl").args(["list", "sources"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut candidates = Vec::new();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Source #") {
            push_pulse_candidate(&mut candidates, name.take(), description.take());
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Name:") {
            name = Some(value.trim().to_owned());
        } else if let Some(value) = trimmed.strip_prefix("Description:") {
            description = Some(value.trim().to_owned());
        }
    }
    push_pulse_candidate(&mut candidates, name, description);
    candidates
}

fn push_pulse_candidate(candidates: &mut Vec<Candidate>, name: Option<String>, description: Option<String>) {
    let Some(name) = name else {
        return;
    };
    if name == "@DEFAULT_SOURCE@" || name == "@DEFAULT_MONITOR@" {
        return;
    }
    let label = description
        .filter(|description| !description.trim().is_empty())
        .unwrap_or_else(|| name.clone());
    candidates.push(Candidate {
        backend: "pulse".to_owned(),
        route: name,
        label,
        device: None,
    });
}

fn probe_candidate(candidate: Candidate, duration: Duration) -> ProbeResult {
    if let Some(device) = candidate.device.clone() {
        probe_cpal(candidate, &device, duration)
    } else {
        probe_pulse(candidate, duration)
    }
}

fn probe_pulse(candidate: Candidate, duration: Duration) -> ProbeResult {
    let mut child = match Command::new("parec")
        .args([
            "--record",
            "--raw",
            "--format=s16le",
            "--channels=1",
            "--rate",
            &PULSE_RATE.to_string(),
            "--latency-msec",
            "20",
            "--process-time-msec",
            "10",
            "--client-name=fretboard-mic-probe",
            "--stream-name=fretboard-mic-probe",
            "--device",
            &candidate.route,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ProbeResult {
                candidate,
                status: format!("spawn:{err}"),
                stats: LevelStats::default(),
                rate: Some(PULSE_RATE),
            };
        }
    };

    let Some(mut stdout) = child.stdout.take() else {
        return ProbeResult {
            candidate,
            status: "no-stdout".to_owned(),
            stats: LevelStats::default(),
            rate: Some(PULSE_RATE),
        };
    };

    let reader = thread::spawn(move || {
        let mut data = Vec::new();
        let _ = stdout.read_to_end(&mut data);
        data
    });

    thread::sleep(duration);
    let _ = child.kill();
    let _ = child.wait();
    let data = reader.join().unwrap_or_default();
    let stats = stats_from_i16le(&data);
    let status = if stats.samples == 0 { "silent" } else { "ok" }.to_owned();

    ProbeResult {
        candidate,
        status,
        stats,
        rate: Some(PULSE_RATE),
    }
}

fn probe_cpal(candidate: Candidate, device: &cpal::Device, duration: Duration) -> ProbeResult {
    let config = match device.default_input_config() {
        Ok(config) => config,
        Err(err) => return error_result(candidate, format!("config:{err}"), None),
    };
    let rate = config.sample_rate();
    let channels = usize::from(config.channels());
    let stats = Arc::new(Mutex::new(LevelStats::default()));
    let stream_config = config.clone().into();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            build_cpal_probe_stream::<f32>(device, &stream_config, channels, stats.clone())
        }
        cpal::SampleFormat::I16 => {
            build_cpal_probe_stream::<i16>(device, &stream_config, channels, stats.clone())
        }
        cpal::SampleFormat::U16 => {
            build_cpal_probe_stream::<u16>(device, &stream_config, channels, stats.clone())
        }
        other => Err(format!("format:{other:?}")),
    };
    let stream = match stream {
        Ok(stream) => stream,
        Err(err) => return error_result(candidate, err, Some(rate)),
    };

    if let Err(err) = stream.play() {
        return error_result(candidate, format!("play:{err}"), Some(rate));
    }
    thread::sleep(duration);
    let _ = stream.pause();
    drop(stream);

    let stats = stats.lock().map(|stats| stats.clone()).unwrap_or_default();
    let status = if stats.samples == 0 { "silent" } else { "ok" }.to_owned();
    ProbeResult {
        candidate,
        status,
        stats,
        rate: Some(rate),
    }
}

fn build_cpal_probe_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    stats: Arc<Mutex<LevelStats>>,
) -> Result<cpal::Stream, String>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if let Ok(mut stats) = stats.lock() {
                    for frame in data.chunks(channels) {
                        if let Some(raw) = frame.first() {
                            stats.push(f32::from_sample(*raw));
                        }
                    }
                }
            },
            |err| eprintln!("input stream error: {err}"),
            None,
        )
        .map_err(|err| format!("build:{err}"))
}

fn stats_from_i16le(data: &[u8]) -> LevelStats {
    let mut stats = LevelStats::default();
    for pair in data.chunks_exact(2) {
        let sample = f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        stats.push(sample);
    }
    stats
}

fn error_result(candidate: Candidate, status: String, rate: Option<u32>) -> ProbeResult {
    ProbeResult {
        candidate,
        status,
        stats: LevelStats::default(),
        rate,
    }
}

fn command_ok(command: &str, arg: &str) -> bool {
    Command::new(command)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn cpal_device_display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| "Unknown input".to_owned())
}
