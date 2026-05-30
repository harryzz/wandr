---
name: project_signal_wasip2_transport_swap
description: "Task 67 — how to put a Signal client in a wasm guest; the fork-and-swap of libsignal-service-rs's reqwest transport onto task-66 wasi:tls (Phase-0 facts + sized design)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 48a1a9e9-e5d8-4f1f-8fc4-ac9c8bb50eb9
---

Task 67 = Signal client as a wasm32-wasip2 GUEST (see [[project_signal_client_architecture]]).
Phase-0 probe (`repros/signal-phase0/`) verdict + the chosen implementation path.

**Phase-0 fact (de-risked 2026-05-30):** the Signal *crypto/protocol* half
(`libsignal-protocol`, `zkgroup`, `signal-crypto`, the signal `curve25519-dalek`
fork, etc.) **compiles cleanly to wasm32-wasip2** after two GENERIC (non-wasip2)
build-graph fixes: (1) modern `protoc` ≥3.12 — installed 35.0 at
`~/tools/protoc/bin/protoc`, pass `PROTOC=...`; (2) replicate libsignal's
`[patch.crates-io] curve25519-dalek = signalapp fork @ signal-curve25519-4.1.3`
or zkgroup sees two `RistrettoPoint` types (220 errors). Drop default-features to
skip `cdsi` (libsignal-net/BoringSSL).

**The ONLY blocker is transport.** `libsignal-service-rs` (main `f93ec5a`) does
HTTP+WS via `reqwest`/`reqwest-websocket`; on `target_arch=wasm32` those force the
browser/wasm-bindgen backend (`wasm-streams`) which can't encode as a wasip2
component. reqwest has no wasip2 backend (reqwest#2979).

**DECISION (user): Option 1 — fork libsignal-service-rs and swap the transport
onto task-66 wasi:tls.** Fork cloned at `external/libsignal-service-rs` @ `f93ec5a`.

**Surface is small/bounded** (grepped): reqwest types needing a shim = `Client`,
`ClientBuilder`, `RequestBuilder`, `Response`, `Certificate`, `Error`
(`Method`/`StatusCode` come free from the `http` crate). RequestBuilder methods
used: send/json/header/headers/body/status/text/basic_auth/bytes/query/upgrade —
~12 simple ones. `multipart` = 1 use (CDN upload; out of scope text-only v1, stub).
tokio = 3 sites only (`time::interval_at`, `time::Instant`, `task::spawn`).
Signal REST is **HTTP/1.1 only** (`.http1_only()` in push_service/mod.rs:101);
WS is a standard `wss` upgrade. Transport code lives in `src/push_service/` +
`src/websocket/` + `src/push_service/response.rs` (the `SignalServiceResponse` /
`ReqwestExt` trait impls on `reqwest::Response`).

**Design:** add `src/transport/` to the fork. `#[cfg(not(wasm32))]` → `pub use
reqwest::{...}` + `reqwest_websocket::{...}` (native unchanged, tests still pass).
`#[cfg(wasm32)]` → our backend: a reqwest-shaped `Client/RequestBuilder/Response`
doing HTTP/1.1 over wasi:tls, a `WebSocket`(Stream+Sink of `Message`) doing
RFC6455 over wasi:tls, all driven by `wstd`'s wasi:io/poll executor (replaces the
tokio reactor; `wstd` also gives timers + spawn for the 3 tokio sites). wasi:tls /
wasi:sockets bindings reuse `repros/wasi-tls-probe`. Then rewrite `use reqwest::` /
`use reqwest_websocket::` → `use crate::transport::` across ~19 files (mechanical).

**Foundation PROVEN (2026-05-30, `repros/wstd-wasitls-spike/`):** `wstd 0.6.6`'s
wasi:io/poll async executor drove a full async DNS→TCP→TLS→HTTP/1.1 over task-66
`wasi:tls` (example.com 200; chat.signal.org 404 via the Signal-CA runner = TLS+HTTP
OK). Integration facts: `wstd`(uses self-contained `wasip2 1.0.3` crate, no
wit-bindgen-rt) + our own `wit_bindgen::generate!` (0.53) for the `wasi:tls` draft
coexist with NO `cabi_realloc` collision; bridge our wit-bindgen `pollable` into
wstd by raw handle (`take_handle`→`from_handle`)→`wstd::runtime::AsyncPollable::
new(p).wait_for().await`; mind component-model DROP ORDER (parent `ClientConnection`
/ `TcpSocket` must outlive their child streams or runtime traps `resource has
children`); imports-only world (no run export) so it doesn't clash with wstd's.

