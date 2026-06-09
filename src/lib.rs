pub mod app;
pub mod audio;
pub mod core_types;
pub mod ui;

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
