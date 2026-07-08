//! Task 115 — the executor seam. The Signal engine + libsignal fork call ONLY
//! these for timers and detached background tasks, so flipping the `p3-async`
//! feature swaps the whole engine between the step-executor reactor (p2) and
//! native CM-async (p3) without touching call sites.

use std::future::Future;
use std::time::Duration;

/// Async sleep. p2: parks on the step-executor's `wasi:io/poll` reactor.
/// p3: `wasi:clocks@0.3` monotonic `wait-for` — a native async host call
/// suspended/resumed by the host event loop.
#[cfg(feature = "p2")]
pub async fn sleep(d: Duration) {
    wandr_step_executor::sleep(d).await
}
#[cfg(feature = "p3-async")]
pub async fn sleep(d: Duration) {
    crate::p3::wasi::clocks::monotonic_clock::wait_for(d.as_nanos() as u64).await
}

/// Spawn a detached background task. p2: step-executor (`spawn().detach()` —
/// the returned `async_task::Task` cancels on drop). p3: the CM-async
/// executor; wit-bindgen's `spawn` returns no handle (detached by design), so
/// callers needing cancellation wrap the future with a shared flag.
#[cfg(feature = "p2")]
pub fn spawn(fut: impl Future<Output = ()> + 'static) {
    wandr_step_executor::spawn(fut).detach()
}
#[cfg(feature = "p3-async")]
pub fn spawn(fut: impl Future<Output = ()> + 'static) {
    wit_bindgen::rt::async_support::spawn_local(fut)
}
