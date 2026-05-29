# subsecond (vendored + patched for wasm32-wasip2) — task 60

This is a **vendored copy of the upstream `subsecond` crate v0.7.9**
(from crates.io, part of [DioxusLabs/dioxus](https://github.com/DioxusLabs/dioxus)),
with **one tiny patch** so that dioxus ≥0.7 builds for `wasm32-wasip2`
(our system-component guest target). It is *not* a git submodule — it's a
deliberate vendored patch held until the fix lands upstream.

## Why

`dioxus-core 0.7` has a **non-optional** dependency on `subsecond` (its
hot-patch engine). Upstream subsecond declares its browser deps
(`wasm-bindgen`, `js-sys`, `web-sys`, `wasm-bindgen-futures`) under
`cfg(target_arch = "wasm32")`, which matches **both** `wasm32-unknown-unknown`
(browser) **and** `wasm32-wasip2`. The only consumer is `apply_patch`
(`src/lib.rs`), a browser-only dev hot-reload (`web_sys::window().fetch()`).
On `wasm32-wasip2` those wasm-bindgen `__wbindgen_*` intrinsics have no host
and don't link under `wasm-component-ld` → the guest build fails. No feature
disables subsecond, so dioxus 0.7+ is unbuildable for our target without this.

## The patch (the *only* diff vs upstream 0.7.9)

Narrow the wasm `cfg` from `target_arch = "wasm32"` to
`all(target_arch = "wasm32", target_os = "unknown")` — i.e. browser
(`target_os = "unknown"`) only, never wasip2 (`target_os = "wasi"`):

- `Cargo.toml`: the 4 browser deps move from
  `[target.'cfg(target_arch = "wasm32")'.dependencies.*]` to
  `[target.'cfg(all(target_arch = "wasm32", target_os = "unknown"))'.dependencies.*]`.
- `src/lib.rs` (the `apply_patch` wasm branch): `#[cfg(target_arch = "wasm32")]`
  → `#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]`.

On wasip2, `apply_patch` then has neither its `#[cfg(any(unix, windows))]`
branch nor the wasm branch and falls through to its existing trailing `Ok(())`
— i.e. hot-patching is a no-op, which is correct: wart never hot-reloads a
packaged guest.

## How it's wired

Each dioxus-0.7 guest adds, in its own `Cargo.toml` (`[patch]` is per
build-root, and our guests are standalone crates):

```toml
[patch.crates-io]
subsecond = { path = "<relative path to external/subsecond>" }
```

## Upstream

The same `cfg` narrowing has been proposed upstream (it helps every
WASI/non-browser wasm consumer, not just wart). Once it ships in a released
dioxus, delete this directory and the `[patch.crates-io]` lines. See
`tasks/60-dioxus-0.7-wasip2-subsecond.md`.
