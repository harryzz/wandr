//! A drop-in `reqwest-websocket`: the real crate on native, an RFC6455 client over
//! the wandr-reqwest wasi:tls `TlsStream` on wasm32 (task 67). Cargo requires
//! one canonical source per dependency name across targets, hence cfg-dispatch.

#[cfg(target_arch = "wasm32")]
mod imp;
#[cfg(target_arch = "wasm32")]
pub use imp::*;

#[cfg(not(target_arch = "wasm32"))]
pub use reqwest_websocket::*;
