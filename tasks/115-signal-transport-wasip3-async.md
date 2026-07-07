# Task 115 — Retire `wandr-step-executor` on the Signal transport (wasip3 native async)

> Scoped 2026-07-07. 🔲 not started — a **sketch / go-no-go**. **M0 resolved
> 2026-07-07: the 0.3 async `wasi:tls`+`wasi:sockets` host impls are already in
> the pinned wasmtime 46** (wandr just links the p2 variant) — so this is
> reachable now on the runtime we ship, gated only on RC-level API stability
> (see "M0"). Outgrowth of the wasip3 async
> analysis (`docs/shared-runtime-and-app-size.md` §"Composed components & the
> wasip3 shared event loop"). Goal: replace the hand-rolled frame-stepped async
> reactor on the **Signal transport** with **native Component-Model async**
> (WASI 0.3 / wasip3), now that wandr runs wasmtime 46 with CM-async default-on.
> Rust-guest only; nothing here touches Kotlin/Compose (KT-64568).

## Why this, why now

The Signal engine is the guest that *most* exercises `wandr-step-executor`, and
wasip3 removes the exact limitation the executor works around. wandr is already
on **wasmtime 46, CM-async default-on** (`runtime/wandr-host/Cargo.toml:18`), so
the runtime side is ready. Best first candidate for retiring the executor.

## What exists today (three pieces, one problem)

The problem being worked around: **each `chat.poll-events` is a *separate*
component-export call**, so async state must survive across calls (wasip2 has no
native way).

1. **`crates/wandr-step-executor`** — persistent, frame-stepped single-threaded
   reactor. `init()` once → `spawn(fut)` → `step()` per `poll-events`
   (non-blocking `wasi:io/poll` "ready-pollable 0-duration" check; never blocks
   the frame). Modeled on wstd 0.6.6's `runtime/reactor.rs`.
2. **`crates/wandr-reqwest/src/tls.rs`** — bridges `wasi:io/poll.pollable` →
   the executor's `AsyncPollable` (`schedule()`), so the reqwest-shim TLS byte
   stream is driven by `step()`.
3. **Signal engine** (`apps/user/wandr.signal/engine/{lib,engine}.rs`) — `init`
   spawns the receive+send+keepalive loop on the executor; each `poll_events()`
   export runs one `step()` and drains produced events.

Shape today: **host pulls `poll-events` per frame → guest advances its reactor
one non-blocking step → yields.**

## Why wasip3 removes all three

A **host-owned event loop** + `async fn` exports make async *natively span export
calls* — the exact constraint the executor fakes. The runtime holds the suspended
task; no manual `step()`, no persistent hand-rolled reactor, no non-blocking-poll
trick.

## The migration, file by file

| Piece | Today | After |
|---|---|---|
| `wandr-step-executor` | `init`/`spawn`/`step` reactor | **deleted** for this guest (keep only for not-yet-migrated guests) |
| `wandr-reqwest/tls.rs` | pollable→`AsyncPollable` bridge, `step()`-driven | **await the async stream directly**; delete the poll bridge |
| Signal engine | `init` spawns loop; `poll_events` = one `step()` + drain | receive/send/keepalive is a spawned **`async` task** that `.await`s I/O |
| `chat` WIT | `poll-events() -> list<event>` (host pulls) | **(A)** keep `poll-events`, make it `async`; or **(B)** `subscribe() -> stream<event>` (no polling) |
| Host | drives per-frame `poll-events` | run the `Store` on the async executor (`call_async`); host-impl the async `chat` |

**Two shapes:**
- **(A) Minimal** — keep the `poll-events` contract, make the export `async fn`;
  the background task awaits natively instead of being `step()`-ed. Smallest diff,
  preserves the UI's poll cadence. **Do this first.**
- **(B) Streaming** — replace polling with `subscribe() -> stream<event>`; the UI
  awaits the stream. Cleaner, but changes the guest↔host contract + the UI's
  consumption model. Consider after (A) proves out.

## M0 — RESOLVED 2026-07-07: the 0.3 async impls are already in wandr's wasmtime 46 ✅

The transport rides `wasi:tls`, and wandr currently *links* the p2 variant
(`wasmtime-wasi-tls = "46"` serving `wasi:tls@0.2.0-draft`,
`runtime/wandr-host/Cargo.toml`). The open question was whether **0.3-async**
networking (`wasi:sockets`/`wasi:tls` in async form) even exists in the pinned
runtime. **It does** — verified against wasmtime upstream:

- **`wasi:tls` p3 (async) landed** — PR **#12834 "feat(p3): implement wasi:tls"**
  (merged 2026-03-30), **#12896** "same view types for both p2 & p3" (2026-03-31),
  **#12780** merged the crates into one with **feature flags**. All merged before
  **v46.0.0** (2026-06-22). So the `wasmtime-wasi-tls 46.0.0` crate wandr already
  depends on **ships both p2 and p3** — wandr just links p2 today. (The earlier
  `feat(p3)` PR #12174 was closed *unmerged*; #12834 is the one that landed.)
  Client-side TLS (all Signal needs) is covered.
- **`wasi:sockets` 0.3 async** — present in wasmtime's P3 support (exercised in
  wasmCloud fixtures), but **RC-status**: thin external docs, API names may still
  shift.

**So M0 is no longer "is it available?" (yes, in the runtime we ship) but a
narrower wiring step:** switch `wandr-host` from the **p2** to the **p3**
`wasmtime-wasi-tls` variant (+ p3 sockets) and expose the async `chat` — accepting
**RC-level API churn** while wasi:tls (still a phase-2 *proposal*, not ratified
core WASI 0.3) and wasi:sockets 0.3 settle.

- **Full clean retirement** (guest awaits real async streams end-to-end) is
  **reachable now** on the pinned wasmtime 46 — the runtime is not the blocker.
- **Risk** = pre-stable API: pin exact wasmtime patch, expect churn, keep a p2
  fallback path until the interfaces freeze. Prove it in M1 (spike) before
  ripping out the executor.

Evidence: wasmtime PRs #12834 / #12896 / #12780; issue #12102 (phase-2 tracker).

## Preserve these semantics (don't regress)

- **Device-suspend / battery** — today the loop only advances while the UI polls
  (`engine.rs:209`). Native async changes *who drives* it; re-verify the engine
  stays quiescent when the UI isn't looking.
- **Single-thread cooperative** — the async task must `.await` at I/O points; no
  CPU spin (would block the shared loop, incl. composed deps).
- **Rendering untouched** — this migration is the I/O/transport task only; the
  frame path stays host-pull / synchronous / frame-paced.

## Milestones (each a kill-gate)

- **M0** — ✅ **DONE 2026-07-07:** 0.3 async `wasi:tls` (PR #12834) + `wasi:sockets`
  are in the pinned wasmtime 46; scope = **full-retire**, gated on RC-API stability
  (switch `wandr-host` p2→p3 `wasmtime-wasi-tls`). See the "M0" section above.
- **M1** — spike: async-restructured transport in a `repros/wstd-wasitls-spike`
  analog (async export + spawn; no `step()`), desktop dev loop.
- **M2** — shape (A) in the real engine: `async` `poll-events`, delete the
  `step()`/reactor path; send+receive+keepalive on the host loop; desktop.
- **M3** — delete `wandr-step-executor` from the Signal build + the
  `wandr-reqwest` poll bridge; confirm no other Signal-path consumer remains.
- **M4** — Pixel 2 XL: messages send/receive + keepalive survive UI idle; no
  dropped events vs the `step()` baseline; battery behavior unchanged.
- **M5 (optional)** — shape (B): `subscribe() -> stream<event>`.

## Scope / non-goals

- **Rust guest only.** Signal engine is Rust → eligible. Kotlin/Compose guests
  keep the reactor/step-executor model (KT-64568) — out of scope.
- **Not a contract redesign** (unless M5): (A) keeps the `chat` shape.
- **Not the render path.** Frame-driven rendering is unchanged.
- Other step-executor users (`wandr.audio.player`, `repros/signal-link`) are
  separate migrations — do them only after this proves the pattern.

## Watch

- 0.3 `wasi:tls`/`wasi:sockets` async status in wasmtime (the M0 gate).
- `wit-bindgen` async codegen (`async fn`, `stream<T>`, `async-spawn`).
- Keep the `wandr-step-executor` crate until ALL guests migrate — deleting it is
  the very last step, not this task.
