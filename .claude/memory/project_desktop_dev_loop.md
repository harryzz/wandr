---
name: project_desktop_dev_loop
description: "✅ Desktop dev loop (task 101, 2026-06-11): x86_64 wandr-host runs the SAME guest wasm as the device in a winit window (WSLg-verified) — WANDR_DESKTOP_SIZE=WxH + JIT; gotchas: desktop present was MISSING (softbuffer fix), on-resize wasn't dispatched (masqueraded as 3 render bugs), key-input v3 = W3C codes+modifiers"
metadata: 
  node_type: memory
  type: project
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Desktop dev loop — DONE + user-verified 2026-06-11 (task 101; recipe in
`docs/build-pipeline.md` "Desktop dev loop"; full narrative in
`tasks/101-desktop-dev-loop.md`).**

```bash
WANDR_DESKTOP_SIZE=500x1000 \
  runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host \
  apps/user/<app>/components/ui.wasm
```

- **Same wasm binary as the device** (JIT desktop / AOT device), render
  parity verified (Slint demo + dioxus differential), mouse + HW keyboard
  (modifiers!) + live resize + color emoji.
- **Why:** edit → build wasm → run locally → install the identical artifact
  on the phone. Also the live portability proof for the wasi-canvas idea.

**Hard-won gotchas:**
- The desktop renderer NEVER had a present path (`flush_and_swap` was
  android-only; offscreen raster + ok=true + black window forever) →
  softbuffer blit (N32-premul BGRA → 0RGB u32 LE), no GL.
- ‼️ Desktop didn't dispatch `on-resize` to guests — ONE gap that
  masqueraded as THREE draw bugs (gradient black, slider track + ListView
  rows missing): the guest laid out for the stale size. On "missing UI
  elements", suspect layout-vs-surface size mismatch FIRST; localize with
  a differential guest (dioxus demo rendered fine → host canvas clean).
- `my:skiko-gfx/key-input` (v3): W3C UIEvents code TOKEN (string —
  winit KeyCode Debug names ARE the tokens) + alt/ctrl/meta/shift + text +
  repeat; probe-only world (frame-pacing pattern, additive); hosts emit
  v1+v2+v3 from winit/InputFlinger/IME; consumers keep v2 as fallback
  until the first v3 arrives. HW-keyboard user-verified.
- Desktop x86_64 build had silently rotted (task-93 aarch64 cpufeatures +
  ungated android module refs) — keep `cargo check --target
  x86_64-unknown-linux-gnu` honest when touching the host.
- Emoji on desktop: `/usr/share/fonts/truetype/noto` preopened as
  `/system-fonts` (device parity); test with STATIC emoji in UI text
  (input-free — no keyboard can type emoji).
- Agent screenshot recipe + pkill-self trap: see the task file
  "Known desktop non-goals / edges".
- No arbiter on desktop: IME warns (no overlay), insets 0; scroll-wheel
  unmapped. [[reference_slint_wasip2]] [[project_wasm_runtime]]
