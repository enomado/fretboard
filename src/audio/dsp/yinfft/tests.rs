use super::*;

fn tone(sample_rate: SampleRate, hz: f64, amplitude: f64) -> Vec<f32> {
    (0..WINDOW_SAMPLES)
        .map(|i| (amplitude * (std::f64::consts::TAU * hz * i as f64 / sample_rate.0 as f64).sin()) as f32)
        .collect()
}

#[test]
fn no_temporal_memory_at_capture_rates() {
    let mut count = 0;
    for rate in [8000, 16000, 44100, 48000, 96000] {
        let sr = SampleRate(rate);
        let mut detector = YinFft::new(sr);
        for hz in [110.0, 440.0, 1760.0, 2637.0] {
            let samples = tone(sr, hz, 0.6);
            let found = detector.estimate(&samples);
            // Pitch accuracy is checked against the independent WASM fixture
            // below: R2 itself has a short-window bias and subharmonic minima
            // on some pure tones, so proximity to the generator is not its
            // numerical contract. This test checks reuse and finite output.
            if let Some(found) = found {
                assert!(found.hz.0.is_finite() && found.hz.0 > 0.0);
                assert!(found.periodicity.is_finite());
            }
            assert_eq!(
                found.map(|found| found.hz),
                YinFft::new(sr).estimate(&samples).map(|found| found.hz)
            );
            count += 1;
        }
    }
    println!("{count} rate/frequency controls, reused == fresh");
}

#[test]
fn silence_gate_uses_untapered_window_energy() {
    let sr = SampleRate(44100);
    let mut detector = YinFft::new(sr);
    assert!(detector.estimate(&vec![0.0; WINDOW_SAMPLES]).is_none());
    assert!(detector.estimate(&tone(sr, 440.0, 0.004)).is_none());
    assert!(detector.estimate(&tone(sr, 440.0, 0.005)).is_some());
    assert!(detector.estimate(&vec![0.5; WINDOW_SAMPLES]).is_none());
}

#[test]
fn missing_fundamental_and_noise_voicing() {
    let sr = SampleRate(44100);
    let mut samples = tone(sr, 880.0, 0.3);
    for (sample, third) in samples.iter_mut().zip(tone(sr, 1320.0, 0.3)) {
        *sample += third;
    }
    let mut detector = YinFft::new(sr);
    let found = detector.estimate(&samples).unwrap();
    assert!((1200.0 * (found.hz.0 / 440.0).log2()).abs() < 5.0);
    // Fixed integer noise, independent of a random library or global seed.
    let mut state = 1u32;
    for sample in &mut samples {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *sample = ((state >> 16) as f32 / 32768.0 - 1.0) * 0.3;
    }
    assert!(detector.estimate(&samples).is_none());
}

#[test]
#[should_panic(expected = "complete window")]
fn rejects_incomplete_window() {
    YinFft::new(SampleRate(44100)).estimate(&[0.0; 1024]);
}

/// A small checked-in reference, so ordinary tests exercise the numerical
/// contract without Node, WASM, a network connection or the external corpus.
#[test]
fn preserved_wasm_windows() {
    #[derive(serde::Deserialize)]
    struct Case {
        rate:        u32,
        hz:          f64,
        amplitude:   f64,
        kind:        String,
        expected_hz: f32,
    }
    #[derive(serde::Deserialize)]
    struct Fixture {
        window_samples: usize,
        cases:          Vec<Case>,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("../../../../testdata/yinfft_oracle.json")).unwrap();
    assert_eq!(fixture.window_samples, WINDOW_SAMPLES);
    let count = fixture.cases.len();
    assert_eq!(count, 365);
    let mut max_cents = 0.0f32;
    for case in fixture.cases {
        let mut state = 1u32;
        let samples: Vec<f32> = (0..WINDOW_SAMPLES)
            .map(|i| {
                let phase = std::f64::consts::TAU * case.hz * i as f64 / case.rate as f64;
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (state >> 16) as f64 / 32768.0 - 1.0;
                let value = match case.kind.as_str() {
                    "pure" => phase.sin(),
                    "missing" => ((2.0 * phase).sin() + (3.0 * phase).sin()) / 2.0,
                    "harmonic_noise" => {
                        (phase.sin() + 0.8 * (2.0 * phase).sin() + 0.6 * (3.0 * phase).sin() + 0.3 * noise)
                            / 2.4
                    }
                    "noise" => noise,
                    "dc" => 1.0,
                    _ => panic!("unknown oracle signal"),
                };
                (case.amplitude * value) as f32
            })
            .collect();
        let actual = YinFft::new(SampleRate(case.rate)).estimate(&samples);
        assert_eq!(
            actual.is_some(),
            case.expected_hz > 0.0,
            "{} Hz @ {}, {}, amplitude {}: {actual:?} vs {}",
            case.hz,
            case.rate,
            case.kind,
            case.amplitude,
            case.expected_hz
        );
        if let Some(found) = actual {
            let cents = (1200.0 * (found.hz.0 / case.expected_hz).log2()).abs();
            max_cents = max_cents.max(cents);
            assert!(
                cents < 1.0,
                "{} Hz @ {}, {}: {cents} cents",
                case.hz,
                case.rate,
                case.kind
            );
        }
    }
    println!("{count} preserved WASM windows: max {max_cents:.6} cents, voicing identical");
}

