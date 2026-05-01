#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use eframe::NativeOptions;
#[cfg(not(target_os = "android"))]
use fretboard::app::create_app;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
#[derive(serde::Deserialize)]
struct HotReloadMsg {
    jump_table: Option<subsecond::JumpTable>,
    for_pid:    Option<u32>,
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
#[derive(serde::Deserialize)]
enum DevserverMsg {
    HotReload(HotReloadMsg),
    #[serde(other)]
    Other,
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn connect_subsecond() {
    let Some(endpoint) = dioxus_cli_config::devserver_ws_endpoint() else {
        return;
    };

    std::thread::spawn(move || {
        let uri = format!(
            "{endpoint}?aslr_reference={}&build_id={}&pid={}",
            subsecond::aslr_reference(),
            dioxus_cli_config::build_id(),
            std::process::id()
        );

        let (mut websocket, _) = match tungstenite::connect(uri) {
            Ok(v) => v,
            Err(_) => return,
        };

        while let Ok(msg) = websocket.read() {
            if let tungstenite::Message::Text(text) = msg
                && let Ok(DevserverMsg::HotReload(msg)) = serde_json::from_str(&text)
                && msg.for_pid == Some(std::process::id())
                && let Some(jump_table) = msg.jump_table
            {
                unsafe { subsecond::apply_patch(jump_table).unwrap() };
            }
        }
    });
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn main() -> eframe::Result {
    connect_subsecond();

    subsecond::call(|| {
        eframe::run_native(
            "fretboard",
            NativeOptions::default(),
            Box::new(|cc| Ok(Box::new(create_app(cc)))),
        )
    })
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("fretboard_canvas")
            .unwrap()
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();

        eframe::WebRunner::new()
            .start(canvas, web_options, Box::new(|cc| Ok(Box::new(create_app(cc)))))
            .await
            .unwrap();
    });
}

#[cfg(target_os = "android")]
fn main() {
}
