#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use eframe::CreationContext;
#[cfg(not(target_arch = "wasm32"))]
use eframe::NativeOptions;
use fretboard::app::App;

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct HotReloadMsg {
    jump_table: Option<subsecond::JumpTable>,
    for_pid:    Option<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
enum DevserverMsg {
    HotReload(HotReloadMsg),
    #[serde(other)]
    Other,
}

#[cfg(not(target_arch = "wasm32"))]
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
            if let tungstenite::Message::Text(text) = msg {
                if let Ok(DevserverMsg::HotReload(msg)) = serde_json::from_str(&text) {
                    if msg.for_pid == Some(std::process::id()) {
                        if let Some(jump_table) = msg.jump_table {
                            unsafe { subsecond::apply_patch(jump_table).unwrap() };
                        }
                    }
                }
            }
        }
    });
}

fn create_app(cc: &CreationContext) -> App {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = cc.egui_ctx.clone();
        ctx.set_pixels_per_point(1.75);
        subsecond::register_handler(Arc::new(move || ctx.request_repaint()));
    }

    App::new(cc)
}

#[cfg(not(target_arch = "wasm32"))]
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
