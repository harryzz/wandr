# Task 115 — Retire `wandr-step-executor` on the Signal transport (wasip3 native async)

> Scoped 2026-07-07. 🔲 not started — a **sketch / go-no-go**, gated on 0.3
> networking availability (see "The real gate"). Outgrowth of the wasip3 async
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

## The real gate ⚠️ (not purely guest-side)

The transport rides `wasi:tls`, and the host currently serves
**`wasi:tls@0.2.0-draft`** (`wasmtime-wasi-tls = "46"`, `runtime/wandr-host/Cargo.toml`).
Native async end-to-end wants **0.3-async networking** (`wasi:sockets` / `wasi:tls`
in async form). Decision gate **before committing**:

- **0.3 async `wasi:tls`/`wasi:sockets` available in wasmtime 46** → full clean
  retirement (guest awaits real async streams). ✅ preferred.
- **Not yet** → can still make the **guest structure** async (async export +
  spawn, removing the manual `step()` loop), but the bottom I/O edge may still
  bridge a 0.2 pollable until async transport lands. Partial win.

**M0 = confirm what 0.3 networking wasmtime 46 actually ships.** This decides
whether the task is "full retire" or "restructure now, finish when transport
lands."

## Preserve these semantics (don't regress)

- **Device-suspend / battery** — today the loop only advances while the UI polls
  (`engine.rs:209`). Native async changes *who drives* it; re-verify the engine
  stays quiescent when the UI isn't looking.
- **Single-thread cooperative** — the async task must `.await` at I/O points; no
  CPU spin (would block the shared loop, incl. composed deps).
- **Rendering untouched** — this migration is the I/O/transport task only; the
  frame path stays host-pull / synchronous / frame-paced.

## Milestones (each a kill-gate)

- **M0** — confirm 0.3 async networking surface in wasmtime 46 (full-retire vs
  restructure-only). Decides scope.
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
