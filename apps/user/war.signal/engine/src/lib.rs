//! Task-67 Phase 2, item (1): the Signal **engine** as a wasm32-wasip2 reactor
//! component exporting `wart:signal/chat`. It owns the network (Signal protocol
//! over the task-66 host `wasi:tls` transport), the in-memory + on-disk protocol
//! state, and message history; a UI component drives it purely through the chat
//! contract (init / poll-events / send / history / state).
//!
//! The hard part is that each `poll-events` is a *separate* component-export call
//! and no guest code runs between calls. The background receive loop + websocket
//! keepalive run on [`wart_step_executor`] — a persistent, frame-stepped reactor
//! installed at `init` and advanced one non-blocking step per `poll-events` — so
//! those tasks survive across frames. See `engine.rs`.

wit_bindgen::generate!({
    world: "signal-engine",
    path: "wit",
    generate_all,
});

mod call;
mod engine;
mod persist;
mod store;

use exports::wart::signal::chat::{CallState, Contact, Event, Group, Guest, Message, Profile};

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
}

export!(Component);
