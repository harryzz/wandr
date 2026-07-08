//! Task-67 Phase 2, item (1): the Signal **engine** as a wasm32-wasip2 reactor
//! component exporting `wandr:signal/chat`. It owns the network (Signal protocol
//! over the task-66 host `wasi:tls` transport), the in-memory + on-disk protocol
//! state, and message history; a UI component drives it purely through the chat
//! contract (init / poll-events / send / history / state).
//!
//! The hard part is that each `poll-events` is a *separate* component-export call
//! and no guest code runs between calls. The background receive loop + websocket
//! keepalive run on [`wandr_step_executor`] — a persistent, frame-stepped reactor
//! installed at `init` and advanced one non-blocking step per `poll-events` — so
//! those tasks survive across frames. See `engine.rs`.

// Task 115 — two flavors of the same engine. Default (p2): the step-executor
// model described above. `p3-async`: native CM-async — the world adds the
// host-called `engine-start.start` async export (the spawn point; the reactor
// and its per-frame `step()` are gone), and the transport rides wandr-reqwest's
// p3 backend (WASI 0.3 sockets/tls streams). See tasks/115.
#[cfg(feature = "p2-legacy")]
wit_bindgen::generate!({
    world: "signal-engine",
    path: "wit",
    generate_all,
});
#[cfg(feature = "p3-async")]
wit_bindgen::generate!({
    world: "signal-engine-p3",
    path: "wit",
    generate_all,
});

mod call;
mod engine;
mod persist;
mod store;

use exports::wandr::signal::chat::{CallState, Contact, Event, Group, Guest, Message, Profile};

struct Component;

impl Guest for Component {
    fn init() {
        engine::init();
    }

    fn poll_events() -> Vec<Event> {
        engine::poll_events()
    }

    fn send(thread: String, text: String) -> Result<(), String> {
        engine::send(thread, text)
    }

    fn mark_read(thread: String) {
        engine::mark_read(thread);
    }

    fn history() -> Vec<Message> {
        engine::history()
    }

    fn contacts() -> Vec<Contact> {
        engine::contacts()
    }

    fn sync_contacts() {
        engine::sync_contacts();
    }

    fn groups() -> Vec<Group> {
        engine::groups()
    }

    fn sync_groups() {
        engine::sync_groups();
    }

    fn state() -> String {
        engine::state()
    }

    fn account_id() -> String {
        engine::account_id()
    }

    fn my_profile() -> Profile {
        engine::my_profile()
    }

    fn sync_profile() {
        engine::sync_profile();
    }

    fn place_call(thread: String) -> Result<(), String> {
        engine::place_call(thread)
    }

    fn accept_call() {
        engine::accept_call();
    }

    fn hangup_call() {
        engine::hangup_call();
    }

    fn call_status() -> CallState {
        engine::call_status()
    }

    fn call_peer() -> String {
        engine::call_peer()
    }

    fn set_video(enabled: bool) {
        engine::set_video(enabled);
    }

    fn set_video_layout(rx: u32, ry: u32, rw: u32, rh: u32, px: u32, py: u32, pw: u32, ph: u32) {
        engine::set_video_layout((rx, ry, rw, rh), (px, py, pw, ph));
    }

    fn video_status() -> (bool, bool) {
        engine::video_status()
    }
}

/// Task 115 (p3 flavor) — the host-called async spawn point; see engine::start.
#[cfg(feature = "p3-async")]
impl exports::wandr::signal::engine_start::Guest for Component {
    async fn start() {
        engine::start().await;
    }
}

export!(Component);
