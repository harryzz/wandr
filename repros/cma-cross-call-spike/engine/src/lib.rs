//! Engine-like component (task 115 M2a spike). `start` (async-lifted, called by
//! the HOST via `call_async`) spawns a forever ticker on the native CM-async
//! executor — the pattern that replaces `wandr_step_executor::spawn(run()).detach()`.
//! `poll` is a sync drain the UI calls per frame, like the real `poll-events`
//! after `step()` is deleted.
wit_bindgen::generate!({ world: "engine", path: "wit", generate_all });

use std::sync::atomic::{AtomicU32, Ordering};
use wit_bindgen::rt::async_support::spawn;

static TICKS: AtomicU32 = AtomicU32::new(0);

struct EngineC;

impl exports::demo::cma::chat::Guest for EngineC {
    async fn start() {
        spawn(async {
            loop {
                // 100 ms tick — models the engine's idle 200 ms / in-call 10 ms cadence.
                crate::wasi::clocks::monotonic_clock::wait_for(100_000_000).await;
                TICKS.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    fn poll() -> u32 {
        TICKS.load(Ordering::Relaxed)
    }
}

export!(EngineC);
