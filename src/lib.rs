pub mod app;
pub mod audio;
pub mod core_types;
pub mod fretboard;
pub mod ui;

#[cfg(test)]
mod tests;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {
    let native_options = eframe::NativeOptions {
        android_app: Some(android_app),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Resonator Snail",
        native_options,
        Box::new(|cc| Ok(Box::new(app::create_app(cc)))),
    );
}
