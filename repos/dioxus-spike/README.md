# dioxus-spike — feasibility probe (task 57 follow-up)

**Question:** is `dioxus-core` + `taffy` a viable *light* reactive Rust UI
framework for wandr guests (a Compose alternative), given our rendering
boundary — the guest has no DOM / no WebView / no GPU, only the
high-level host-Skia **canvas WIT**?

**Answer: yes, the foundation is viable.** Verified 2026-05-29:

| Check | Result |
|-------|--------|
| Compiles to `wasm32-wasip2` | ✅ — `dioxus 0.6` (`default-features=false`, features `macro,html,signals,hooks`) + `taffy 0.7`. **No `wasm-bindgen`** pulled (the main risk — `dioxus-web` is the only thing that needs it, and it's off). |
| Binary size | **424 KB** (release, `opt-level="s"`, lto, stripped) — framework + reactive component + layout engine. ~37× smaller than Kotlin/Compose (15.7 MB). |
| Runs under `wasmtime` | ✅ — `VirtualDom::rebuild_to_vec()` produces the mutation list; `taffy` computes correct flexbox geometry (`tile_b` at x=168 = 132 + 36 gap). |
| Reactive model | ✅ — `rsx!`, `use_signal`, `onclick` all compile + work. |

Run it:
```
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/dioxus-spike.wasm
```

## What this de-risks (and what it doesn't)

**De-risked:** the heavy unknowns — these crates build for our target,
they're light, and the reactive API works headless.

**Still to build (the real one-time framework work, NOT done here):** a
**custom renderer** — "a tiny Blitz for our canvas WIT":
1. consume the `VirtualDom` mutations → maintain a node arena;
2. map element tags/attributes → `taffy::Style`s; run `compute_layout`;
3. walk the laid-out tree → emit canvas-WIT verbs (`draw-rrect`,
   `create-text-blob`/`draw-text-blob`, host fonts for text);
4. route host `on-pointer-event-v2` back into the VirtualDom as dioxus
   events; re-render on signal change.

**Do NOT use full `dioxus-native`/Blitz** — it pulls Servo's stylo +
parley + **vello/wgpu** (GPU we don't have) and is megabytes. The light
path is `dioxus-core` + `taffy` + our own canvas-WIT painter.

## Recommendation

This is the leading candidate for a general light reactive Rust UI
framework on wandr, for when a *complex* Rust guest is needed (rich status
bar, settings app). For simple system UI (the launcher, a basic taskbar),
hand-rolled canvas-WIT drawing stays lighter (see `wandr.launcher`, ~70 KB).
See `tasks/57-launcher.md` and [[feedback_no_art_layer_dependencies]].
