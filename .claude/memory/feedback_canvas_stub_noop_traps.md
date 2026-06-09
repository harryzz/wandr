---
name: canvas-stub-noop-traps-compose
description: "RESOLVED 2026-05-19 via task 28 Path D. Original lesson preserved below: don't no-op the throwing stubs — Compose corrupts save/restore state and SIGILLs. Task 28 wired the stubs to host-side skia raster surfaces via 38 bc-* WIT verbs, which is the proper fix."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

**RESOLVED 2026-05-19 via task 28 Path D.** The 41 throw-stubs in
`SkiaTypes.wasi.kt`'s `Canvas` class now dispatch to host-side raster
surfaces via new `bc-*` WIT verbs (see
`tasks/28-skiko-abstract-canvas.md` Closeout). SegmentedButton's
checkmark + label rendering and DatePicker's grid both work
end-to-end. The historical cautionary tale below is preserved because
the underlying mechanism (no-op `save()` breaking Compose invariants
→ wasm `unreachable` → SIGILL) is still true; the resolution is just
"don't no-op, implement properly" — which task 28 did.

---

When Material3 widgets (DatePicker, SegmentedButton, plus parts of
Slider, AlertDialog, BottomSheet) crash with `Canvas.save: not
implemented` from `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/
SkiaTypes.wasi.kt`, the temptation is to convert all 41
`error("Canvas.X: not implemented")` stubs into no-op returns
(`this` for Canvas-returning, `0` for the save-count returners).

**Don't.** Tested 2026-05-18: cold start passes, the widgets render
their primary content, app works for ~10-30 seconds, then SIGILL in
JIT'd wasm after the user interacts. Tombstone shows a single frame
in `<anonymous:JIT region>` with PC at offset +0x260006c, registers
that look like normal data — i.e. the wasm guest hit an
`unreachable` instruction.

**Why:** Compose internally allocates intermediate `Canvas`
instances (PictureRecorder, bitmap-backed Canvas for layer draw)
and threads save/restore counts through state machines that expect
real semantics. With no-ops:
  * `save()` always returns 0 — so save counts never increase
  * `restoreToCount(0)` no-ops
  * Drawing methods silently discard output
But Compose's `GraphicsLayer` / drawing pipeline checks invariants
(e.g. `check(saveCount == expectedDepth)`, or relies on `getMatrix()`
agreeing with applied transforms). Those checks fail → Kotlin's
`error()` / `check()` lower to wasm `unreachable` → SIGILL.

The throwing stubs, by contrast, crash LOUDLY and EARLY (cold
start render_frame #0) so we know which widgets to disable. Silent
no-ops let Compose appear to function then crash on interaction
with no useful diagnostic.

**How to apply:** keep the stubs throwing. Disable widgets that hit
them (currently DatePicker, SegmentedButton) and document them as
"needs Canvas.save wired through to host-side skia-safe" follow-up.
The proper fix is non-trivial: it requires recording save/restore
state in `WasiCanvas` per-instance, threading it through WIT to a
host-side `skia_safe::Canvas` we maintain in parallel, and having
the abstract Canvas instances either share that path or have their
own host-side counterparts. Either route is a real task, not a
quick stub.

When a Material3 widget hits this in the future:
  1. Add it to the disabled list in
     `wandr-app/src/wasmWasiMain/kotlin/RealComposeApp.kt`'s
     `MaterialDemoApp`, with a comment pointing at this memory.
  2. Don't even try the no-op shortcut.

Affected widgets observed so far: `DatePicker`, `SegmentedButton`,
`TimePicker` (assumed, by analogy — untested).

Confirmed-clean widgets that don't hit it: `Button`, `Checkbox`,
`Switch`, `RadioButton`, `Slider` (without steps animation),
`DropdownMenu` (with `LocalInspectionMode provides true` workaround),
`FilterChip` / `AssistChip` / `SuggestionChip` / `InputChip`,
`FloatingActionButton`, `ExtendedFloatingActionButton`, `TextField`
(BasicTextField + TextFieldState), `LazyColumn`, `Card`, `Snackbar`
(hand-rolled — `SnackbarHost` itself is untested).
