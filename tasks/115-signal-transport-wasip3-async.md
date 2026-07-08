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

## Blast radius — dual-serve guard (do NOT replace p2) ⚠️

Switching the host wasi:tls/sockets impl from **p2 to p3 as a *replacement*** would
break **every guest that still imports p2 — across languages**. The host provides
the impl; each guest imports a specific version; an imported interface **must be
satisfied at instantiation or the component fails to load** (importing ≠ using —
see below). Ground truth from scanning the built components (`wasm-tools
component wit`, **not** source grep — toolchains import networking implicitly):

| Component | Lang | Imports | Real user? |
|---|---|---|---|
| wandr.signal | Rust | `wasi:sockets@0.2.9` + `wasi:tls@0.2.0-draft` | yes — chat + calls |
| wandr.audio.player | Rust | `wasi:sockets@0.2.12` + `wasi:tls@0.2.0-draft` | yes — cover-art fetch |
| wandr.video.test | Rust | `wasi:sockets@0.2.9` (tcp/udp) | probe/test |
| **wandr.avalonia.demo** | **.NET/C#** | `wasi:sockets@0.2.0` (tcp/udp), no tls | **NO — dead import** |
| repros/call-live, signal-engine-smoke | Rust | sockets (+tls) | test |

**Importing ≠ using.** `wandr.avalonia.demo` is a UI-only demo, yet it imports the
**entire standard WASI P2 world** (`cli`/`clocks`/`filesystem`/`io`/`random`/
`sockets` + its real `wasi:canvas`). Reason: `componentize-dotnet` (NativeAOT-LLVM
+ WASI SDK) links the full `wasi:cli/command`-style world because the .NET
runtime/BCL *references* those syscalls — regardless of whether the app calls
them. **Managed / full-libc runtimes (.NET, TinyGo, anything on the full WASI SDK)
declare the whole world; lean custom-world Rust guests (the chrome guests import
just `wasi:cli/stderr`) do not.** The sockets import is a dead capability
declaration — but linking is *by declaration*, so the host must still satisfy it.

**Version fragmentation:** `wasi:sockets` appears at **0.2.0** (Avalonia), **0.2.9**
(Signal/video/call), **0.2.12** (audio.player); `wasi:tls` at `0.2.0-draft`
everywhere. The p2 host impl must keep satisfying that whole 0.2.x spread.

**The guard (mandatory):** the host must **dual-serve** — keep the full **p2**
`wasi:sockets`+`wasi:tls` surface (via `wasmtime_wasi`'s default linker, already
in place) and **add p3 additively** (like the wasi-canvas default-on pattern that
serves `my:skiko-gfx` *and* `wasi:canvas` at once). Add the p3 linker at **both**
instantiate sites (`app_loader.rs:268`, `:390`) plus `run_once.rs`/`standalone.rs`/
`lib.rs`. **Drop p2 only when the *last* importer — across all languages —
migrates** (and note most sockets imports are dead, so some may never "migrate",
they just need p2 satisfied). This keeps the migration incremental, not a
big-bang rebuild-everything.

**Method note:** audit consumers by scanning built components with `wasm-tools
component wit`, not source grep — a source-level check misses toolchain-implicit
imports (this is exactly how the .NET sockets dependency was nearly missed).

**Capability-hygiene aside (separate task):** a UI guest importing raw TCP/UDP it
never uses holds more authority than it should. If `componentize-dotnet` can emit
a **narrower world** (trim unused `sockets`/`filesystem`), it shrinks both the
attack surface and this blast radius. Worth checking — out of scope for 115.

## Migration signposting — `#[deprecated]` + `p3-async` feature + scan-checklist

Three complementary tools to drive the app-by-app migration off p2 (they cover
different gaps — use all three):

