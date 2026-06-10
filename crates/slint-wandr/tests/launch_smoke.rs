//! Compile-only smoke for the `launch!` expansion (M1): proves the inline
//! export-world `generate!`, the Guest impls and the export wiring all
//! type-check in a consumer crate. Checked via
//! `cargo check --tests --target wasm32-wasip2` — never run (the real
//! exercise is the wandr.slint.test guest on device, M4).
#![cfg(target_arch = "wasm32")]

slint_wandr::launch!(|| ());
