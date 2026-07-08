//! UI-like component (task 115 M2a spike): a plain SYNC `run-frame` export that
//! drains the engine's `poll` (sync→sync across the composed boundary) each
//! frame. It does NOT call `start` — a sync-lifted export may not block on an
//! async-lifted callee (wasmtime trap `CannotBlockSyncTask`); the HOST starts
//! the engine. The `-import` filter keeps the unused `start` binding sync so
//! this stays a fully-sync component (no async ABI section).
wit_bindgen::generate!({
    world: "ui",
    path: "wit",
    generate_all,
    async: ["-import:demo:cma/chat#start"],
});

struct Ui;

impl Guest for Ui {
    fn run_frame() -> u32 {
        demo::cma::chat::poll()
    }
}

export!(Ui);