/// Same PCM16 WAVs and every CSV row used by R2, including all sixteen step
/// positions. Kept ignored because the preserved oracle lives outside git.
#[test]
#[ignore = "needs datasets/app_yinfft_2048_2026-10-20/step_controls"]
fn r2_step_oracle() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("datasets/app_yinfft_2048_2026-10-20/step_controls");
    let mut cases = 0;
    let mut frames = 0;
    let mut max_cents = 0.0f32;
    let mut delays = [Vec::new(), Vec::new()];
    for entry in std::fs::read_dir(root.join("inputs")).unwrap() {
        let wav = entry.unwrap().path();
        let name = wav.file_stem().unwrap().to_str().unwrap();
        let mut reader = hound::WavReader::open(&wav).unwrap();
        let sr = SampleRate(reader.spec().sample_rate);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        let reference =
            std::fs::read_to_string(root.join("yinfft_2048").join(format!("{name}.csv"))).unwrap();
        let expected: Vec<(f64, f32)> = reference
            .lines()
            .map(|line| {
                let (t, f) = line.split_once(',').unwrap();
                (t.parse().unwrap(), f.parse().unwrap())
            })
            .collect();
        let mut detector = YinFft::new(sr);
        let mut actual = Vec::new();
        for (row, start) in (0..=samples.len() - WINDOW_SAMPLES)
            .step_by(sr.samples_in(HOP))
            .enumerate()
        {
            let t = (start + WINDOW_SAMPLES) as f64 / sr.0 as f64;
            let hz = detector
                .estimate(&samples[start..start + WINDOW_SAMPLES])
                .map(|f| f.hz.0);
            assert!((t - expected[row].0).abs() < 1e-9);
            assert_eq!(hz.is_some(), expected[row].1 > 0.0, "{name} row {row}");
            if let Some(hz) = hz {
                let cents = (1200.0 * (hz / expected[row].1).log2()).abs();
                max_cents = max_cents.max(cents);
                assert!(cents < 1.0, "{name} row {row}: {cents} cents");
            }
            actual.push((t, hz));
        }
        assert_eq!(actual.len(), expected.len());
        let (target, step) = if let Some((kind, position)) = name.split_once('_') {
            let step = (22050 + 256 * position.parse::<usize>().unwrap()) as f64 / sr.0 as f64;
            (
                if kind == "tone" {
                    440.0 * 2.0f32.powf(2.0 / 12.0)
                } else {
                    880.0
                },
                Some(step),
            )
        } else {
            (440.0, None)
        };
        let correct = |hz: Option<f32>, tolerance: f32| {
            hz.is_some_and(|hz| (1200.0 * (hz / target).log2()).abs() <= tolerance)
        };
        assert!(
            actual.iter().rev().take(10).all(|(_, hz)| correct(*hz, 5.0)),
            "{name} final pitch"
        );
        if let Some(step) = step {
            let first = actual
                .windows(3)
                .find(|triple| triple[0].0 >= step && triple.iter().all(|(_, hz)| correct(*hz, 50.0)))
                .unwrap();
            delays[usize::from(name.starts_with("octave"))].push((first[0].0 - step) * 1000.0);
        }
        cases += 1;
        frames += actual.len();
    }
    assert_eq!(cases, 34);
    for (label, values) in ["tone", "octave"].into_iter().zip(delays) {
        assert_eq!(values.len(), 16);
        println!(
            "{label}: {:.6}..{:.6} ms",
            values.iter().copied().fold(f64::INFINITY, f64::min),
            values.iter().copied().fold(0.0, f64::max)
        );
    }
    println!("R2 steps: {cases} cases, {frames} frames, max {max_cents:.6} cents, voicing identical");
}
