//! Runtime `RECORD_AUDIO` permission glue for the NativeActivity Android build.
//!
//! Why this exists: NativeActivity has no Java subclass, so the framework's
//! `onRequestPermissionsResult()` callback never reaches Rust (see
//! rust-mobile/android-activity#174). We therefore drive the permission by hand:
//!
//!   * fire the system dialog once via JNI `Activity.requestPermissions(...)`,
//!   * observe the outcome by *polling* `Context.checkSelfPermission(...)` —
//!     there is no callback to await.
//!
//! The audio engine opens its capture stream once at startup; on a fresh install
//! that happens before the user taps "Allow", so the first open fails. The UI
//! layer watches [`newly_granted`] and re-opens capture on the rising edge.

use std::ffi::{
    CString,
    c_char,
    c_int,
    c_void,
};
use std::ptr;
use std::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    Ordering,
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
/// `android.content.pm.PackageManager.PERMISSION_GRANTED` is the constant `0`.
const PERMISSION_GRANTED: jint = 0;

// Direct logcat output. The `log` crate is silent here (some dependency compiles
// it out via a `max_level_*` feature), so we call liblog's `__android_log_write`
// straight. priority 6 = ANDROID_LOG_ERROR (always shown).
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Write one line to logcat under tag "snail" (visible via `adb logcat -s snail`).
pub fn alog(msg: &str) {
    let text = CString::new(msg).unwrap_or_default();
    unsafe { __android_log_write(6, c"snail".as_ptr(), text.as_ptr()) };
}

/// We fire the dialog exactly once per process. Re-prompting every frame would be
/// hostile; a denied user can still grant from Settings.
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// Latches once we have observed the grant and told the audio engine to re-open.
static KICKED: AtomicBool = AtomicBool::new(false);

/// Global ref to the real `NativeActivity` instance, stashed from `android_main`.
///
/// Why not use `ndk_context::android_context().context()`? android-activity
/// publishes the **Application** object there (see android-activity `init.rs`,
/// `get_application(...)`), not the Activity. The Application is a `Context`, so
/// `checkSelfPermission` resolves on it — but `requestPermissions` is an
/// `Activity`-only method and JNI reports `MethodNotFound` for it. The real
/// Activity comes from `AndroidApp::activity_as_ptr()`, a global ref valid for
/// the process lifetime; we capture it once via [`init_activity`].
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// Stash the `NativeActivity` jobject (from `AndroidApp::activity_as_ptr()`).
/// Must be called from `android_main` before any permission call.
pub fn init_activity(activity: *mut c_void) {
    ACTIVITY.store(activity, Ordering::Relaxed);
}

/// Attach the current thread to the JVM that android-activity published in
/// `ndk_context`, then run `f` with a JNI env plus the Activity object (the one
/// captured by [`init_activity`], *not* `ndk_context`'s Application — see
/// [`ACTIVITY`]). Returns `None` if the activity isn't stashed yet (called too
/// early) or a JNI call failed — callers treat that as "not granted / not done",
/// so a missed early frame is harmless.
fn with_activity<R>(f: impl FnOnce(&mut Env, &JObject) -> Result<R, Error>) -> Option<R> {
    let ctx = ndk_context::android_context();
    let raw_activity = ACTIVITY.load(Ordering::Relaxed);
    if ctx.vm().is_null() || raw_activity.is_null() {
        alog("activity not ready (vm null or init_activity not called)");
        return None;
    }
    // SAFETY: `vm()` is the process JavaVM (published by ndk_context) and
    // `raw_activity` is a global ref to the NativeActivity instance that
    // android-activity keeps alive for the whole process lifetime.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    match vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(&*env, raw_activity.cast()) };
        f(env, &activity)
    }) {
        Ok(value) => Some(value),
        Err(err) => {
            alog(&format!("JNI call failed: {err:?}"));
            None
        }
    }
}

/// `Context.checkSelfPermission(RECORD_AUDIO) == PERMISSION_GRANTED`.
pub fn record_audio_granted() -> bool {
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

/// Fire the system permission dialog at most once. No-op if already granted or
/// already asked this run. The result is read back via [`newly_granted`].
pub fn request_record_audio() {
    if KICKED.load(Ordering::Relaxed) || REQUESTED.swap(true, Ordering::Relaxed) {
        return;
    }
    if record_audio_granted() {
        alog("RECORD_AUDIO already granted, not requesting");
        return;
    }
    alog("requesting RECORD_AUDIO via Activity.requestPermissions");
    // One-shot sanity log: confirm the stashed object is actually an Activity
    // (it must be, now that we capture activity_as_ptr() rather than the
    // ndk_context Application). If this ever prints `false`, the capture broke.
    with_activity(|env, activity| {
        let is_activity = env.is_instance_of(activity, jni_str!("android/app/Activity"))?;
        alog(&format!("activity is_instance_of Activity = {is_activity}"));
        Ok(())
    });
    let fired = with_activity(|env, activity| {
        let perm = env.new_string(RECORD_AUDIO)?;
        // String[]{ RECORD_AUDIO } — requestPermissions takes an array.
        let array = env.new_object_array(1, jni_str!("java/lang/String"), &perm)?;
        // Activity.requestPermissions(String[] permissions, int requestCode).
        // requestCode is irrelevant: we poll the result, we don't read the callback.
        env.call_method(
            activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::Object(&array), JValue::Int(0)],
        )?;
        Ok(())
    });
    alog(&format!("requestPermissions fired ok = {}", fired.is_some()));
}

/// Returns `true` exactly once: on the first poll where the permission is seen
/// granted. The caller uses that rising edge to (re-)open the capture stream.
/// After it fires once this stays `false` forever (no further JNI polling).
pub fn newly_granted() -> bool {
    if KICKED.load(Ordering::Relaxed) {
        return false;
    }
    if record_audio_granted() {
        KICKED.store(true, Ordering::Relaxed);
        return true;
    }
    false
}
