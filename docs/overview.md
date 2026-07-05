# wandr — a portable UI runtime for WASM apps

> **The one idea:** the contracts are OS-agnostic; only the backend layer
> is OS-specific. A UI app compiled once — in any language, with any
> framework — to a WASM component runs natively wherever wandr has a
> backend. **Android is the production backend (post-ART); Linux is a
> working desktop/dev backend; others are proposals.**

This is the front-door doc. It gives the mental model; every other doc
in `docs/` is a layer or a slice of it. If you read one thing first,
read this, then jump to the layer you care about via the
[index](README.md).

## What wandr is

wandr runs UI applications compiled to WASM components. A guest imports
a fixed set of **OS-agnostic WIT contracts** (rendering, input, IME,
chrome, device, media, audio, events) and never names an OS. `wandr-host`
— wasmtime + the component model + a Skia rendering core — implements
those contracts, delegating only the physically OS-specific bits
(surfaces, input devices, native services) to a per-OS **backend**.

The payoff: **write the UI once, run it on any OS wandr has a backend
for.** Today that's Android in production and Linux as a working
desktop/dev target — the same guest `.wasm` runs on both.

## The layer model

```
┌─ GUESTS ─ any language → any UI framework → WASM component ────────┐
│  Compose · Slint · dioxus · Avalonia · SwiftUI · (egui/Flutter…)   │
└────────────────────────────────────────────────────────────────────┘
        │ imports  (OS-agnostic WIT — the portable ABI)
┌─ CONTRACTS ───────────────────────────────────────────────────────┐
│  render (wasi:canvas / webgpu) · input · ime · chrome · device     │
│  media · audio · events · assets     ◀── THIS is the portable part │
└────────────────────────────────────────────────────────────────────┘
        │ implemented by
┌─ RUNTIME ─ wandr-host + arbiter/zygote ───────────────────────────┐
│  wasmtime + component model + Skia rendering core (OS-agnostic)    │
└────────────────────────────────────────────────────────────────────┘
        │ backed by  (the ONLY OS-specific layer)
┌─ OS BACKENDS ─────────────────────────────────────────────────────┐
│  Android  production  ·  Linux  working  ·  Redox/…  proposed      │
└────────────────────────────────────────────────────────────────────┘
```

- **Guests** — a UI app in any language/framework, compiled to a WASM
  component. It links only against the contract WIT; it is unaware of
  the OS underneath.
- **Contracts** — the portable ABI. This is the layer that makes wandr
  "portable": the same WIT spec everywhere. See the *Contracts* section
  of the [index](README.md).
- **Runtime** — `wandr-host` (wasmtime, component model, Skia) plus the
  `arbiter`/`zygote` coordination processes. OS-agnostic; it owns
  rendering and app lifecycle, and calls down to a backend for the
  physical bits. See [`architecture-runtime.md`](architecture-runtime.md)
  and [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md).
- **OS backends** — the only OS-specific layer: surfaces, input, native
  services. Swapping the backend is what ports wandr to a new OS. See
  [`wandr-os-portability.md`](wandr-os-portability.md) for how a backend
  plugs in.

## Maturity — honest status by layer

"Portable" is a property of the architecture, not a claim that every
backend is finished. Where things actually stand:

### OS backends

| Backend | Status | Notes |
|---|---|---|
| **Android** (aarch64, Pixel 2 XL / API 35) | **production** | Rendering, input, IME, chrome, audio, camera, calls, sensors, auto-rotation/brightness, and the full `--no-art` native-service stack. This is the flagship. |
| **Linux** (x86_64, WSLg-verified) | **working dev/desktop** | The *same* guest `.wasm` runs via the desktop dev loop (JIT, `WANDR_DESKTOP_SIZE`, softbuffer present path, W3C key input). Missing the device/chrome/native-service depth Android has — see the Linux gap notes in [`wandr-os-portability.md`](wandr-os-portability.md). |
| **Redox / others** | **proposed** | Feasibility only — see [`redox-wandr-feasibility.md`](redox-wandr-feasibility.md). |

### Guest languages

A guest can be written in any language that emits a WASM component for our
world. Status vocabulary: **✅ in use** (a real wandr guest ships in it) ·
**🟢 ready** (proven P2-component toolchain, would drop in with no host
change) · **🟡 DIY** (possible, but you hand-roll the plumbing, or it's
Preview-1 only) · **🔴 not yet** (no practical path today).

