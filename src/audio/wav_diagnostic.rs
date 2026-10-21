//! Offline WAV entry point through the same two pipelines as native/web capture.
//! No device or wall-clock sleep: callbacks are stamped on the WAV's audio clock.

use std::fs::{
    self,
    File,
};
use std::io::{
    BufWriter,
    Write,
};
use std::path::Path;

use super::sample_rate::SampleRate;
use super::test_rig::Rig;
use super::types::{
    AnalysisSettings,
    PitchFrontend,
};
use crate::core_types::pitch::Midi;

/// Save line, raw estimates and reference smoothing with the complete settings.
/// Existing output files are refused so a diagnostic cannot silently erase a run.
pub fn run(wav: &Path, output: &Path, settings: AnalysisSettings) -> Result<usize, String> {
    let mut reader = hound::WavReader::open(wav).map_err(|e| format!("open WAV {}: {e}", wav.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate < 8_000 {
        return Err("expected mono WAV at 8000 Hz or higher".into());
    }
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>(),
        hound::SampleFormat::Int => {
            let scale = 2.0_f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect()
        }
    }
    .map_err(|e| format!("read WAV samples: {e}"))?;
    if samples.iter().any(|s| !s.is_finite()) {
        return Err("WAV contains non-finite samples".into());
    }
    fs::create_dir_all(output).map_err(|e| format!("create output directory: {e}"))?;
    let csv_name = wav.with_extension("csv");
    let csv_name = csv_name.file_name().ok_or("WAV has no filename")?;
    let diagnostics_name = Path::new(csv_name).with_extension("frames.csv");
    let trajectory = File::create_new(output.join(csv_name)).map_err(|e| format!("create pitch CSV: {e}"))?;
    let diagnostics = File::create_new(output.join(&diagnostics_name))
        .map_err(|e| format!("create frame diagnostics: {e}"))?;
    let metadata = File::create_new(output.join("run.json")).map_err(|e| format!("create run.json: {e}"))?;
    let settings = settings.sanitized();
    serde_json::to_writer_pretty(
        metadata,
        &serde_json::json!({
            "format": 1, "audio": wav.to_string_lossy(), "sample_rate": spec.sample_rate,
            "trajectory": Path::new(csv_name).to_string_lossy(), "diagnostics": diagnostics_name.to_string_lossy(),
            "samples": samples.len(), "detector": settings.resonator.frontend.key(),
            "settings": settings, "callback_ms": 10,
            "columns": "delivery time in seconds; frequencies in Hz; 0 means unvoiced",
        }),
    )
    .map_err(|e| format!("write run.json: {e}"))?;
    let sr = SampleRate(spec.sample_rate);
    let mut rig = Rig::new(sr);
    *rig.settings.lock().unwrap() = settings.clone();
    let mut writer = BufWriter::new(trajectory);
    let mut diagnostic_writer = BufWriter::new(diagnostics);
    writeln!(
        diagnostic_writer,
        "time,frequency,raw_frequency,reference_frequency,level,detector"
    )
    .map_err(|e| format!("write trajectory header: {e}"))?;
    let mut cursor = None;
    let mut count = 0;
    for part in samples.chunks(Rig::chunk(sr) * 50) {
        rig.feed(part, sr);
        for frame in rig.melody_since(cursor) {
            cursor = Some(frame.seq);
            let hz = |pitch: Option<Midi>| pitch.map_or(0.0, |m| m.to_hz(settings.concert_pitch_hz).0);
            writeln!(writer, "{:.9},{}", frame.t, hz(frame.pitch))
                .map_err(|e| format!("write pitch CSV: {e}"))?;
            writeln!(
                diagnostic_writer,
                "{:.9},{},{},{},{},{}",
                frame.t,
                hz(frame.pitch),
                hz(frame.raw_pitch),
                hz(frame.reference_pitch),
                frame.level,
                frame.frontend.key()
            )
            .map_err(|e| format!("write trajectory: {e}"))?;
            count += 1;
        }
    }
    writer.flush().map_err(|e| format!("flush trajectory: {e}"))?;
    diagnostic_writer
        .flush()
        .map_err(|e| format!("flush diagnostics: {e}"))?;
    Ok(count)
}

/// Parse the same method names accepted by the corpus benchmark.
pub fn settings_for(detector: &str) -> Result<AnalysisSettings, String> {
    let mut settings = AnalysisSettings::default();
    settings.resonator.frontend = PitchFrontend::from_key(detector)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_and_float_wavs_use_the_same_live_pipeline_for_every_detector() {
        let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join("t/scratch")
            .join(format!("fretboard-wav-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let samples: Vec<i16> = (0..24_000)
            .map(|i| (12000.0 * (std::f32::consts::TAU * 440.0 * i as f32 / 48000.0).sin()) as i16)
            .collect();
        let mut spec = hound::WavSpec {
            channels:        1,
            sample_rate:     48_000,
            bits_per_sample: 16,
            sample_format:   hound::SampleFormat::Int,
        };
        // The input name must not collide with the diagnostic output's name.
        let pcm = root.join("diagnostics.wav");
        let float = root.join("float.wav");
        let mut writer = hound::WavWriter::create(&pcm, spec).unwrap();
        for sample in &samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
        spec.bits_per_sample = 32;
        spec.sample_format = hound::SampleFormat::Float;
        let mut writer = hound::WavWriter::create(&float, spec).unwrap();
        for sample in &samples {
            writer.write_sample(*sample as f32 / 32768.0).unwrap();
        }
        writer.finalize().unwrap();
        for frontend in PitchFrontend::ALL {
            let settings = settings_for(frontend.key()).unwrap();
            let pcm_out = root.join(format!("{}-pcm", frontend.key()));
            let float_out = root.join(format!("{}-float", frontend.key()));
            let count = run(&pcm, &pcm_out, settings.clone()).unwrap();
            assert!(count >= 10);
            assert_eq!(count, run(&float, &float_out, settings.clone()).unwrap());
            assert_eq!(
                fs::read(pcm_out.join("diagnostics.csv")).unwrap(),
                fs::read(float_out.join("float.csv")).unwrap()
            );
            assert_eq!(
                fs::read(pcm_out.join("diagnostics.frames.csv")).unwrap(),
                fs::read(float_out.join("float.frames.csv")).unwrap()
            );
            let metadata: serde_json::Value =
                serde_json::from_reader(File::open(pcm_out.join("run.json")).unwrap()).unwrap();
            let restored: AnalysisSettings = serde_json::from_value(metadata["settings"].clone()).unwrap();
            assert_eq!(restored.resonator.frontend, frontend);
            assert!(
                run(&pcm, &pcm_out, settings).is_err(),
                "must refuse to overwrite a run"
            );
        }
    }
}
