//! Persistent, frame-stepped single-threaded async executor for wasm32-wasip2.
//!
//! Usage (engine side):
//! ```ignore
//! wandr_step_executor::init();            // once, before any AsyncPollable/spawn
//! wandr_step_executor::spawn(receive_loop());
//! // ...later, once per `chat.poll-events`:
//! wandr_step_executor::step();            // non-blocking; pumps the network
//! ```
//!
//! The reactor is created by [`init`] and never torn down, so pollables and
//! tasks registered during one [`step`] survive to the next. [`step`] advances
//! every runnable that is ready and does a *non-blocking* `wasi:io/poll` check
//! (the "ready pollable" 0-duration trick, like wstd's `nonblock_check_pollables`)
//! to wake futures whose I/O has become ready — but never blocks the frame. When
//! the only remaining work is parked on not-yet-ready pollables, [`step`]
//! returns and the guest yields control back to the host until the next call.
//!
//! The reactor bookkeeping (Registration / AsyncPollable / WaitFor / Reactor) is
//! modeled on `wstd` 0.6.6's `runtime/reactor.rs`; the lifecycle (persistent
//! thread-local instead of per-`block_on`) and the non-blocking [`step`] differ.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_task::{Runnable, Task};
use slab::Slab;
use wasip2::io::poll::Pollable;

thread_local! {
    /// The single, persistent reactor. Installed by [`init`], never cleared.
    static REACTOR: RefCell<Option<Reactor>> = const { RefCell::new(None) };
}

/// Install the persistent reactor. Idempotent — safe to call from every
/// component export entry point.
pub fn init() {
    REACTOR.with(|r| {
        let mut slot = r.borrow_mut();
        if slot.is_none() {
            *slot = Some(Reactor::new());
        }
    });
}

fn current() -> Reactor {
    REACTOR.with(|r| {
        r.borrow()
            .as_ref()
            .expect("wandr_step_executor: call init() before use")
            .clone()
    })
}

/// Spawn a `'static` future onto the persistent reactor. The returned [`Task`]
/// can be `.await`ed or `.detach()`ed (fire-and-forget background work, e.g. the
/// websocket keepalive).
pub fn spawn<F, T>(fut: F) -> Task<T>
where
    F: Future<Output = T> + 'static,
    T: 'static,
{
    current().spawn(fut)
}

/// Advance the reactor without blocking. Runs every ready task, then
/// non-blocking-checks pollables to wake any whose I/O is ready, repeating until
/// the only remaining work is parked on not-yet-ready pollables (or a safety cap
/// is hit). Call once per export invocation.
pub fn step() {
    // Guard against a misbehaving self-waking future (wake + Pending with no
    // pollable) spinning the frame forever. A well-behaved transport parks on
    // real pollables and never approaches this.
    const MAX_RUNS: usize = 100_000;

    let reactor = current();
    let mut runs = 0usize;
    loop {
        match reactor.pop_ready_list() {
            // Nothing runnable and nothing parked: fully quiescent.
            None if reactor.pending_pollables_is_empty() => break,
            // Nothing runnable, but futures are parked on pollables. One
            // non-blocking check; if it wakes nothing we can't progress without
            // blocking, so yield the frame.
            None => {
                reactor.nonblock_check_pollables();
                if reactor.ready_list_is_empty() {
                    break;
                }
            },
            Some(runnable) => {
                let woke = runnable.run();
                runs += 1;
                if runs >= MAX_RUNS {
                    break;
                }
                if woke || !reactor.ready_list_is_empty() {
                    reactor.nonblock_check_pollables();
                }
            },
        }
    }
}

/// Sleep for `duration` by parking on a monotonic-clock pollable. Replaces
/// `wstd::task::sleep` (which needs an active `block_on`) for the keepalive timer.
pub async fn sleep(duration: std::time::Duration) {
    let nanos = duration.as_nanos().min(u64::MAX as u128) as u64;
    let pollable = wasip2::clocks::monotonic_clock::subscribe_duration(nanos);
    AsyncPollable::new(pollable).wait_for().await;
}

// ---- reactor internals (modeled on wstd 0.6.6 runtime/reactor.rs) ----------

/// Index into the reactor's `Slab<Pollable>`.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
struct EventKey(usize);

/// A live reference to a reactor-owned pollable; dropping it deregisters the
/// pollable from the reactor.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Registration {
    key: EventKey,
}

impl Drop for Registration {
    fn drop(&mut self) {
        current().deregister_event(self.key)
    }
}

/// A reference-counted [`Registration`]. Clone it freely; build as many
/// [`WaitFor`] futures from it as needed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AsyncPollable(Arc<Registration>);

impl AsyncPollable {
    /// Register a WASI `Pollable` with the persistent reactor.
    pub fn new(pollable: Pollable) -> Self {
        current().schedule(pollable)
    }

