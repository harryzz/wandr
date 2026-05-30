# Task 67 — Simple Signal client (text-only), as a wasm guest

## ▶ RESUME HERE (fresh session, 2026-05-30)

**Phase 0 + Phase 1 DONE.** A working text Signal client exists as a wasm32-wasip2
guest, **zero crypto/networking compiled in** (all transport over host `wasi:tls`):
link-as-secondary-device, receive+decrypt, **send**, and pure-Rust file
persistence (resume w/o re-link + message history). Verified on desktop AND
on-device (Pixel 2 XL via wart-host).

- **Working code:** `repros/signal-link/` — the full client as a `wasi:cli/command`
  (`src/main.rs` link/receive/send, `src/store.rs` ProtocolStore, `src/persist.rs`
  WASI-fs persistence). Forked transport: `external/libsignal-service-rs/`
  (+`wart-wasi-shims/` = reqwest/reqwest-websocket over wasi:tls). Foundation
  spike: `repros/wstd-wasitls-spike/`. Desktop runner (Signal CA + `/state`
  preopen): `repros/wasi-tls-runner/`.
- **Build:** `cd repros/signal-link && PROTOC=~/tools/protoc/bin/protoc cargo build
  --target wasm32-wasip2 --release`. Run: `repros/wasi-tls-runner/.../wasi-tls-runner
  <wasm> <state-dir>`. **QR:** render a PNG from the printed `sgnl://` URL
  (`qrencode -s 12 -m 4`); terminal QRs don't scan; scan fast (~30-60s window).
- **NEXT = Phase 2** (architecture decided; see "Phase 2 architecture" + "Next
  action" below): split into WIT-decoupled `signal-engine` (exports
  `wart:signal/chat`) + `signal-ui` (dioxus over skiko-gfx). The engine needs a
  small **persistent step-executor** (the gating finding below). Start with the
  engine.
- Detail/gotchas: `[[project_signal_wasip2_transport_swap]]`,
  `[[project_signal_client_architecture]]`. Housekeeping: prune orphaned "wart"
  linked devices in Signal (in-memory test runs left several; persistence ends it).

---

🟡 Phase 0 narrative below is historical; resume from the block above.

## Goal & scope (v1 = "simple")

A working Signal Messenger client as a wart app. Deliberately minimal:
- **Text chat only** — no voice/video, no media/attachments, no groups, no
  stories. 1:1 text conversations.
- **Link as a secondary device** (scan a QR from an existing Signal phone) — not
  primary registration. Avoids phone-number/captcha/SMS provisioning.
- **Receive-while-foreground.** Background delivery is explicitly out of scope for
  v1 (see "Background" below).

## Architecture (corrected 2026-05-30 — guest-side)

**The Signal client is a single `wasm32-wasip2` guest component. All client logic
lives in the guest.** It is just another wart app: it uses the *generic* host
capabilities every app shares — `skiko-gfx` to render, **`wasi:tls`/`wasi:sockets`
(task 66) to reach Signal's servers**, WASI fs (or a future generic host store
capability) to persist Signal-protocol state + messages.

> **Rejected: the native-host-daemon design.** An earlier scaffold put the
> connection in a bespoke native daemon (`wart-signal`, presage-based). That was
> discarded. **App logic belongs in the app**; the host stays generic. Building a
> per-app native host extension doesn't scale and breaks the host/guest boundary
> the whole project is built on. **Task 66 wired `wasi:tls` precisely so a guest
> can do this itself.** Do not reintroduce a per-app host service without explicit
> user direction. See [[project_signal_client_architecture]].

Components:
- **Signal guest** — one `wasm32-wasip2` component. Signal protocol (`libsignal` /
  `libsignal-service-rs`) + dioxus UI in the same guest. Networking goes through a
  **wasip2 transport implemented over task-66 `wasi:tls`** (the working raw
  reference is `repros/wasi-tls-probe`). Foreground-only; freely frozen/killed
  when backgrounded.
- **Host** — unchanged from task 66. Grants network (`inherit_network` +
  `allow_ip_name_lookup`; default deny-all) and the Signal CA via the custom
  `TlsProvider`. No Signal-specific host code.

