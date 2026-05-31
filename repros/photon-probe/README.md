# photon-probe — does `photon-rs` fit the wart component model?

De-risk probe (like `signal-phase0`). **Verdict: YES.** `photon-rs` 0.3.3 builds
*and runs* on `wasm32-wasip2` (our component-model target) and processes images
end-to-end under wasmtime.

## Run

```
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/photon-probe.wasm
# → encode PNG → decode → grayscale/contrast/filter/sepia/resize → re-encode
#   RESULT: photon effects + image codec on wasm32-wasip2 = PASS ✓
```

## What we learned

- **The wasip2 trap is avoidable.** photon's default feature `enable_wasm` pulls
  `wasm-bindgen`/`web-sys`/`js-sys`/`node-sys` — a browser/JS runtime that does NOT
  work on a wasip2 component. Set **`default-features = false`** and those are gone;
  the core uses the pure-Rust `image` + `imageproc` crates, which build cleanly.
- **Effects run.** grayscale, adjust_contrast, filters::filter, monochrome::sepia,
  transform::resize all execute correctly under wasmtime.
- **Use `image` for codec, `photon` for effects.** photon's own
  `native::image_to_bytes` returned *raw* pixels here (not PNG), so do PNG/JPEG
  decode/encode with the `image` crate directly (`load_from_memory` /
  `DynamicImage::write_to`) and hand the raw RGBA to `PhotonImage::new`.
- **Size:** ~1.3 MB component (image with `png` only; more formats → bigger).
- **Caveat:** photon pulls an old `getrandom 0.1.16` (compiles fine). The effects
  tested don't use it; rand-based ones (e.g. noise) may need a getrandom backend
  configured for wasip2 — verify before relying on them.

## To make it a real wart component (next step, not done here)

Wrap with a WIT interface, e.g.:

```wit
package my:photon@0.1.0;
interface process {
  // encoded image bytes in (PNG/JPEG), op list, encoded bytes out
  apply: func(image: list<u8>, ops: list<string>) -> result<list<u8>, string>;
}
world photon { export process; }
```

Then either link it into the host, or `wac plug` it under a guest that wants
image processing — same pattern as the Signal engine/ui split.
