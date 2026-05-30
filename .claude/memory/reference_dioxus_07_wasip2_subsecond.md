---
name: reference_dioxus_07_wasip2_subsecond
description: "dioxus >=0.7 won't build for wasm32-wasip2 because dioxus-core hard-deps subsecond which pulls wasm-bindgen on all wasm32; clean fix = narrow subsecond's cfg to target_os=unknown. Proven. See task 60."
metadata: 
  node_type: memory
  type: reference
  originSessionId: d04324f7-02b4-4277-bb43-167a1ccfb82b
---

**dioxus 0.7+ does not build for `wasm32-wasip2` out of the box** — this is why
[[reference_dioxus_taffy_rust_ui]] / task 59 pin **dioxus 0.6**.

Root cause (NOT "component model unsupported"): `dioxus-core 0.7` has a
**non-optional** dep on `subsecond` (hot-patch engine; dioxus-core has only a
`serialize` feature, no way to disable subsecond). `subsecond` declares
`wasm-bindgen`/`js-sys`/`web-sys`/`wasm-bindgen-futures` under
`[target.'cfg(target_arch = "wasm32")'.dependencies]` — which matches wasip2 too,
even though the only consumer (`apply_patch`, `lib.rs:550`, a `web_sys::window().fetch()`
browser hot-reload) is browser-only. wasm-bindgen's `__wbindgen_*` intrinsics
don't link under `wasm-component-ld` → guest build fails. `main` is NOT fixed
(checked 2026-05-29). 0.6 was clean only because there the sole wasm-bindgen
source was `dioxus-web`, which we keep off.

**Clean fix (proven 2026-05-29 — links + valid component, no wasm-bindgen):**
narrow subsecond's wasm `cfg` from `target_arch = "wasm32"` to
`all(target_arch = "wasm32", target_os = "unknown")` (browser=unknown,
wasip2=wasi). 2 edits: the 4 browser deps in subsecond `Cargo.toml` + the
`#[cfg]` at `lib.rs:550`. `apply_patch` then falls through to its trailing
`Ok(())` on wasip2 (hot-patch = no-op, correct — we never hot-reload guests).
Mechanism for us: `[patch.crates-io] subsecond = { path = … }` to a vendored/forked
patched subsecond (per-build-root, so the line goes in each dioxus-0.7 guest).
Upstreamable PR (cfg narrowing helps all WASI users).

dioxus-canvas renderer core is otherwise 0.7-compatible — only `events.rs` needs
`convert_cancel_data` + a `Modifiers` import path tweak.

**ADOPTED 2026-05-29 (task 60, option B):** the patched crate is vendored at
`external/subsecond/` (+ README); `crates/dioxus-canvas` and
`apps/user/war.dioxus.demo` are on dioxus **0.7** with
`[patch.crates-io] subsecond = { path = … }` (the `[patch]` line is repeated in
each guest — it's per build-root). Device-verified on 0.7.9 (renders; 516 KB; no
wasm-bindgen). **How to apply to a NEW dioxus guest:** depend on dioxus 0.7 +
`dioxus-canvas`, and add the same `[patch.crates-io] subsecond = { path =
"<rel>/external/subsecond" }` to its Cargo.toml. Drop the vendor + patch lines
once the cfg-narrowing lands in a released dioxus. See
`tasks/60-dioxus-0.7-wasip2-subsecond.md`.

**Upstream PR submitted 2026-05-29:** [DioxusLabs/dioxus#5602](https://github.com/DioxusLabs/dioxus/pull/5602)
(fork `harryzz/dioxus`). If/when merged + released, drop `external/subsecond/`
+ the `[patch.crates-io]` lines.
