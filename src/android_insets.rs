//! System-bar insets for the NativeActivity Android build: how much of the window the
//! status bar, the navigation buttons and cutouts cover.
//!
//! egui has a place for this (`RawInput::safe_area_insets`, honoured by
//! `Context::content_rect`), but egui-winit 0.37 fills it on iOS only — on Android the
//! surface is the whole window and egui believes every pixel of it is ours. Under
//! HyperOS's three-button navigation bar the phone shell's bottom row (the config card's
//! last knobs) sat behind the buttons and took their taps.
//!
//! The source is android-activity's content rect: NativeActivity's
//! `onContentRectChanged`, the part of the window its content view is laid out in, which
//! is the window minus the system bars the theme does not hide. It is in window pixels;
//! the insets are the window's edges minus that rect's.

use std::sync::OnceLock;

use android_activity::AndroidApp;
use eframe::egui::{
    self,
    Margin,
};

/// The app handle `android_main` was given — the content rect is read off it each frame
/// (a mutex lock inside android-activity, updated on the activity's callback thread).
static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Remember the app handle. Called once, from `android_main`, before eframe starts.
pub fn init(app: &AndroidApp) {
    APP.set(app.clone())
        .ok()
        .expect("android_insets::init is called once");
}

/// The system-bar insets of the current frame, in egui points (rounded up: a bar's last
/// partial point is still the bar's).
pub fn insets(ctx: &egui::Context) -> Margin {
    let app = APP.get().expect("android_insets::init runs in android_main");
    let content = app.content_rect();
    let ppp = ctx.pixels_per_point();
    let window = ctx.viewport_rect();
    let window_w = window.width() * ppp;
    let window_h = window.height() * ppp;
    // An empty rect = the activity has not reported it yet (first frames): no insets
    // rather than a negative margin.
    if content.right <= content.left || content.bottom <= content.top {
        return Margin::ZERO;
    }
    // Half a pixel off first: the viewport's size comes back from points as e.g.
    // 1600.03 px, and that float residue alone must not ceil into a 1-point margin.
    let side = |px: f32| ((px - 0.5).max(0.0) / ppp).ceil() as i8;
    let margin = Margin {
        left:   side(content.left as f32),
        right:  side(window_w - content.right as f32),
        top:    side(content.top as f32),
        bottom: side(window_h - content.bottom as f32),
    };
    log_once(&format!(
        "insets: content {}..{} x {}..{} px, window {window_w}x{window_h} px, ppp {ppp} → {margin:?}",
        content.left, content.right, content.top, content.bottom
    ));
    margin
}

/// Log each distinct inset reading once — the numbers are the only way to see on a
/// device what the activity reported (rotations and bar changes produce new lines).
fn log_once(line: &str) {
    use std::sync::Mutex;
    static LAST: Mutex<String> = Mutex::new(String::new());
    let mut last = LAST.lock().unwrap();
    if *last != line {
        crate::android_perm::alog(line);
        *last = line.to_owned();
    }
}