1. **`#[deprecated]` on wandr's own p2 crates (the Rust-guest nudge).** Mark the
   items we own so a guest still on p2 gets a compile-time warning:
   ```rust
   #[deprecated(since = "0.x", note = "p2 async is being retired — migrate to \
     native wasip3 async; see tasks/115")]
   pub fn init() { … }   // wandr-step-executor; + the wandr-reqwest p2-TLS path
   ```
   Fitting since `wandr-step-executor` is meant to be retired. **Limits:** Rust
   lint **only** (won't touch the .NET/Avalonia guest); can't annotate the
   wit-bindgen imports or the external `wasmtime-wasi-tls` crate — only our own
   wrappers/re-exports; it's a **warning, not enforcement** (doesn't remove p2,
   doesn't change the dual-serve requirement). Expect noise on guests that can't
   move yet (audio.player, signal-link) → `#[allow(deprecated)]` at those call
   sites until they migrate. Optionally `#![deny(deprecated)]` in CI *later* to
   hard-stop *new* p2 usage once the p3 path is proven.

2. **A `p3-async` feature flag on `wandr-reqwest` (the actual toggle).** Select
   the p2 vs p3 TLS/sockets backend per guest so migration is opt-in per app,
   not big-bang. This is the real switch; `#[deprecated]` just discourages the
   old side.

3. **The binary-scan checklist (source of truth for ALL consumers).** The
   blast-radius table above is authoritative — regenerate it with `wasm-tools
   component wit` per release. **Non-Rust consumers (Avalonia) never appear in
   Rust deprecation warnings**, so the scan is the only reliable "who still
   imports p2" list. p2 can be dropped only when this list is empty (modulo dead
   imports that just need p2 satisfied).

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
- **M1** — ✅ **DONE 2026-07-07 — END-TO-END VERIFIED, live.**
  `repros/wasi-tls-p3-spike/`: a native-async TLS-over-TCP **guest** (`resolve.await`
  / `connect.await` / handshake `.await`, **zero step-executor**) importing the real
  **0.3** contracts (`wasi:sockets@0.3.0`, `wasi:tls@0.3.0-draft`), driven by a
  wasmtime-46 **host** (links `wasmtime_wasi::p2`+`::p3` + `wasmtime_wasi_tls::p3`,
  `Store` on `call_async`). Live result: `run("example.com")` → **`HTTP/1.1 200 OK`**
  (real TLS 1.3 handshake + HTTPS GET, decrypted through the async receive pipe).
  Gotchas captured in the spike README: `generate_all`; don't force `async: true`;
  p3 modules behind a **`p3` cargo feature** + wasmtime **`component-model-async`**;
  **dual-serve is mandatory even for one guest** (the guest pulls p2 `wasi:cli`/
  `wasi:io@0.2.6` via Rust std → host links p2 AND p3 — a live confirmation of the
  blast-radius rule); **don't `drop` the write stream before reading** the response.
