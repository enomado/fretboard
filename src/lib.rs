pub mod app;
pub mod audio;
pub mod core_types;
pub mod ui;

#[cfg(target_os = "android")]
pub mod android_insets;
#[cfg(target_os = "android")]
pub mod android_perm;

#[cfg(test)]
mod tests;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {
    android_perm::alog("android_main start");

    // Capture the real NativeActivity jobject for the runtime permission request.
    // `ndk_context` only exposes the Application object, which lacks the
    // Activity-only `requestPermissions` method — see `android_perm::ACTIVITY`.
    android_perm::init_activity(android_app.activity_as_ptr());
    // The phone shell keeps clear of the system bars by the activity's content rect.
    android_insets::init(&android_app);

    // Where the settings (`app::persist`) live. eframe finds no storage dir of its own on
    // Android (`eframe::storage_dir` is `None` there), so without this path nothing was
    // ever saved: every launch came up on the snail with default tuning, scale and
    // analysis settings. The app's private files dir is always there for an installed
    // app; eframe creates the file and saves it on every suspend (leaving the app, screen
    // off), which on Android may be the last chance before the process is killed.
    let persistence_path = android_app
        .internal_data_path()
        .expect("an installed Android app has an internal data dir")
        .join("app.ron");
    let native_options = eframe::NativeOptions {
        android_app: Some(android_app),
        persistence_path: Some(persistence_path),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Resonator Snail",
        native_options,
        Box::new(|cc| Ok(Box::new(app::create_app(cc)))),
    );
}