### Background (why foreground-only for v1)
Backgrounding **freezes the guest** — on-demand rendering (task 64) gates *all*
guest calls (`standalone.rs:890`). A guest-held websocket therefore can't keep
alive in the background. v1 simply doesn't deliver in the background. If/when we
want it, the fix is a **generic** host capability available to *all* apps (e.g. a
host-managed keep-alive socket / background-task primitive), **not** a Signal-
specific daemon. That is a separate future task, deliberately not in scope here.

## Phased plan

- **Phase 0 — RISK RETIREMENT (in progress; gates everything).** The real unknown
  is **not** an aarch64-android cross-compile — it's whether the **Signal Rust
  stack compiles to `wasm32-wasip2` and can drive its networking over `wasi:tls`
  instead of native TCP/tokio**. Steps:
  1. **Compile probe** — `repros/signal-phase0/`: a throwaway `wasm32-wasip2`
     crate depending on `libsignal`/`libsignal-service-rs` (latest git rev —
     [[feedback_check_latest_versions]]); `cargo build --target wasm32-wasip2`;
     catalog what breaks. Likely offenders: `tokio` (mio/native poll), `hyper`,
     native-TLS, OS-socket assumptions. Pure-crypto crates should pass —
     `libsignal` already has an official wasm build, so the protocol/crypto half
     is plausible.
  2. **Transport seam** — `libsignal-service-rs` hides its HTTP-push + websocket
     behind traits (historically `libsignal-service-hyper` was one impl). Inject a
     **wasip2 transport over `wasi:sockets` + `wasi:tls`** there, reusing all
     protocol/crypto logic; only swap bytes-on-the-wire. Single-threaded wasip2
     async = a futures executor over WASI `poll`, not tokio
     ([[feedback_wasi_threading]]). If `libsignal-service-rs` is too tokio/hyper-
     coupled, fall back to driving `libsignal`'s lower-level crates directly and
     hand-writing provisioning + websocket frames over `wasi:tls`.
  3. **Prove link + receive end-to-end** — minimal guest that runs the secondary-
     device provisioning handshake (surface the `sgnl://` URI / QR; **user scans
     from their real Signal phone** — pause + ask before this live step) and
     receives + decrypts one inbound 1:1 text message, on device, through
     `wart-host`, packaged as a warpkg like `repros/wasi-tls-probe` (task-66
     commit `69714827`).
  A hard wall (un-decouplable tokio/hyper, or a crypto crate that won't build for
  wasip2) is itself the Phase-0 finding → stop, report options. **No fallback to
  the rejected host-daemon design without explicit user direction.**
- **Phase 1 — grow the probe into the real guest.** Provisioning + a persistent
  foreground receive loop + local store (WASI fs), all guest-side. Send + receive
  text in one hardcoded conversation.
- **Phase 2 — dioxus UI.** Conversation view over the protocol layer (same guest).
  Compose alternative considered but dioxus matches the Rust stack.
- **Phase 3 — real UI.** Conversation list, contacts, history scroll.
- **Phase 4 — (future, optional) background delivery.** Only via a generic host
  capability for all apps — separate task.

## Open decisions for the new session
- **Protocol layer:** `presage` (high-level, bundles a store — may drag in native
  assumptions) vs `libsignal-service-rs` directly (lower-level, cleaner transport
  seam for wasip2). Phase-0 Step 1 decides which actually builds.
- **Store backend:** WASI fs now (simplest in-guest) vs a generic host store WIT
  later. Avoid native sqlite/sled assumptions in-guest.
- **UI toolkit:** dioxus (recommend — matches the Rust stack) vs Compose.

## Phase 0 result (2026-05-30) — split verdict, decision pending