- **M2** — ✅ **DONE 2026-07-08 — DESKTOP-VERIFIED, live both-ways chat.**
  The real engine on the desktop p3-async host (released wasmtime 46.0.1 +
  wit-bindgen 0.59 guests), zero step-executor in the path: resumed the
  device-linked account from copied `/state`, connected both Signal websockets,
  ran the full storage.signal.org (contacts/groups) + cdn/cdn3 (avatars/
  attachments) sync, **sent AND received messages with a live peer**, held the
  socket through several 55 s keepalive cycles with no reconnects, and
  exercised supervised reconnect correctly (a session-ownership fight with the
  phone's auto-relaunched instance — server `4409 Connected elsewhere` — was
  diagnosed via the ws-process exit instrumentation, not a transport bug; the
  phone's bg-receipt alarm relaunches wandr.signal, so kill it when testing
  desktop with the same credentials). Deferred to M4: calls, battery, device.
  All wiring feature-flagged (defaults byte-identical):
  - **M2a spike** (`repros/cma-cross-call-spike`) — background task survives +
    advances BETWEEN export calls; quiescent unpumped; p2 apps coexist. KEY
    mechanics discovered: a sync-lifted export **cannot block on an
    async-lifted callee** (`CannotBlockSyncTask`) ⇒ the HOST starts the engine
    via a re-exported `engine-start.start: async func` (wac **compose** script,
    not plug — plug can't add exports); `call_async` does NOT advance unrelated
    pending host futures — **pumping (`run_concurrent`) is required wherever
    the host idles** (standalone nap-pump + a per-frame pump in the winit
    desktop loop); `wasm-tools validate` needs `-f cm-async`.
  - **Host** (`wandr-host` `p3-async` cargo feature) — CMA config flag, p2
    linked ASYNC (sync p2 host fns nest-panic under our driving runtime) + p3
    additive (dual-serve; tls p3 shares SignalTlsProvider), async instantiate,
    `exports:{default:async}` bindgen twins, `guest_call!` at all call sites,
    `engine-start` probe, nap/frame pumps.
  - **Transport** (`wandr-reqwest` `p3-async` feature) — `tls_p3.rs` (same API
    as tls.rs; cancellation-safe by construction via a dedicated reader task —
    select-drop torture: 26 dropped in-flight reads, zero byte loss) +
    `task.rs` executor seam (step-executor ⟷ CM-async); websocket passthrough;
    libsignal fork rebound to the seam (no direct executor dep).
  - **Engine** — `p3-async` feature: world `signal-engine-p3` (+`engine-start`),
    `init` no-op / host-started `start()`, `poll_events` = pure drain, sleeps/
    spawns via the seam, `link()` restructured on `join!`, watchdog kept
    (re-rationalized). UI byte-identical across flavors. `P3=1 build.sh`.
  - **✅ THE BLOCKER, RESOLVED — it was wit-bindgen 0.53, not wasmtime.** With
    0.53 guests, keep-alive protocols stall (the Signal WSS included:
    strace-verified the response bytes reach the host socket and never surface
    in the guest) and a second bindgen instance's `wait-for` wedges the loop;
    `Connection: close` flows pass (EOF flushes), which is why gates A–E
    masked it. First mis-attributed to wasmtime 46 (testing against main
    changed two variables at once); the clean matrix — 46.0.x+0.53 = stall,
    46.0.1+**0.59** = **ALL GATES PASS** incl. live WSS 101 + first
    provisioning frame from chat.signal.org, inline and across pumps. Fix =
    **wit-bindgen 0.59** guests (`spawn` → `spawn_local`); engine tree bumped;
    wandr-host pins bumped `=46.0.0`→`=46.0.1` (fs fix only; tested cell).
  - **NEXT:** phase-5 desktop functional gates on the real app (link +
    send/receive + keepalive-idle + watchdog + calls + dual-serve proof —
    needs the user for the QR link + visual checks).
- **M3** — ✅ **DONE 2026-07-08.** p3 is the DEFAULT Signal build; `cargo tree`
  proves **zero `wandr-step-executor` in its graph** (dep now optional, and the
  p2 poll bridge `tls.rs` is feature-gated out). Mechanics: `wandr-reqwest`
  grew an explicit `p2` feature (default-on → audio.player untouched) with a
  compile_error guard against none/both; the fork + engine + websocket declare
  `default-features = false` so the consuming app picks exactly one backend;
  engine `default = ["p3-async"]`, `p2-legacy` kept ONLY for pre-M4 device
  deploys (`P2=1 ./build.sh`; plain `--deploy` refuses the p3 flavor until M4).
  Remaining p2 consumers (not Signal-path): `wandr.audio.player` (real, its
  own migration later) and `repros/signal-link` (**stale — superseded by the
  engine, scheduled for deletion**; pins `wandr-reqwest/p2` explicitly).
  Desktop smoke on the new default composite: `engine-start` → `connected` ✓.
- **M4** — 🟡 **DEPLOYED 2026-07-08 — device runs the p3 stack; two user-assisted
  gates open.** Done + verified on the Pixel 2 XL:
  - p3-async host built (codegen-crates cleaned first — the AOT-corruption
    gotcha) and pushed via `run-hybrid-stack.sh --wandr-only`; rollback =
    `/data/local/tmp/wandr-host.p2.bak` + the engine's `p2-legacy` flavor.
  - The config-hash flip needed NO mass reinstall: the loader auto-re-precompiles
    on cache-key mismatch — all 6 chrome apps re-AOT'd on first launch and run
    (arbiter list + /proc + screenshot verified; zygote preload fails stale
    cwasm gracefully).
  - Signal p3 composite deployed (`./build.sh --deploy`, which now probes the
    device binary for the 0.3 marker); desktop `/state` synced back first
    (device pre-M4 state kept in `.signal-state-backups/pre-m4`). Engine came
    up `connected` over the native-async transport ON DEVICE (logcat).
  - **Idle/battery parity where it matters**: Signal in Background role =
    0–1% CPU (== p2 baseline); chrome overlays unchanged. KNOWN COST: Signal
    FOREGROUND idles at ~11.5% vs the ~1–3% fg baseline — the per-iteration
    `run_concurrent` nap-pump at the fg poll cadence (~21 sweeps/s after
    idle-decay, ~0.5%/sweep on the A73). Tuning follow-up: rate-limit fg pumps
    when the guest is idle — MUST be done together with the calls gate (the
    in-call 10 ms engine tick is driven ONLY by pumps on p3; bg-tick calls do
    not advance it).
  - **Open (user-assisted): (a)** live receive/send on device with a peer;
    **(b)** a call on the p3 flavor (tick-cadence risk above) — until (b)
    passes, calls on device should be treated as untested on p3.
- **M5 (optional)** — shape (B): `subscribe() -> stream<event>`.

## M1 readiness (assessed 2026-07-07) — startable, gate = async codegen

**Confirmed ready ✅:**
- Runtime = crates.io **wasmtime 46** (no `[patch]` redirect — `external/wasmtime`
  submodule is a read-only source ref at v45, NOT a build input, so no build-time
  skew). p3 wasi-tls host API exists: `wasi_tls::p3::add_to_linker`
  (`external/wasmtime/crates/wasi-tls/src/p3/mod.rs`, present since v45 → in 46).
- Spike harness in-tree: `repros/wstd-wasitls-spike` (the M1 starting point) +
  `repros/wasi-tls-probe` / `wasi-tls-runner`.
- `wit-bindgen 0.53` in the Signal engine + `wandr-reqwest`. Desktop dev loop
  exists (task 101).

**Verify FIRST (M1 opening moves — this is what the spike proves):**
1. ✅ **GATE CLEARED 2026-07-07 — full async chain VERIFIED end-to-end.** On
   wandr's exact toolchain (wit-bindgen 0.53.1, rustc 1.95, wasmtime 46.0.0,
   wasm-tools 1.245.1): (a) **codegen** — `--async all` emits `async fn` +
   `future`/`stream` (imports+exports); (b) **compile+link** — an `async fn run`
   export via the `generate!` **macro** builds for `wasm32-wasip2` and links
   `wit_bindgen::rt::async_support` (`async` is a default feature); (c)
   **component** — valid; (d) **run** — `wasmtime run -W component-model-async=y
   --invoke 'run(41)'` drove the async export to completion → `42`. **No
   wit-bindgen bump needed; async needed only `-W component-model-async=y`, NOT
   `-Sp3`** (the async ABI is orthogonal to importing WASI 0.3). Evidence +
   reproduce: `repros/wit-bindgen-async-probe/`. Still M1's job: the p3
   `wasi:tls`/`wasi:sockets` *transport* wiring — this probe covered the
   toolchain/async-ABI gate, not the network edge.
2. **p3 wit async-shape unverified** — `p3::add_to_linker` exists, but the p3
   `world.wit` read was a bindgen helper stub; confirm the real p3 `wit/deps` is
   stream/future-shaped before writing the guest.
3. **Spike host links p3, not p2** — wandr-host links p2 today; the M1 host wires
   `wasi_tls::p3::add_to_linker` + runs the `Store` on `call_async`.
4. **Bump `external/wasmtime` v45→46** — only to read the *exact* p3 API the build
   links (not a build blocker; it's a source reference).

**Verdict:** green to start — the two hard prerequisites (p3 host API + spike
harness) are in place, and **#1 (the async toolchain) — the true gate — is now
CLEARED end-to-end**: codegen → compile → link → run all verified on the exact
toolchain (`repros/wit-bindgen-async-probe/`). Remaining M1 work is the p3
`wasi:tls`/`wasi:sockets` transport wiring, not the async toolchain.

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