**Increments (1)+(2) DONE 2026-05-30 — `libsignal-service` fork compiles for
wasm32-wasip2.** Implemented as **drop-in shim crates** swapped via cargo
`package =` rename, NOT an import rewrite: cargo forbids one dep name having
different sources per target, so each shim is the *single* source and
cfg-dispatches internally (real `reqwest`/`reqwest-websocket` on native via
`pub use ...::*`; wasi:tls impl on wasm). Crates in
`external/libsignal-service-rs/wart-wasi-shims/{reqwest,reqwest-websocket}`:
HTTP/1.1 `Client/RequestBuilder/Response/multipart` + RFC6455 `WebSocket`
(inherent async send/next/close; `next()` boxed → Unpin for the fork's
`futures::select!`) over a shared `tls::TlsStream` (the spike's wasi:tls+wstd).
Fork source nearly untouched: Cargo deps point at shims; only 3 tokio sites
cfg-gated (`push_service` spawn→`wstd::runtime::spawn().detach()`; `websocket`
keepalive→`wstd::task::sleep` behind a cfg `KeepAliveTimer`). Gotchas hit:
`bytes_stream()` must be `Unpin` (use `stream::iter`, not `once(async{})`);
`Response: Debug`; `wstd::task::sleep` takes `wstd::time::Duration` + resolves to
`Instant` (wrap in `async {}`). Build via `repros/signal-phase0` (path-deps the
fork) with `PROTOC=~/tools/protoc/bin/protoc`.

**Increment 3a DONE+PROVEN 2026-05-30** (`repros/signal-link/`): a wasm32-wasip2
`wstd::main` guest opened Signal's provisioning websocket through the fork's
wasi:tls transport, got a real provisioning UUID from chat.signal.org, rendered the
`sgnl://linkdevice` QR (desktop via `wasi-tls-runner`/Signal CA). **The full
HTTP/1.1 + RFC6455-WebSocket-over-wasi:tls transport works against live Signal.**
Only a 1-line fork export needed (`pub use pipe::{ProvisioningPipe,
ProvisioningStep}`; `from_socket` needs just ws+RNG, no store). Bug fixed:
`tls.rs::read_until` held a `RefCell` `Ref` across a mutable borrow via if-let
temporary lifetime — bind to a local first.

**3b BUILT + verified-to-scan 2026-05-30** (`repros/signal-link/src/store.rs` +
`main.rs`): real in-memory `PreKeysStore` (all 6 traits, modeled on the fork's
`examples/storage.rs`; note `mark_kyber_pre_key_used` has 4 params, `IdentityKeyPair`
is `Copy`, `IdentityChange::{NewOrUnchanged,ReplacedExisting}`) +
`provisioning::link_device` over a channel. Reaches the QR + parks at the scan;
post-scan path (decrypt→replenish→REST register) implemented. **PENDING: user scans
the QR to complete the live link (registers a real "wart" linked device).**

**3b LIVE-VERIFIED 2026-05-30:** user scanned the QR → `LINKED ✓ device_id=4`,
"link flow finished OK". A wasm32-wasip2 guest linked as a REAL Signal secondary
device, all transport over host wasi:tls, zero crypto-networking in the guest. The
whole guest-side architecture is proven end-to-end against live Signal. **QR gotcha:
the terminal unicode QR (qrcode Dense1x2) is NOT phone-scannable — render a PNG from
the printed `sgnl://` URL instead (`qrencode -s 12 -m 4 -o png`; on WSL `xdg-open`
shows it in Windows).** In-memory store ⇒ a linked device is orphaned on process
exit (link+receive must be one run).

**3c DONE + LIVE-VERIFIED 2026-05-30 — PHASE 0 FULLY DE-RISKED.** `repros/signal-link`
links AND receives in one run: `do_link` → `set_identity` (provisioned ACI) →
`do_receive` (`MessageReceiver::create_message_pipe` → `MessagePipe::stream` →
`ServiceCipher::open_envelope`). `store.rs` is now a full `ProtocolStore`
(`SessionStore`+`SenderKeyStore`+`SessionStoreExt`+prekey traits) as `Rc<Inner>`
(Clone = shared handle, required by `ServiceCipher<S: …+Clone>`). **Live: linked
device_id=5, decrypted a real E2EE 1:1 text + the typing indicator.** Accepts
incoming `DataMessage` or own sync sent-transcript (Note-to-Self testable).
Operational: render QR as PNG (`qrencode`; terminal QR won't scan); scan FAST
(provisioning window ~30-60s); in-memory store orphans each linked device on exit.

**4 DONE + ON-DEVICE-VERIFIED 2026-05-30 — TASK 67 v1 CORE COMPLETE ON HARDWARE.**
signal-link packaged as a warpkg (`package.toml`, app_id `war.signal.link`,
`wasi:cli/command`), ran through production wart-host on the Pixel 2 XL: `--install`
(precompiled libsignal-sized component to aarch64 cwasm) → `--zygote-launch
war.signal.link` → `LINKED ✓` (pid 17582) → `MESSAGE ✓` (received+decrypted
on-device). Capture the `sgnl://` URL from logcat, render PNG host-side (`qrencode`),
scan FAST. logcat gotcha: host sink emits one line per newline-terminated write —
`eprintln!` multi-arg splits + only the first surfaces; use a single
`eprint!("{line}\n")`. **Capstone v0.1.1 printed the full line on-device:
`MESSAGE ✓ from <ACI:…>: Мараба` (Cyrillic UTF-8 round-tripped).** DEPLOY GOTCHA
(cost 2 stale runs): `adb push <dir> /data/local/tmp/X` when X exists NESTS the new
files inside → installer keeps reading the stale top-level `package.toml`/wasm, so
"redeploys" run old code. Always `rm -rf` the device warpkg dir **and** the install
dir (`/data/local/tmp/wart-apps/system-apps/<app_id>`) before re-push, and bump
`package.toml` version (the installer caches the cwasm by app_id+version). All hard
risk retired.

**Phase 1 persistence DONE + desktop-verified 2026-05-30.** Pure-Rust WASI-fs
(sqlite reuse rejected: presage-store-sqlite = sqlx+presage+tokio, sqlx wasip2
unresolved sqlx#4056; rusqlite needs a wasi-sdk). `repros/signal-link/src/persist.rs`
+ `store.rs` `snapshot_bytes()`/`load_into()` (serde-JSON of libsignal records via
`.serialize()`/`deserialize`; `SignedPreKeyRecord`/`KyberPreKeyRecord` need
`use …protocol::GenericSignedPreKey`, `IdentityKey::decode` to deserialize). `main.rs`
resumes from `account.json` (no re-link) else links+saves; persists the snapshot
after EVERY envelope. Runner preopens a host dir as guest `/state` (arg 2).
Verified: link once → 3 texts persisted → restart resumed device 4 with no QR +
loaded history. Orphan churn ended.

**Phase 1 SENDING DONE + desktop-verified 2026-05-30 — client is now TWO-WAY**
(link[persisted]+receive+decrypt+send+persist+history). `do_receive` drains
`/state/outbox.txt` → `MessageSender` notes-to-self (identified ws `pipe.ws()`,
unidentified `push.ws::<Unidentified>("/v1/websocket/")`). `MemStore` needs
`unsafe impl Send+Sync` (single-thread wstd; `MessageSender` wants `S: Sync`). KEY
FIX: `SessionStoreExt::get_sub_device_sessions` MUST return real per-device sessions
(parse keys `service_id_string.device`, exclude dev 1) — an empty stub makes
`create_encrypted_messages` only target the primary → MismatchedDevices retry never
converges → "max retries" + a 600s server rate-limit. Sent 2 texts (Cyrillic) →
`SENT ✓`.

**NEXT — Phase 1 cont. (no novel risk):** (a) on-device: wart-host must grant a
WRITABLE per-app `/state` preopen (task 38 wired only `/assets` read) — a generic
capability — then verify resume+send on device; (b) dioxus UI over `skiko-gfx`;
(c) background via a GENERIC host keep-alive capability (never a per-app daemon).
Foreground-only v1.
