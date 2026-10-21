#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    use std::path::Path;

    use fretboard::audio::types::PitchFrontend;
    use fretboard::audio::wav_diagnostic;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(3..=4).contains(&args.len()) {
        return Err(
            "usage: pitch_wav <bank|rtswipe|pyin|sieve|comb|yinfft> <mono.wav> <output-dir> [settings.json]"
                .into(),
        );
    }
    let mut settings = if let Some(path) = args.get(3) {
        serde_json::from_reader(std::fs::File::open(path).map_err(|e| format!("open settings: {e}"))?)
            .map_err(|e| format!("parse settings: {e}"))?
    } else {
        wav_diagnostic::settings_for(&args[0])?
    };
    settings.resonator.frontend = PitchFrontend::from_key(&args[0])?;
    let frames = wav_diagnostic::run(Path::new(&args[1]), Path::new(&args[2]), settings)?;
    println!(
        "{frames} frames → {} (pitch CSV, frame diagnostics, run.json)",
        args[2]
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {
}
