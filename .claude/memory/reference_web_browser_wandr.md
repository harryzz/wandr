---
name: reference_web_browser_wandr
description: "Web browser on wandr — DECISION (2026-08-01): a real browser stays HOST-side (native web engine / WebView, Proposal B), NOT a wasm guest. JS-engine + Web-platform-API + no-in-wasm-JIT walls. Blitz = separate GUEST content-viewer lane (HTML/CSS, no/light JS) → wasi:canvas + wandr-reqwest. On-the-fly JS→wasm JIT doesn't exist / structural mismatch."
metadata:
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-08-01T10:17:24.173Z
---

Researched 2026-08-01. **DECISION: a real web browser on wandr lives HOST-side as a
native web engine / WebView (Proposal B below), NOT a wasm guest.** The web content
is not portable wasm; the guest is just chrome. A separate, portable **Blitz** lane
renders HTML/CSS *content* (no full browser) as a guest.

## Why a browser can't be a wasm guest (the walls)
1. **JS engine.** Real browsers JIT JS to *native* (V8/SpiderMonkey/JSC = C++, not
   wasip2). Pure-Rust engines compile to wasm but run INTERPRETED (no JIT in the
   sandbox). Best pure-Rust engine = **Brimstone** (Hans-Halverson/brimstone; >97%
   test262, ES2026 + Temporal, MIT, active) — beats Boa, but bytecode-VM
   interpreter (V8-Ignition-style), **engine-only (no DOM)**, wasip2 build unproven.
2. **"JS engine ≠ browser."** Even a perfect engine lacks the **Web-Platform / Web-IDL
   surface** (DOM, CSSOM, events, Fetch, Canvas, Storage… thousands of APIs) that must
   be implemented in Rust and BOUND to the engine — that binding layer is most of what
   Gecko/Servo are. Multi-year hand-build; out of reach for a guest.
3. **JS↔DOM must be co-located.** It's the hottest interface (millions of
   get/set/traverse/dispatch per interaction); browsers keep JS + DOM in ONE heap for
   near-zero cost. A wasm boundary there = RPC-per-property = catastrophic. So
   "host-side JS / guest-side DOM" is the WORST seam. "Host JS" only makes sense if the
   DOM/CSSOM/layout go host too = a host web engine = WebView. The real cut is
   **web-engine / browser-chrome**, not JS / DOM.

## On-the-fly JS→wasm JIT: doesn't exist, structural mismatch (asked + ruled out)
- Exists: **compile the ENGINE to wasm, interpret** (Boa/Brimstone/Javy-QuickJS); and
  **AOT JS-subset→wasm** (`Porffor`, build-time, no full dynamism; AssemblyScript ≠ JS).
- Does NOT exist: **runtime JIT of JS→wasm.** Reasons it's a bad fit, not just unbuilt:
  (a) a wasm guest can't instantiate new wasm itself — needs a host "compile+run wasm"
  capability (nested engine, exotic, rarely granted); (b) JS speed needs speculation +
  **deopt** with cheap in-place native patching — emitting/**re-instantiating whole wasm
  modules** is the wrong granularity, churn eats the win; (c) economics invert — you'd
  only want it in a codegen-forbidding sandbox (our guest), but there the host cost
  bites; where native codegen is allowed (browsers) you JIT to native directly.
- Our host runs wasmtime/Cranelift so it *could* offer wasm-compile — but it wouldn't
  help: if paying for speed, put the WHOLE engine host-side (Proposal B). Guest-JS
  ceiling = a good interpreter (Brimstone); real JS speed = host engine.

## Core components — where they go
| Component | Rust option | A: viewer (guest) | B: browser (host) |
|---|---|---|---|
| Net/cache/cookies | — | Host (`wandr-reqwest`/wasi:tls) | Host engine |
| HTML parse→DOM | html5ever/blitz-dom | Guest | Host |
| CSS cascade | **Stylo** | Guest | Host |
| Layout flex/grid + inline | Taffy + Parley | Guest | Host |
| DOM live tree | blitz-dom | Guest | Host |
| JS + Web-API bindings | Brimstone + Web-IDL | Guest (interp, light) or omit | Host (JIT, full) |
| Text shape/fonts | Parley/swash | Guest shape → Host fonts | Host |
| Paint→draw | blitz-paint | **Guest → `wasi:canvas`** | Host (WebRender) |
| Image/media decode | — | Host (`wandr:video`/image) | Host |

## Proposal A — Blitz content viewer (GUEST, portable) — the buildable lane
One wasip2 guest = **Blitz** (`DioxusLabs/blitz`: blitz-dom + html5ever + Stylo +
Taffy + Parley + blitz-paint), optionally Brimstone for light JS. Rides EXISTING host
services via `blitz-traits` seams: Renderer→`wasi:canvas`, Net→`wandr-reqwest`
(wasi:tls); fonts/decode/input→existing WIT. **No new host contract, no WIT change.**
Same integration shape as the Floem port ([[reference_floem_wandr_candidate]]). Delivers
real HTML/CSS (articles, email, RSS, docs, reader-mode, HTML-as-UI) + light JS widgets —
NOT heavy web apps. Also the native-DOM engine that lets web-style UI target wandr.
Unknowns = wasip2 builds of Stylo (Servo-derived, big — keep Gecko-FFI features off) +
Brimstone; Floem-style spike. Blitz status: pre-alpha but capable.

## Proposal B — Real browser via host-native engine / WebView (DECIDED path)
The WHOLE engine lives HOST-side (native JIT JS + DOM + Stylo + WebRender + media),
exposed as a **web-surface service** behind a NEW `wandr:webview`-style WIT contract
(navigate / input-forward / load-events / surface-handle) — **contract needs approval
when pursued (rule #4)**. The GUEST is just browser chrome (tabs/URL bar) in
dioxus/Slint, compositing the host surface — same surface model as `wandr:video` /
the `sf_media` child surface ([[reference_wasi_webgpu_gfx]]). Full web at speed. Cost:
host carries a native engine, **per-OS** — **Servo/Verso** (Android/Linux), WKWebView
(macOS), WebView2 (Windows). Integration work, not engine-building (Servo has embedding
APIs). Off the portable-wasm thesis for the *content*, on it for the chrome.

Related: [[reference_wasi_webgpu_gfx]] (two-lane model; Blitz on wasi:canvas, GPU→wasi:webgpu),
[[reference_floem_wandr_candidate]] (same pluggable-renderer spike shape),
[[reference_slint_wasip2]]/[[reference_dioxus_taffy_rust_ui]] (chrome UI),
[[reference_wandr_wasi_tls_transport]] (net), [[project_wandr_video_host.md]] (surface-composite precedent).