Probe in `repros/signal-phase0/` (`cargo build --target wasm32-wasip2`):
- ✅ **Crypto/protocol half compiles to wasm32-wasip2** — `libsignal-protocol`,
  `zkgroup`, `signal-crypto`, the signal `curve25519-dalek` fork, etc. (after two
  generic host-tooling fixes: modern `protoc` ≥3.12 — installed 35.0 at
  `~/tools/protoc/bin/protoc`; and replicating libsignal's `[patch.crates-io]`
  curve25519 redirect so zkgroup doesn't see two `RistrettoPoint` types).
- ❌ **Transport half does NOT** — current `libsignal-service-rs` (`f93ec5a`) does
  HTTP+WS via `reqwest`/`reqwest-websocket`, which on `target_arch=wasm32`
  force-select the browser/wasm-bindgen backend (`wasm-streams`); it can't encode
  as a wasip2 component, and reqwest has no wasip2-native backend
  (reqwest#2979, open). Transport is localized to `src/push_service/` +
  `src/websocket/`.

**The blocker is purely transport.** Guest-side is still correct. Open options
(awaiting user decision — do NOT default to the rejected host-daemon):
1. **Fork-and-swap** `libsignal-service-rs`'s transport (push_service + websocket)
   onto `wasi:tls`. Keeps current protocol; real but bounded fork (reqwest types
   leak into signatures, not a clean trait).
2. **Pin an older trait-based libsignal-service** (libsignal-service-hyper era) and
   add a `wasi:tls` transport impl. Cleaner seam; risk of an outdated wire protocol
   Signal servers may reject.
3. **Drive `libsignal-protocol` directly + hand-write the service layer**
   (provisioning, websocket, send/receive) over `wasi:tls`. Most control/work.

### Decision (user, 2026-05-30): Option 1 — fork + swap transport onto wasi:tls
Fork cloned at `external/libsignal-service-rs` @ `f93ec5a`. Detailed design +
hard-won facts: [[project_signal_wasip2_transport_swap]].

**Surface is small/bounded** (grepped the fork): reqwest types needing a shim =
`Client`, `ClientBuilder`, `RequestBuilder`, `Response`, `Certificate`, `Error`
(`Method`/`StatusCode` come free from the `http` crate). RequestBuilder methods
used ≈12 (send/json/header(s)/body/status/text/basic_auth/bytes/query/upgrade);
`multipart` is 1 use (CDN, stub for text-only v1). tokio = 3 sites
(`time::interval_at`, `time::Instant`, `task::spawn`). REST is **HTTP/1.1 only**
(`push_service/mod.rs:101`), WS is a standard `wss` upgrade. Transport lives in
`src/push_service/` + `src/websocket/` + `push_service/response.rs`.

**Foundation:** `wstd` (Bytecode Alliance WASI-0.2 async stdlib — executor over
`wasi:io/poll`, timers, TCP) + task-66 `wasi:tls` (bindings reusable from
`repros/wasi-tls-probe`). HTTP/1.1 client + RFC6455 WebSocket hand-rolled over the
TLS stream.

**Implementation increments:**
- **(0) Transport seam** — add `external/libsignal-service-rs/src/transport/`;
  `#[cfg(not(wasm32))]` re-exports reqwest/reqwest-websocket (native unchanged,
  tests pass); `#[cfg(wasm32)]` points at the new backend. Rewrite
  `use reqwest::`/`use reqwest_websocket::` → `use crate::transport::` (~19 files,
  mechanical). Verify: native `cargo build` still green.
- **(1) wasi HTTP/1.1 client** over wasi:tls + wstd → `libsignal-service` compiles
  for `wasm32-wasip2`. (Recommend a standalone `wstd`+`wasi:tls` HTTP-GET spike in
  `repros/signal-phase0/` first to de-risk the executor↔wasi:tls integration under
  wasmtime, then drop that code into the transport module.)
- **(2) wasi WebSocket** (Stream+Sink of `Message`); map the 3 tokio sites to wstd.
- **(3) link + receive** one 1:1 text message, on device via wart-host (warpkg,
  like `repros/wasi-tls-probe`, commit `69714827`). User scans the QR.

### Increment (1)-spike — DONE 2026-05-30 (`repros/wstd-wasitls-spike/`)
**Async transport foundation PROVEN.** `wstd 0.6.6` async executor drove a full
async DNS→TCP→TLS-handshake→HTTP/1.1 pipeline over task-66 `wasi:tls`:
`example.com` → 200 OK; through the Signal-CA runner, `chat.signal.org` → 404
(TLS+HTTP completed — 404 just means `GET /` isn't an endpoint). Key facts in the
spike README + [[project_signal_wasip2_transport_swap]]: `wstd`(`wasip2` crate) +
our `wit_bindgen` tls bindings coexist with no `cabi_realloc` collision; bridge
pollables by raw handle into `wstd::runtime::AsyncPollable`; mind component-model
drop order (parent resource outlives its child streams).

### Increment (1)+(2) — DONE 2026-05-30: libsignal-service compiles for wasm32-wasip2
Built the transport as **drop-in shim crates** swapped via cargo `package =`
rename (NOT an import rewrite — cargo forbids per-target sources for one dep name,
so each shim is the single source and cfg-dispatches: real crate on native,
wasi:tls impl on wasm). Location: `external/libsignal-service-rs/wart-wasi-shims/`:
- `reqwest/` (`wart-reqwest-shim`) — `Client`/`ClientBuilder`/`RequestBuilder`/
  `Response`/`Certificate`/`Error`/`multipart` over HTTP/1.1 on the shared
  `tls::TlsStream` (wasi:tls + wstd, from the spike). Exposes `random_bytes` +
  `tls` for the ws shim.
- `reqwest-websocket/` (`wart-reqwest-websocket-shim`) — RFC6455 `WebSocket`
  (inherent async `send`/`next`/`close`, `next()` boxed→Unpin for the fork's
  `select!`), `Message`/`CloseCode`, `RequestBuilderExt::upgrade`.
- Fork edits: `Cargo.toml` deps point at the shims (+ wstd wasm-only); 3 tokio
  sites cfg-gated (`push_service` spawn → `wstd::runtime::spawn().detach()`;
  `websocket` keepalive `interval_at` → a cfg `KeepAliveTimer` over `wstd::task::
  sleep`). Build: `repros/signal-phase0` now `path`-deps the local fork;
  `PROTOC=~/tools/protoc/bin/protoc cargo build --target wasm32-wasip2` → OK.
Details: [[project_signal_wasip2_transport_swap]].

### Increment (3a) — DONE 2026-05-30: provisioning QR over wasi:tls, real Signal
`repros/signal-link/` (wasm32-wasip2 `wstd::main` guest) opened Signal's
provisioning websocket through the fork's wasi:tls transport, received a real
provisioning UUID from chat.signal.org, and rendered the `sgnl://linkdevice` QR.
Run desktop via `wasi-tls-runner` (Signal CA). **Proves the whole HTTP/1.1 +
RFC6455-WebSocket-over-wasi:tls transport against live Signal.** Needed only a
1-line fork export (`pub use pipe::{ProvisioningPipe, ProvisioningStep}`) — no
store, since `ProvisioningPipe::from_socket` needs just the ws + RNG. Fixed a
`RefCell` double-borrow footgun in `tls.rs::read_until` (if-let temp lifetime).

### Increment (3b) — built + verified to the scan point, 2026-05-30
`repros/signal-link/` now runs the FULL secondary-device link: an in-memory
`PreKeysStore` (`src/store.rs`, real HashMap/Cell impl of all 6 traits — modeled on
the fork's `examples/storage.rs`; `mark_kyber_pre_key_used` takes 4 params) +
`provisioning::link_device` driven over a channel. Desktop run (Signal-CA runner)
reaches the QR + parks at "waiting for you to scan…". Everything past the scan
(decrypt `ProvisionMessage` → `replenish_pre_keys` → `push_service.link_device`
REST register → `NewDeviceRegistration`) is implemented and runs on scan. RNG =
ChaCha20 from getrandom; random device password; device name "wart".

**3b LIVE-VERIFIED 2026-05-30:** user scanned → `LINKED ✓ device_id=4`, "link flow
finished OK". A wasm guest linked as a real Signal secondary device, all transport
over host wasi:tls. QR gotcha: the terminal unicode QR is NOT phone-scannable —
render a PNG from the printed `sgnl://` URL (`qrencode -s 12 -m 4`, `xdg-open`).
Store is in-memory so device 4 is now orphaned server-side (state lost on exit).

### Increment (3c) — DONE + LIVE-VERIFIED 2026-05-30: received + decrypted a message
`repros/signal-link/` now does link **+ receive** in one run: `do_link` →
set the provisioned ACI identity into the store → `do_receive` (`PushService` +
`ServiceCredentials` → `MessageReceiver::create_message_pipe` → `MessagePipe::stream`
→ `ServiceCipher::open_envelope`). `store.rs` grew to a full `ProtocolStore`
(`SessionStore` + `SenderKeyStore` + `SessionStoreExt` + all prekey traits) as
`Rc<Inner>` so `Clone` is a shared handle (the cipher clones it). **Live result:**
linked as device_id=5, then decrypted a real E2EE 1:1 text ("How are you wart") +
the typing indicator. Accepts an incoming `DataMessage` or your own sync
sent-transcript (Note-to-Self works as a test).

**PHASE 0 IS FULLY DE-RISKED** — crypto→wasip2, transport over wasi:tls, link, and
receive+decrypt all proven live. Operational notes: render the QR as a PNG
(`qrencode`; terminal QRs don't scan) and scan FAST (provisioning window ~30-60s);
the in-memory store orphans each linked device on exit (prune old "wart" devices).

### Increment (4) — DONE + ON-DEVICE-VERIFIED 2026-05-30
Packaged `signal-link` as a warpkg (`package.toml`, `app_id = war.signal.link`,
`wasi:cli/command`) and ran it through the production **wart-host** on the Pixel 2
XL: `--install` (precompiled the libsignal-sized component to aarch64 cwasm) →
`--zygote-launch war.signal.link`. On-device logcat: forked pid 17582 → `LINKED ✓`
→ message socket open → `MESSAGE ✓` (received + decrypted an incoming message
entirely on aarch64-android, transport over the device network + Signal CA). Capture
the `sgnl://` URL from logcat, render a PNG host-side (`qrencode`), scan. Fixed the
logcat multi-write truncation (single `eprint!("{line}")` incl `\n`, per the
wasi-tls-probe README) and redeployed.

**TASK 67 v1 CORE COMPLETE on real hardware:** a wasm guest links + receives a
Signal message on-device, zero crypto-networking in the guest.

### Phase 1 — Persistence DONE + desktop-verified 2026-05-30
Pure-Rust WASI-fs persistence (sqlite reuse rejected: `presage-store-sqlite` is
sqlx+presage+tokio, and sqlx wasip2 is unresolved — sqlx#4056; rusqlite would need
a wasi-sdk C build). `src/persist.rs` (Account / StoredMessage / store snapshot /
messages.jsonl under the `/state` preopen) + `store.rs` `snapshot_bytes()`/
`load_into()` (serde-JSON of libsignal records via `.serialize()`/`deserialize`).
`main.rs` resumes if `account.json` exists (load + `set_identity` + `load_into`),
else links + saves; `do_receive` persists the snapshot after every envelope (the
ratchet advances sessions) + appends messages, and keeps looping. Runner preopens a
host dir as guest `/state` (arg 2). **Verified:** first run linked + persisted 3
texts; second run (same state) **resumed device 4 with no QR/re-link**, loaded the
3-message history, kept receiving. Ends the orphaned-device churn.

### Phase 1 — Sending DONE + desktop-verified 2026-05-30
`do_receive` drains `/state/outbox.txt` and sends each line as a note-to-self via
`MessageSender` (identified ws = `pipe.ws()`, unidentified ws =
`push.ws::<Unidentified>("/v1/websocket/")`, cipher/store/identity), then receives.
`MemStore` got `unsafe impl Send+Sync` (single-thread wstd; `MessageSender` wants
`S: Sync`). **Key fix that unblocked it:** `SessionStoreExt::get_sub_device_sessions`
must return the real per-device sessions — the empty stub made
`create_encrypted_messages` only target the primary, so the MismatchedDevices retry
never converged ("max retries" + a 600s rate-limit). **Verified:** the guest sent 2
notes-to-self (Cyrillic incl.) → `SENT ✓`, persisted outgoing. The client is now
**two-way**: link (persisted) → receive+decrypt → send → persist + history.

### Phase 2 architecture (decided 2026-05-30, user): WIT-decoupled engine + UI
The Signal client is **two composed components** with a WIT contract between them,
NOT one monolith and NOT a host daemon:
- **signal-engine** — imports `wasi:tls`/sockets/fs/random/clocks; **exports
  `wart:signal/chat`**. Owns link/resume + the persistent connection + store +
  history (all the work we built in `repros/signal-link`).
- **signal-ui** — **imports `wart:signal/chat`** + the `my:skiko-gfx` world;
  exports `renderer`. A thin dioxus (later Compose) view. Toolkit-agnostic because
  the contract is the only coupling.
- Composed via WAC (`link.wac`, like task 36) into one app component.

The `wart:signal/chat` contract:
```wit
package wart:signal@0.1.0;
interface chat {
    record message { id: u64, from: string, text: string, ts: u64, outgoing: bool }
    variant event { message(message), link-url(string), linked(string), connected, disconnected }
    init: func();
    poll-events: func() -> list<event>;        // UI calls each frame; pumps net + drains events
    send: func(text: string) -> result<_, string>;
    history: func() -> list<message>;
    state: func() -> string;
}
world signal-engine { /* imports wasi:tls/sockets/fs/random/clocks */ export chat; }
world signal-ui     { import chat; /* + my:skiko-gfx */ export renderer; }
```

**KEY RUNTIME FINDING (gates the engine):** `wstd::block_on` creates a fresh
reactor per call and clears it on return — spawned tasks do NOT survive across
component-export calls, and no guest code runs between calls. So the engine can't
do a short `block_on` per `poll-events`. It needs a **persistent single-thread
step-executor** (built in `init`, stepped non-blocking each `poll-events`), and the
`wart-wasi-shims` pollable-await (`tls.rs::schedule` → `wstd::runtime::AsyncPollable`)
must bind to that executor instead. wstd's reactor stepping methods are private, so
roll a minimal executor (futures + task queue + a non-blocking `wasi:io/poll` with a
0-timeout pollable). Contained in the engine; UI/contract unaffected.

## Next action (Phase 2 build order)
1. `repros/signal-engine/` — refactor `signal-link` into a component exporting
   `wart:signal/chat` (init/poll-events/send/history/state) over a persistent
   step-executor; reuse `store.rs`/`persist.rs`/the shims.
2. `repros/signal-ui/` — dioxus-canvas guest importing `chat`; render history +
   live events from `poll-events`; input → `send`; QR from `link-url`.
3. Compose via `link.wac`; run on device.
Also pending: on-device **writable** `/state` preopen in wart-host (task 38 wired
only `/assets` read).
- **Sending:** wire `MessageSender` (outgoing 1:1 text).
- **UI:** dioxus guest rendering a conversation over a thin in-guest API to the
  link/receive engine; `skiko-gfx` for rendering.
- **Background delivery:** a *generic* host keep-alive capability for all apps
  (never a per-app daemon).
- Cleanup: prune the orphaned "wart" linked devices in Signal.

## Key references
- Transport (load-bearing): `tasks/66`, `repros/wasi-tls-{probe,runner}`,
  [[reference_wart_wasi_tls_transport]]. Network grant: `inherit_network` +
  `allow_ip_name_lookup`.
- Signal: github.com/whisperfish/presage · github.com/whisperfish/libsignal-service-rs
  · github.com/signalapp/libsignal (has a wasm build).
- wasip2 single-thread async constraint: [[feedback_wasi_threading]].
- Constraints: clean library usage ([[feedback_clean_library_usage]]); latest
  versions ([[feedback_check_latest_versions]]); no ART-layer deps
  ([[feedback_no_art_layer_dependencies]]).
