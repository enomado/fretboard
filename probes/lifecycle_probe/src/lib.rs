//! What actually reaches Rust around the runtime-permission dialog?
//!
//! Measurement for <https://github.com/rust-mobile/android-activity/issues/174>.
//!
//! Under `NativeActivity` there is no `onRequestPermissionsResult()` callback, so
//! the proposed workaround is to treat "the activity resumed" as the signal that
//! the dialog closed. That rests on an assumption nobody has published numbers
//! for: **does the permission dialog actually pause/resume the activity
//! underneath it?** The dialog is a translucent activity belonging to another
//! process, so it might only take focus — and OEM builds may differ.
//!
//! This probe answers that empirically. It does nothing but log, with
//! milliseconds since start, every `MainEvent` plus the live result of
//! `checkSelfPermission`, and fires the dialog once so the interesting window is
//! bracketed by markers in the log.
//!
//! Deliberately standalone (no eframe, no fretboard deps): eframe owns the event
//! loop and swallows `MainEvent`, so the transitions are invisible from inside a
//! real app. The JNI glue below is duplicated from `src/android_perm.rs` rather
//! than shared, so this directory can be copied out and built by anyone.

use std::ffi::{
    CString,
    c_char,
    c_int,
    c_void,
};
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    Ordering,
};
use std::time::{
    Duration,
    Instant,
};

use android_activity::{
    AndroidApp,
    MainEvent,
    PollEvent,
};
use jni::errors::Error;
use jni::objects::JObject;
use jni::sys::jint;
use jni::{
    Env,
    JValue,
    JavaVM,
    jni_sig,
    jni_str,
};

const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";
/// `android.content.pm.PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: jint = 0;

/// How long to sit idle after the first `Resume` before provoking the dialog.
/// Long enough that the startup burst of events has clearly finished, so the log
/// around the dialog is unambiguous.
const SETTLE_BEFORE_REQUEST: Duration = Duration::from_secs(2);

unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Zero of the timeline. Every line is stamped relative to this so the ordering
/// and the gaps are both readable.
static START: OnceLock<Instant> = OnceLock::new();
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// One line to logcat, tagged `probe` (priority 6 = ERROR, so it survives any
/// log-level filtering). `adb logcat -s probe`.
fn log(msg: &str) {
    let ms = START.get_or_init(Instant::now).elapsed().as_millis();
    let text = CString::new(format!("[{ms:>6} ms] {msg}")).unwrap_or_default();
    unsafe { __android_log_write(6, c"probe".as_ptr(), text.as_ptr()) };
}

/// Attach to the JVM and run `f` with the real `Activity` (see the issue: the
/// object `ndk_context` publishes is the *Application* as of android-activity
/// 0.6.1, and `requestPermissions` is an Activity-only method).
fn with_activity<R>(f: impl FnOnce(&mut Env, &JObject) -> Result<R, Error>) -> Option<R> {
    let ctx = ndk_context::android_context();
    let raw_activity = ACTIVITY.load(Ordering::Relaxed);
    if ctx.vm().is_null() || raw_activity.is_null() {
        return None;
    }
    // SAFETY: `vm()` is the process JavaVM published by ndk_context; `raw_activity`
    // is the global ref owned by `AndroidApp`, which outlives this call because
    // `android_main` holds the `AndroidApp` for the duration of the loop below.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    match vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(&*env, raw_activity.cast()) };
        f(env, &activity)
    }) {
        Ok(value) => Some(value),
        Err(err) => {
            log(&format!("!! JNI call failed: {err:?}"));
            None
        }
    }
}

fn granted() -> bool {
    with_activity(|env, activity| {
        let perm = env.new_string(RECORD_AUDIO)?;
        let code = env
            .call_method(
                activity,
                jni_str!("checkSelfPermission"),
                jni_sig!("(Ljava/lang/String;)I"),
                &[JValue::Object(&perm)],
            )?
            .i()?;
        Ok(code == PERMISSION_GRANTED)
    })
    .unwrap_or(false)
}

/// Fire the system dialog exactly once.
fn request_once() {
    if REQUESTED.swap(true, Ordering::Relaxed) {
        return;
    }
    log("=== FIRING requestPermissions — dialog should appear now ===");
    let fired = with_activity(|env, activity| {
        let perm = env.new_string(RECORD_AUDIO)?;
        let array = env.new_object_array(1, jni_str!("java/lang/String"), &perm)?;
        // requestCode is irrelevant: under NativeActivity the result callback
        // never arrives, which is the whole reason this probe exists.
        env.call_method(
            activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::Object(&array), JValue::Int(0)],
        )?;
        Ok(())
    });
    log(&format!("=== requestPermissions returned ok={} ===", fired.is_some()));
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    START.get_or_init(Instant::now);
    ACTIVITY.store(app.activity_as_ptr(), Ordering::Relaxed);
    log("probe start — waiting for first Resume before provoking the dialog");

    // Set once we see the first Resume; the request fires SETTLE_BEFORE_REQUEST
    // later. Requesting before the activity is resumed is a known way to get no
    // dialog at all, which would measure nothing.
    let mut resumed_at: Option<Instant> = None;
    // Only log permission transitions, not the state on every event — otherwise
    // the flip we care about is buried in noise.
    let mut last_granted = granted();
    log(&format!("initial checkSelfPermission granted={last_granted}"));

    loop {
        // Short timeout so the post-dialog permission flip is timestamped
        // accurately even if no event accompanies it — the null result ("nothing
        // fires, only the poll notices") is itself a finding worth recording.
        app.poll_events(Some(Duration::from_millis(100)), |event| {
            match event {
                PollEvent::Main(main) => {
                    // `MainEvent` is #[non_exhaustive] and derives Debug; printing
                    // it verbatim keeps the probe honest about variants we didn't
                    // anticipate, instead of matching a hand-picked subset.
                    log(&format!("MainEvent::{main:?}"));
                    if matches!(main, MainEvent::Resume { .. }) && resumed_at.is_none() {
                        resumed_at = Some(Instant::now());
                    }
                }
                // Wake/Timeout are pure loop noise — not logged.
                PollEvent::Wake | PollEvent::Timeout => {}
                _ => {}
            }
        });

        let now_granted = granted();
        if now_granted != last_granted {
            log(&format!(">>> checkSelfPermission FLIPPED to granted={now_granted}"));
            last_granted = now_granted;
        }

        if let Some(at) = resumed_at
            && at.elapsed() >= SETTLE_BEFORE_REQUEST
        {
            request_once();
        }
    }
}
