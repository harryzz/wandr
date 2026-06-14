//! A drop-in `reqwest` for our libsignal-service-rs fork. On **wasm32** it is a
//! subset of the reqwest API implemented over task-66 wasi:tls + the wstd
//! (wasi:io/poll) async executor (task 67 — Signal client as a wasm guest). On
//! **native** targets it transparently re-exports the real `reqwest`, so the fork
//! still builds + tests on desktop. Cargo requires one canonical source per
//! dependency name across targets, hence this cfg-dispatch rather than a
//! per-target dependency swap.

#![cfg_attr(not(target_arch = "wasm32"), allow(unused))]

// ---- wasm32: wasi:tls implementation ----

#[cfg(target_arch = "wasm32")]
wit_bindgen::generate!({
    world: "shim",
    path: "wit",
    features: ["tls"],
    generate_all,
});

#[cfg(target_arch = "wasm32")]
pub mod tls;
#[cfg(target_arch = "wasm32")]
mod http1;
#[cfg(target_arch = "wasm32")]
pub mod multipart;
#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
pub use api::*;

/// Host CSPRNG bytes (via wasi:random). Exposed for the websocket shim's frame
/// masking keys + Sec-WebSocket-Key.
#[cfg(target_arch = "wasm32")]
pub fn random_bytes(len: u64) -> Vec<u8> {
    crate::wasi::random::random::get_random_bytes(len)
}

// ---- native: passthrough to the real reqwest ----

#[cfg(not(target_arch = "wasm32"))]
pub use reqwest::*;
