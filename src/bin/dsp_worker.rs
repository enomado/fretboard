//! DSP web worker binary.
//!
//! trunk builds this as a `data-type="worker"` target (see index.html). It's a
//! thin shim: the real worker logic lives in the library (`audio::worker`) so it
//! can reach the crate-internal DSP pipelines. On non-wasm targets this is an
//! empty `main` so `cargo build`/`cargo test` over all bins stays happy.
fn main() {
    #[cfg(target_arch = "wasm32")]
    fretboard::audio::worker_entry();
}