    /// A future that resolves once the pollable is ready.
    pub fn wait_for(&self) -> WaitFor {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        WaitFor {
            waitee: Waitee {
                pollable: self.clone(),
                unique,
            },
            needs_deregistration: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct Waitee {
    pollable: AsyncPollable,
    unique: u64,
}

/// Future returned by [`AsyncPollable::wait_for`].
#[must_use = "futures do nothing unless polled or .awaited"]
#[derive(Debug)]
pub struct WaitFor {
    waitee: Waitee,
    needs_deregistration: bool,
}

impl Future for WaitFor {
    type Output = ();
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let reactor = current();
        if reactor.ready(&self.as_ref().waitee, cx.waker()) {
            std::task::Poll::Ready(())
        } else {
            self.as_mut().needs_deregistration = true;
            std::task::Poll::Pending
        }
    }
}

impl Drop for WaitFor {
    fn drop(&mut self) {
        if self.needs_deregistration {
            current().deregister_waitee(&self.waitee)
        }
    }
}

#[derive(Debug, Clone)]
struct Reactor {
    inner: Arc<InnerReactor>,
}

#[derive(Debug)]
struct InnerReactor {
    pollables: Mutex<Slab<Pollable>>,
    wakers: Mutex<HashMap<Waitee, std::task::Waker>>,
    ready_list: Mutex<VecDeque<Runnable>>,
}

impl Reactor {
    fn new() -> Self {
        Self {
            inner: Arc::new(InnerReactor {
                pollables: Mutex::new(Slab::new()),
                wakers: Mutex::new(HashMap::new()),
                ready_list: Mutex::new(VecDeque::new()),
            }),
        }
    }

    fn pending_pollables_is_empty(&self) -> bool {
        self.inner.wakers.lock().unwrap().is_empty()
    }

    /// Without blocking, wake any futures whose pollables are ready. Appends a
    /// always-ready 0-duration timer so `poll` returns immediately, then erases
    /// it from the result.
    fn nonblock_check_pollables(&self) {
        if self.pending_pollables_is_empty() {
            return;
        }
        use std::sync::LazyLock;
        static READY_POLLABLE: LazyLock<Pollable> =
            LazyLock::new(|| wasip2::clocks::monotonic_clock::subscribe_duration(0));

        let wakers = self.inner.wakers.lock().unwrap();
        let pollables = self.inner.pollables.lock().unwrap();

        let mut indexed_wakers = Vec::with_capacity(wakers.len());
        let mut targets = Vec::with_capacity(wakers.len() + 1);
        for (waitee, waker) in wakers.iter() {
            indexed_wakers.push(waker);
            targets.push(&pollables[waitee.pollable.0.key.0]);
        }
        let ready_index = targets.len();
        targets.push(&READY_POLLABLE);

        let mut ready_list = wasip2::io::poll::poll(&targets);
        ready_list.retain(|e| *e != ready_index as u32);

        for index in ready_list {
            indexed_wakers[index as usize].wake_by_ref();
        }
    }

    fn schedule(&self, pollable: Pollable) -> AsyncPollable {
        let mut pollables = self.inner.pollables.lock().unwrap();
        let key = EventKey(pollables.insert(pollable));
        AsyncPollable(Arc::new(Registration { key }))
    }

    fn deregister_event(&self, key: EventKey) {
        self.inner.pollables.lock().unwrap().remove(key.0);
    }

    fn deregister_waitee(&self, waitee: &Waitee) {
        self.inner.wakers.lock().unwrap().remove(waitee);
    }

    fn ready(&self, waitee: &Waitee, waker: &std::task::Waker) -> bool {
        let ready = self
            .inner
            .pollables
            .lock()
            .unwrap()
            .get(waitee.pollable.0.key.0)
            .expect("only live EventKey can be checked for readiness")
            .ready();
        if !ready {
            self.inner
                .wakers
                .lock()
                .unwrap()
                .insert(waitee.clone(), waker.clone());
        }
        ready
    }

    fn spawn<F, T>(&self, fut: F) -> Task<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        let this = self.clone();
        let schedule =
            move |runnable| this.inner.ready_list.lock().unwrap().push_back(runnable);
        // SAFETY: identical to async_task::spawn_local — the schedule closure is
        // not Send/Sync because Runnable isn't, which is sound on wasm32-wasip2
        // (always single-threaded).
        #[allow(unsafe_code)]
        let (runnable, task) = unsafe { async_task::spawn_unchecked(fut, schedule) };
        self.inner.ready_list.lock().unwrap().push_back(runnable);
        task
    }

    fn pop_ready_list(&self) -> Option<Runnable> {
        self.inner.ready_list.lock().unwrap().pop_front()
    }

    fn ready_list_is_empty(&self) -> bool {
        self.inner.ready_list.lock().unwrap().is_empty()
    }
}
