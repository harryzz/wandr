---
name: reference-photon-image-lib
description: "Image-processing library for guests — photon-rs, proven to fit wasm32-wasip2"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

Need image manipulation (filters, resize, crop, color ops) in a wart guest/component?
Use **`photon-rs`** — proven to build AND run on `wasm32-wasip2` (our component
model) under wasmtime.

**Full write-up + working probe: `repros/photon-probe/` (README.md).** It's a
`wasi:cli/command` that runs encode→decode→effects→re-encode end-to-end (PASS).

Key points (detail in the README):
- `default-features = false` is MANDATORY — photon's default `enable_wasm` pulls
  `wasm-bindgen`/`web-sys`/`js-sys` (browser/JS), which break on wasip2. Off → pure
  Rust `image` + `imageproc`. (Same class of trap as [[reference_dioxus_07_wasip2_subsecond]].)
- Architecture: **`image` crate for codec** (PNG/JPEG decode/encode), **photon for
  effects** — photon's `native::image_to_bytes` returns raw pixels on wasm, so do
  I/O with `image` directly and hand RGBA to `PhotonImage::new`.
- ~1.3 MB component (png-only). Pulls old `getrandom 0.1.16` (fine unless you use
  rand-based effects like noise — verify a wasip2 backend then).
- Not yet wrapped as a real component — README sketches the `my:photon/process`
  WIT (`apply(bytes, ops) -> bytes`); link into host or `wac plug` under a guest.
