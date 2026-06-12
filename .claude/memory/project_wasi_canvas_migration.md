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
- frame bracket (post canvas-context alignment, commit b2f8dc00):
  `get-context() -> canvas-context { graphics, get-current-buffer,
  present }` — wasi-gfx graphics-context idiom; guests keep a lazy
  thread_local context (`wctx`/`__wc_ctx` helpers) and bracket frames
  with `get-current-buffer()` … `present()`. Dims from canvas handle.
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
✅ ROOT-CAUSED + FIXED (2026-06-11, wandr 9e7a8fef + ~/xl/kotlin
f0f3496ab748): Kotlin/Wasm wraps EVERY @WasmExport in an epilogue that
calls kotlin.wasm.internal.invokeOnExportedFunctionExit on
outermost-export exit — INCLUDING cabi_realloc. Host lowers export-arg
strings → cabi_realloc's epilogue pumps kotlinx-coroutines' queued jobs
(live Compose always has some) while realloc memory is pending →
withScopedMemoryAllocator in pumped code throws "Can't create new
allocators while realloc-allocated memory is not freed" → escapes
uncatchably (compiler-generated epilogue, outside all user code) →
poisons the instance. Found by reading the component WAT (wasm-tools
print; the trap-classifier trick — distinct trap flavors as a message
channel — proved the catch never ran). FIX: stdlib fork
internalCallback.kt skips the pump while isComponentModelReallocPending();
republish kotlin-stdlib-wasm-wasi 2.4.258-SNAPSHOT
(`./gradlew :kotlin-stdlib:publishWasmWasiPublicationToMavenLocal` in
~/xl/kotlin). Kotlin guests now export ALL THREE handlers; IME typing
device-verified through key-handler. UPSTREAM-RELEVANT: hits anyone
combining kotlinx-coroutines with host-lowered export args (JetBrains
wit-bindgen flow included) — candidate for a KT issue + the KT-86415
thread.
Debug affordances added: desktop host `--app <id>` (resolves cross-app
deps from WANDR_APPS_ROOT; pull dep components from the phone — wasm is
target-independent) + desktop `--install`; ime-inbound key failure log
prints `{e:?}`.

