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
});

mod engine;
mod persist;
mod store;

use exports::wart::signal::chat::{Contact, Event, Guest, Message};

struct Component;

impl Guest for Component {
    fn init() {
        engine::init();
    }

    fn poll_events() -> Vec<Event> {
        engine::poll_events()
    }

    fn send(text: String) -> Result<(), String> {
        engine::send(text)
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

    fn state() -> String {
        engine::state()
    }
}

export!(Component);
