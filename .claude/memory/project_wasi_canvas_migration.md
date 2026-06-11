---
name: project_wasi_canvas_migration
description: "✅ wasi:canvas + wasi:input-handlers drafts (proposals/) shipped 2026-06-11 — host impl default-on; Slint/dioxus(+Signal)/all-4-chrome migrated + device-verified; my:skiko-gfx canvas left to Kotlin/Compose only; wit-bindgen multi-package gotchas (path order, qualified world, generate_all, textually-scoped export!)"
metadata: 
  node_type: memory
  type: project
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Status (2026-06-11): the wasi:canvas draft is the production 2D path for
every non-Kotlin guest.** Drafts live in `proposals/wasi-canvas` (types /
draw / glyphs / layout / embedding) + `proposals/wasi-input-handlers`
(pointer/key/frame handlers; host routes EXCLUSIVELY to bound handlers,
legacy renderer events only when unbound). Host impl:
`runtime/wandr-host/src/wasi_canvas_impl.rs`, cargo feature `wasi-canvas`
DEFAULT-ON; host serves my:skiko-gfx AND the drafts simultaneously.
Consumers migrated + device-verified (--no-art): slint-wandr (proving
consumer), dioxus-canvas dual backend (`launch_wasi_canvas!` — demo,
taskmanager, connectivity.test, Signal w/ history intact), and all 4
hand-rolled chrome guests (launcher/statusbar/taskbar/keyguard — verified
boot-to-home, lock + swipe-up unlock, tile-tap launch, taskbar home).
Legacy `my:skiko-gfx` canvas consumer class remaining: Kotlin/Compose only.
Roadmap + design notes: `proposals/wasi-canvas/README.md` + COMPATIBILITY.md.

**Why:** one standardizable 2D contract instead of the private skiko WIT;
layout paragraphs carry REAL metrics, which retired every per-glyph-advance
centering/truncation hack in chrome (and earlier caught the legacy
alpha-REPLACE paint bug via Slint).

**How to apply (porting a raw wit_bindgen guest to the drafts):**
- world wit: delete the local `interface canvas` copy; world imports
  `wasi:canvas/{types,draw,layout,embedding}@0.0.1` (layout only if text).
- `generate!`: world must be QUALIFIED (`"my:skiko-gfx/<world>"` — multiple
  main packages), `path: ["../../../proposals/wasi-canvas/wit", "wit"]`
  (proposal root FIRST — resolution order matters), and `generate_all`
  (else "missing with mapping for wasi:canvas/types").
- frame bracket: `let cv = wembed::begin_frame(); … drop(cv);
  wembed::end_frame();` — dims from `cv.width()/height()`.
- text: Para{p,baseline,width,height} built from
  `layout::ParagraphBuilder`; paint at top-left origin (`y_baseline -
  baseline` when matching old blob baselines).
- wit-bindgen's `export!`/`pub_export_macro` is TEXTUALLY scoped: the
  export! invocation + Guest impls must live INSIDE the same module as the
  generate! (see `wire_wasi_canvas!`'s `mod __dioxus_input`).
- evdev tap/swipe injection for --no-art verification:
  `/data/local/tmp/{tap,swipe-up}.sh` (sendevent type-B on
  /dev/input/event1; `adb shell input` is dead without ART).
- ‼️ reinstalling a `kind=system` app needs a ZYGOTE RESTART: the zygote
  auto-preloads every system bundle at startup, and forks after a
  reinstall run the STALE preloaded image against the new cwasm →
  SIGSEGV in a lower-import trampoline (a new face of
  [[reference_missing_instance_error_stale_zygote]]). User apps
  (apps/*) are not preloaded and reload fresh. Also: never debug-launch
  a second instance with `wandr-host --standalone --app X` while the
  hybrid stack runs — it creates an unmanaged full-screen surface that
  steals focus (looks like system-wide lag + dead taskbar).
**Kotlin/Compose input migration (2026-06-11, commit 0fdd098f):**
wandr-app + wandr.ime.keyboard export pointer-handler + frame-handler
(device-verified: clicks, IME, typing). TWO HARD-WON FACTS:
(1) the Kotlin pointer wrapper MUST fan out to BOTH
`RendererImpl.onPointerEvent` (v1 — the only path that feeds the Compose
scene) AND `onPointerEventV2` (opt-in WasiInput handler, silently
discarded when unregistered) — v2-only delivers but produces no clicks.
(2) ‼️ key-handler is NOT exportable to Compose guests yet: host lowering
of the key-event strings into a LIVE Compose app throws an exception
Kotlin `catch(Throwable)` cannot intercept (escapes even from inside
cabi_realloc's own catch; componentModelRealloc's guards exonerated by
source-read), poisoning the instance ("cannot enter component instance",
app exits). The clean-room spike passes 100k/100k, so the trigger is
Compose-app runtime state — OPEN follow-up; host auto-falls-back to
legacy on-key-event-v2 when key-handler is unbound, so typing works.
Debug affordances added: desktop host `--app <id>` (resolves cross-app
deps from WANDR_APPS_ROOT; pull dep components from the phone — wasm is
target-independent) + desktop `--install`; ime-inbound key failure log
prints `{e:?}`.

Related: [[reference_slint_wasip2]], [[feedback_shared_wit_rebuild_all_consumers]].