*Compiled (can drive the render loop):*

| Language | Status | Notes |
|---|---|---|
| **Rust** | ✅ in use | Reference toolchain; the light chrome guests (launcher/statusbar/taskbar/dioxus). Native wasip2, no adapter. |
| **Kotlin/Wasm** | ✅ in use | The Compose guests. WasmGC + wandr-fork P1→P2 reactor adapter (KT-86415 pin). |
| **C# / .NET** | ✅ in use | The Avalonia demo (device-verified). `componentize-dotnet` (NativeAOT-LLVM), full WIT import/export. |
| **Swift** | ✅ in use (spike) | OpenSwiftUI 2048 plays + device-stable. wasm32-wasip1 + adapter. |
| **Go (TinyGo)** | 🟢 ready | `tinygo -target=wasip2`; would slot in, but no Skia-backed Go UI lib exists yet. Stock `go` is P1-only. |
| **Zig** | 🟡 DIY | wit-bindgen C generator + Zig C-interop + adapter; all plumbing manual. |
| **Dart (Flutter)** | 🔴 not yet | dart2wasm is JS-env-only today; non-JS standalone is active upstream (dart-lang/sdk#53884), not shipped. |
| **Java (JVM)** | ✅ in use (spike) | **TeaVM WasmGC+WASI (JS-free) proven end-to-end on desktop** — pure-Java → WasmGC core (zero JS imports) → component with a custom WIT, called under wasmtime with correct results (tasks 112/113; wandr fork `harryzz/teavm:wasmgc-wasi-poc`, patches in `repros/java-wasm-spike/`). Productionization (gradle plugin, allocating guests) + device consumption pending. GraalVM **Web Image** (`--tool:svm-wasm`) and **J2CL** also compile Java→WasmGC, but are **browser/JS-host only (no WASI)** — watch, not usable by wandr yet. |

*Interpreted / dynamic (embed a whole VM — good for logic/plugin guests, poor for a 60 fps render hot path):*

| Language | Status | Notes |
|---|---|---|
| **JavaScript / TS** | 🟢 ready | ComponentizeJS + StarlingMonkey (SpiderMonkey→wasm), ~8 MB engine. Best-supported dynamic language. |
| **Python** | 🟢 ready | `componentize-py` embeds CPython; full WIT import/export, custom worlds. |
| **Ruby** | 🟡 DIY | `ruby.wasm` is Preview-1 only; no first-class componentize-ruby yet — manual P1→P2. |
| **Lua** | 🟡 DIY | No component-model tooling; embed Lua inside a Rust component instead. |
| **Perl** | 🔴 not yet | WebPerl is Emscripten/browser-JS only; no WASI-P2 path. |

Full survey + toolchain detail: [`wasm-component-language-support.md`](wasm-component-language-support.md).

### Guest UI frameworks

| Framework | Status |
|---|---|
| **Compose Multiplatform** | shipped — the original target; real Compose UIs render on device |
| **Slint** | shipped / device-verified (task 100) |
| **dioxus** | shipped — `crates/dioxus-canvas`, production guest UI |
| **Avalonia** (.NET) | shipped / device-verified (tasks 106–107) |
| **Swift / OpenSwiftUI** | spike working, device-stable |
| **egui** | analyzed — belongs on the wasi:webgpu lane, not wasi:canvas |
| **Flutter** | analyzed — blocked on dart2wasm standalone (upstream, active) |
| **Qt** | analyzed — no wasi port; not practical today |
| **Ruby** | analyzed — viable-but-DIY (no Skia UI layer yet) |

See the *Guest languages & UI-framework feasibility* section of the
[index](README.md) for the per-framework memos.

### Contracts

| Contract | Status |
|---|---|
| `wasi:canvas` + `wasi:input-handlers` | in use, device-verified; 0.0.2 redesign underway |
| `wandr:*` (ui-shell / device / chrome / assets / ime) | in use |
| `wasi:webgpu` (2nd rendering lane, guest-owns-renderer) | proposed / host-side |
| `wandr:media` · audio player · `wasi:media-session` | designed / partial |

## Demo apps — language/framework × contracts exercised

The apps in `apps/user/` (and a couple of spikes in `repros/`) are the
polyglot proof: different languages and UI frameworks, all coexisting behind
the same WIT. A **†** marks a **proposed / WASI-track** contract (`wasi:*`);
unmarked `wandr:*` interfaces are wandr host contracts (OS-agnostic, not yet
proposed to WASI).

Proposed-WASI contracts in play: `wasi:canvas@0.0.2`† (render) ·
`wasi:input-handlers@0.0.2`† (pointer/key/IME input) · `wasi:audio/pcm@0.0.1`†
· `wasi:media-session@0.0.1`† (← W3C Media Session) · `wasi:tls`† (guest TLS).

### Interactive UI demos

| App | Language / framework | Contracts exercised | Status |
|---|---|---|---|
| **wandr.signal** — Signal messenger, **fully functional incl. audio + video calls** | Rust (dioxus-canvas UI + pure-Rust WebRTC engine) | `wasi:canvas`† · `wasi:input-handlers`† · `wasi:tls`† (signaling/transport) · `wasi:audio/pcm`† · `wandr:crypto/aead` (SRTP HW-AES offload) · `wandr:video/encoder`+`decoder` (HW VP8 + PiP) · `wandr:audio-focus` · `wandr:chrome/status` · `wandr:notify` · `wandr:alarm` · `wandr:signal/chat` | live-verified both ways |
| **2048** (`repros/openswiftui-wasm`) | **Swift / OpenSwiftUI** | `wasi:canvas`† (render) · `wasi:input-handlers/pointer-handler`†+`frame-handler`† (swipe → `on_pointer` → `@State`) · `wandr:ui-shell/frame-pacing` | **user-playable on device** (Pixel 2 XL, swipe to move) + a demo auto-play mode; device-confirmed |
| **wandr.tetris** — Tetris | Rust | `wasi:canvas`† (draw/embedding/layout) · `wasi:input-handlers`† · `wasi:audio/pcm`† (SFX) · `wandr:chrome/launcher` · `wandr:ui-shell` | playable |
| **wandr.audio.player** — audio player | Rust | `wasi:audio/pcm`† · `wasi:media-session`† · `wandr:background` | task 108 |
| **wandr.avalonia.demo** — Fluent controls | **.NET / C# (Avalonia)** | `wasi:canvas`† (draw/embedding/glyphs) · `wandr:ui-shell/ime`+`metrics` | shipped, device-verified |
| **wandr.slint.test** | Rust / **Slint** | `wasi:canvas`† · `wasi:input-handlers`† · IME · emoji | shipped |
| **wandr.dioxus.demo** | Rust / **dioxus** | `wasi:canvas`† · `wasi:input-handlers`† | production lib demo |
| **wandr-app** — reference Compose app | **Kotlin / Compose Multiplatform** | `wasi:canvas`† · `wasi:input-handlers`† · IME | reference guest |
| **wandr.ktcanvas.test** | Kotlin / Compose | `wasi:canvas`† | canvas test |
| **wandr.taskmanager** | Rust / dioxus | `wandr:task-manager` | shipped |
| **wandr.alarm.test** | Rust | `wandr:alarm/scheduler` · `wandr:audio-focus` · `wandr:notify` | test |

### Capability tests & benches (headless — `wasi:cli/command`)

| App | Language | Contracts exercised | Purpose |
|---|---|---|---|
| **wandr.video.test** | Rust | `wandr:video/encoder`+`decoder` | camera → HW VP8 → HW decode |
| **wandr.crypto.test** | Rust | `wandr:crypto/{aead,hash,kdf,cipher,mac,key-exchange,signatures,caps}` | host crypto surface |
| **wandr.srtp.bench** | Rust | `wandr:crypto/aead-oneshot` | SRTP HW-AES throughput bench |
| **wandr.connectivity.test** | Rust | networking | link/connectivity probe |

*(System chrome — launcher, statusbar, taskbar, IME keyboard, keyguard,
powermenu, settings — are Rust guests on the same `wasi:canvas`† /
`wasi:input-handlers`† contracts; see `apps/system/`.)*

## Where to go next

- **The portable ABI** → *Contracts* in the [index](README.md)
  (rendering, input, IME, media).
- **How the runtime works** →
  [`architecture-runtime.md`](architecture-runtime.md),
  [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md).
- **Porting to a new OS** →
  [`wandr-os-portability.md`](wandr-os-portability.md).
- **Running a given UI framework** → *Guest frameworks* in the
  [index](README.md).
- **Setup / build** → [`build-pipeline.md`](build-pipeline.md) and
  `~/wandr/CLAUDE.md`.