**Kotlin wasi:canvas migration (task 102) — Stage 1 DONE 2026-06-11:**
`apps/user/wandr.ktcanvas.test` = bare-Kotlin spike with bindings GENERATED
by the Kotlin wit-bindgen fork (rev 6b9cb12, recipe in
[[wit-bindgen-no-kotlin-generator]]) — imports wasi:canvas
{types,draw,layout,embedding}, exports my:skiko-gfx/renderer, all from ONE
generator pass, ZERO hand-written ABI code. Desktop + device verified
(~1-2ms/frame desktop, RSS flat under per-frame paragraph/canvas churn).
Every hot ABI shape proven: spilled paint blobs, shader borrow-in-record,
gradient list<tuple>, SVG-path strings, mask-blur option<record>,
lines() list<record> lift, offscreen→snapshot→draw-image, resource close().
**Stage 2 DONE 2026-06-11 (same day): Compose geometry on wasi:canvas,
device-verified.** The skiko main canvas routes transforms/clips/geometry
draws + gradient shaders through generated wasi:canvas bindings
(skiko `generated/wasicanvas/`, world = `wit-canvas/world.wit`, regen note
inside; generated runtime's `cabi_realloc` MUST be deleted after regen — the
app's RendererExports.kt exports it) behind
`org.jetbrains.skiko.wasi.WasiCanvasBackend.enabled` (wandr-app main() flips
it; default false = legacy bit-for-bit). KEY ARCHITECTURAL FACT that makes
per-verb migration safe: legacy AND wasi verbs hit the SAME
`renderer.canvas()` (canvas_impl.rs:809 routes into the active legacy
recording stack) — so wasi draws are captured by WasiDrawable/RenderNode
recordings, state (matrix/clip/saves) is shared, and any call can fall back
per-paint. Routing rule (`WasiCanvas.wc()`): legacy when paint has
color-filter (not in draft) or a legacy-only shader (image shaders);
outside the frame bracket frameCanvas==null → legacy (composition-time
rasterization just works). Shader = dual-handle (legacy id always + wres
when enabled, both dropped in discard()). Frame bracket swaps wholesale
(canvas-context ≡ legacy begin/end-frame host-side). Legacy-kept: text
blobs/paragraph (stage 3), images + bitmap-canvas, pictures/drawables,
set/reset-matrix. Verified: desktop (full demo screenshot) + Pixel 2 XL
(counter taps, scroll-over-recordings, RSS flat). wandr.ime.keyboard:
world got the imports + new build componentizes, but the on-device
component is intentionally the OLD build (no behavior change with flag
off; redeploy needs a zygote restart — ride along with the next one).
**Stage 3 DONE 2026-06-12: Compose text on wasi:canvas/layout +
the ONE batched WIT break, deployed everywhere.** Draft changes
(proposals/wasi-canvas): paint += `filter: option<color-filter>`
(variant blend(color-blend{color,mode})/invert — host honors the mode,
unlike legacy's hardcoded Modulate), line-metrics extended to the full
13-field editor shape, paragraph += ideographic-baseline + line-count,
rects-for-range REPLACED by selection-boxes(start,end,height-style,
width-style)->list<text-box{rect,direction}> (no callers existed).
WIT copies live in FIVE extra places: wandr-app + ime.keyboard +
ktcanvas.test wit/deps + skiko wit-canvas/deps + **dioxus-canvas
launch_wasi.rs INLINE string** (slint-wandr generate! points at
proposals directly — no copy). skiko: paragraph/{Paragraph,
ParagraphBuilder}.kt dual-path (wasi-only resource when enabled —
paragraphs aren't dual-handle like shaders; builder nulls wres on
build so close() can't drop handle 0; maxWidth = guest-remembered
layout width). Parity choices (deliberate, revisit in a fidelity pass):
lineHeight=0, align=START (legacy ignored ParagraphStyle entirely —
mapping align would double-align), colorFilter still routes legacy
until images migrate. Rebuilt + deployed: host (desktop+android),
9 Rust guests + Signal (state backed up), skiko→compose×9→wandr-app +
ime.keyboard (keyboard redeployed this round — zygote restarted via
run-hybrid-stack --wandr-only; plain restart bails on home-resolution
under --no-art). Device user-verified (chrome boot, wandr-app text/
fields, Signal). GOTCHA: use `adb shell` directly (adbd is root) — NOT
`su -c` (the Magisk am-spin trap, [[reference_artoff_magisk_am_spin]]).
Stage 4 remaining: images/bitmap-canvas → wasi (ATOMIC Image flip —
images must not be dual-handle, double pixel memory; color-filter
adoption rides along), then retire my:skiko-gfx canvas+paragraph from
the Kotlin worlds (my:skiko-gfx keeps window/IME/lifecycle + WasiDrawable
layers + text blobs until their own story). Gotchas: WSLg Wayland resets the
desktop host connection after a few seconds — use `WINIT_UNIX_BACKEND=x11
WAYLAND_DISPLAY=`; screenshot via `xwd -name "WASM Android Runtime"` +
ffmpeg (gnome-screenshot/xwd-root fail under WSLg); on-disk
wandr.slint.test components/ui.wasm predates the canvas-context realignment
and no longer instantiates (rebuild before using it as a desktop reference).

**FINAL DESIGN STATE (2026-06-12) — the user-fixed GOAL: architecturally
clean, NO overlapping functionality, WASI-acceptable, 100% consumable by
the reference libraries.** Both contracts redesigned and acceptance-
checked (NOT wired): proposals/wasi-canvas/REDESIGN-0.0.2.md (§11 =
overlap audit + final 4-criteria check, all PASS; one documented R5
exception: `clear`) and proposals/wasi-input-handlers/REDESIGN-0.0.2.md
(0.0.1 pointer-event FAILED the six-consumer union — missing
button/buttons/device/tilt/twist + enter/leave kinds, all
breaking-class; fixed shapes wasm-tools-validated). SEQUENCING DECISION
(path B): implement BOTH 0.0.2 packages side-by-side first (scene = a
re-skin of the host WasiDrawable machinery), THEN one single Kotlin
finale (images + drawables→scene + blobs→paragraphs + setMatrix→guest
matrix tracking) — NOT stage-4-on-0.0.1 (would touch skiko twice).
After that event both packages are FROZEN: evolution = additive methods
(R2) or side-by-side versions (R3) only.

**THIRD DESIGN (2026-06-12): proposals/wasi-surface/DESIGN.md** — OWNERSHIP-CORRECTED: surface/graphics-context are UPSTREAM-owned (wasi-gfx org); our doc = the change-set we'd propose + design record, NOT a wandr package (no version-lineage claim; ship-before-upstream fallback = wandr:surface@0.0.1) —
the socket model de-floated: wasi:surface + wasi:graphics-context 0.0.2
(capability-granted context — fixes upstream's ambient constructor; 
request/configure geometry; optional pull-profile pollables), four
producer types (webgpu/frame-buffer upstream, canvas third, video
fourth), fused-form equivalences documented (canvas embedding ≡
primary-surface.get-context; video placement verbs = the factoring
lane). Design-only; triggers: engine-class guest, video factoring, or
upstream conversation. The family is now THREE designed proposals under
one goal: canvas, input-handlers (+gesture-handler), surface/socket.

**The four REFERENCE LIBRARIES (validation set, user-fixed 2026-06-12):
skiko-compose, dioxus, slint, Avalonia UI** — every wasi:canvas contract
decision must be cross-checked against ALL FOUR (they span the ownership
axes; 0.0.1 broke by validating against only the easy three). Design
rules + redesign live in proposals/wasi-canvas/REDESIGN-0.0.2.md
(R1 union-sized records / R2 additive methods / R3 record-change =
version event / R4 binding floor / R5 no derivable verbs — funcC
composable from funcA+funcB at no capability/wire cost stays OUT, in
guest adapters). Avalonia gotcha for any set-transform debate:
IDrawingContextImpl.Transform is an absolute SETTER per visual.

Related: [[reference_slint_wasip2]], [[feedback_shared_wit_rebuild_all_consumers]].
