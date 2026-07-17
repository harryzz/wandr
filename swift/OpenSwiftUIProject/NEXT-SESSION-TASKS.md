# OpenSwiftUI-on-wandr — next-session tasks (structural cleanup, then blur)

> Ordered by dependency — each step unblocks the next. Full rationale is in
> `COMPONENTS-AND-BUILD.md` (§ references below). The frosted backdrop **blur** is deliberately
> LAST: it needs a `wasi:canvas` WIT verb + host change, which is far cleaner to do once the
> package layering is normalized. Do not start blur before the structure work.
>
> Current state (end of the 2048 effects session): clip/fill, 3D tilt, and drop shadow are all
> landed. The app (`repros/swift-canvas-spike`) still names the low-level layers directly
> (`CSwiftSpike`, `WandrCG`) — that's what these tasks fix.

---

## 0. Gesture / interaction bugs (user-facing — likely do FIRST)

Reported after the effects session. All cluster around **hit-testing using a flat layout `hitFrame`
that ignores render transforms** (`.offset`, modal positioning), plus **modals not blocking
background input**. Fragile subsystem: `EventBindingManager` / `GestureViewModifier` /
`HitTestBindingModifier` / the `ViewResponder` tree. Prior root-cause analysis is in
`repros/swift-canvas-spike/HANDOFF-eleev-openswiftui.md` ("REMAINING PROBLEMS") and the memory
[[reference_openswiftui_gestures_offapple]].

**READ FIRST — do NOT patch-and-cycle (this area burned days before):** trace how `.offset` →
GeometryEffect → (render vs event) transform flows, and how `ViewResponder.containsGlobalPoints`
already does transform-aware containment. The likely single fix is to route hit-testing through the
transform-aware responder tree instead of the flat `hitFrame` for offset/modal subtrees.

1. **Swipe registers above the board (outside it).** Board-drag `hitFrame` is greedy `(0,0,460,734)`
   — includes the header. Constrain the board DragGesture's hitFrame to the board square (commit
   c7a0a71b tried, didn't fully constrain).
2. **Board swipe intermittently freezes; recovers after tap-a-tile-then-swipe; a header-area swipe
   never freezes.** The board subtree's responders rebuild every tile-animation frame, dropping the
   in-flight drag binding. Keep the active drag's binding alive across animation re-renders.
3. **Modal buttons + menu items don't accept clicks (no Settings, no About).** Two causes:
   (a) modal buttons sit in `.offset(y:)`; the gesture `hitFrame` is the un-offset LAYOUT frame, so
   the visible button ignores taps (and the stale frame lands elsewhere — see bug 4). (b) Settings/
   About: the tap FIRES (menu closes) but `switch selectedView` doesn't swap the shown view — a
   wasm conditional-view metadata-ABI issue ([[reference_openswiftui_conditional_wasm_metadata]]),
   NOT gestures.
4. **Game-over dialog isn't modal — background stays interactive.** Can click the (invisible)
   hamburger behind it; clicking it then "T2iles" starts a new game; sometimes clicking the board
   starts a new game (a stale/mis-placed `.offset` hitFrame from the modal buttons landing on the
   board/hamburger area). Fix: the modal must BLOCK background input (`.allowsHitTesting(false)` on
   the background when a modal is up — the mechanism exists, verify it covers the game-over modal),
   AND fix the `.offset` hitFrame misplacement (3a) so phantom frames stop landing on the board.

**Root theme:** one transform-aware-hit-testing fix (responder-tree `containsGlobalPoints` instead
of flat `hitFrame` for offset/modal subtrees) likely resolves 1, 3a, and 4 together. 2 is
responder-stability-during-animation; 3b is the separate conditional-view ABI issue.

---

## 1. ✅ DONE (2026-07-17) Split `CSwiftSpike` → a standalone leaf `CWASICanvas`  (unblocks everything) — §6, §7

New package `swift/OpenSwiftUIProject/CWASICanvas` (own `wit/cwasi-canvas.wit`, imports only
wasi:canvas/{types,draw,embedding,layout}). `WandrCG`, `T2iles`, `SwiftSpike`, `OpenSwiftUIDemo` now
depend on it instead of getting those bindings from the app's own `CSwiftSpike`. The app's
`wit/spike.wit` was shrunk to exports + audio/metrics only (no more wasi:canvas — avoids a
duplicate-symbol link error, since wit-bindgen names functions by interface, not by world, so both
generations would define identical C symbols if both were linked). The two worlds' generated
`component_type.o` files are linked together at the final link step and compose correctly — verified
both by successful `wasm-tools component new`/`validate` AND by actually running T2iles (rendered
frames, app-bundled fonts still loaded, full gameplay confirmed working).

One gotcha worth knowing for #2/#3: wit-bindgen's *generic helper* types (`string`/`list`/`tuple`/
`option` wrappers, e.g. what was `swift_spike_string_t`) are prefixed by **world name**, not
interface name — unlike the `wasi_canvas_*` interface-level symbols, which matched exactly across
both worlds. `CGContext.swift`/`CGImage.swift` needed their `swift_spike_string_t` etc. references
renamed to `cwasi_canvas_string_t` etc. to match CWASICanvas's own generated names.

---

`CSwiftSpike` is a per-app generated blob holding BOTH the wasi:canvas *drawing* bindings AND the
input/export trampolines. Because it lives inside the app package (which depends on OpenSwiftUI),
nothing above the app can import it (package cycle).

- Extract JUST the `wasi_canvas_*` (draw/types/layout/embedding) bindings into a standalone leaf
  C module/package **`CWASICanvas`** (depends on nothing).
- Leave the `exports_wasi_input_handlers_*` + `wandr:ui-shell/frame-pacing` export trampolines with
  the reactor/runtime (task 3).
- Verify: `CWASICanvas` builds standalone; the app still builds consuming it.

## 2. ✅ DONE (2026-07-17) Normalize OpenCoreGraphics — `CGContext` lives in OCG, retired vendored `WandrCG` — §2, §6, §7

Added `OpenCoreGraphicsWASICanvas` (new target in the OCG submodule) holding
`CGContext.swift`/`CGImageHandle.swift`/`CGColor.swift`/`CGGradient.swift`, depending on
`OpenCoreGraphics` (geometry) + `CWASICanvas`. Wired through `OpenCoreGraphicsShims`'
pre-existing `#if OPENCOREGRAPHICS_COREGRAPHICS / #elseif os(WASI) / #else` platform-select —
NOT a new public product, matching upstream's own `<X>Shims` convention (confirmed against the
upstream repo: 15/15 import sites use `OpenCoreGraphicsShims` directly, never a bare
`OpenCoreGraphics`). All 6 consumers in `swift-canvas-spike` (`T2iles`, `SwiftSpike`,
`OpenSwiftUIDemo`) now `import OpenCoreGraphicsShims`; the vendored
`repros/swift-canvas-spike/Sources/OpenCoreGraphics` (`WandrCG`) directory is deleted.

Gotcha worth knowing: the vendored image type was previously named `CGImage`, same as
`OpenCoreGraphics`'s own portable, `encodedData`-based `CGImage` (added separately for
`NamedImage.swift`'s off-Apple bitmap resolve). Declaring a second `CGImage` in
`OpenCoreGraphicsWASICanvas` shadowed the real one through the `@_exported import
OpenCoreGraphics` re-export chain and silently broke named-image loading. Renamed the
wasi:canvas-internal handle type to `CGImageHandle` — it's a `CGContext`-internal
implementation detail (`decodeImage`/`makeImage`/`drawImageFitting`/`draw`), never crosses the
`WandrDrawSink` boundary (which passes raw encoded bytes), so the rename had no other call sites
beyond `CGContext.swift` and the app's own `CGSink`/`DisplayList` glue.

Verified end-to-end on desktop (Linux + Windows, same wasm, checksum-matched): the game renders,
including the newly-working bitmap assets (`Icon`/`3x3`/`4x4`/`5x5`) and audio, no regressions.
(The "no images / maybe crashed" scare mid-session was a self-inflicted desktop-launch mistake —
a bare `.wasm` positional arg hits `AppRef::DevCwasm` mode, which skips the installed-app loader
and never preopens `/assets`; the fix is `--app <id>` with the correct `WANDR_APPS_ROOT`, not a
code change.)

Retiring the dormant `harryzz/OpenCoreGraphics@wasm32-wasip1` branch (a GitHub branch deletion)
was intentionally left undone — a destructive, hard-to-reverse action outside this task's scope
without explicit confirmation.

## 3. Move the runtime/plumbing OUT of the app — a shared `wandr-runtime` — §7, `Sources/T2iles/RULES.md`

- Create a shared **`wandr-runtime`** product (imports OpenSwiftUI + `CWASICanvas`) holding: the
  `@_cdecl` exports (`on_frame`/`on_pointer`/`on_resize`/`next_frame_delay`), the wasi:canvas
  embedding handshake, the `CGSink` (`WandrDrawSink` conformer), frame pacing, and a **`runWandrApp`
  runner** beside the framework's existing `runStdoutApp` (see `App/App/App.swift:153` dispatch +
  `App/Stdout/StdoutApp.swift`).
- The app then collapses to `dependencies: [<one wandr product>]`, source `import OpenSwiftUI`
  only — carrying just **Audio / Store / startup** per `RULES.md`. No more `import CSwiftSpike` /
  `import WandrCG` in `WandrReactor.swift`.

## 4. Normalize OpenSwiftUIProject structure (finish) — §7

- Apple-compat shims already extracted to `swift/apple-compat` ✅.
- Confirm the final layering: app = views + Audio/Store/startup; framework = OpenSwiftUI; runtime =
  `wandr-runtime` + `CWASICanvas`; geometry = OCG. Update `COMPONENTS-AND-BUILD.md` §3/§7 to match.
- Optional: relocate the spike session docs (`HANDOFF-*.md`, `AUDIT-*.md`, `CONDITIONAL_WASM_*`,
  `openswiftui_unimplemented.md`) into `swift/OpenSwiftUIProject/tests/` and retire
  `repros/swift-canvas-spike` once the app has a real home under `apps/user/wandr.swiftui.demo`.

## 5. THEN: frosted backdrop blur behind modals — §2 "POLISH TODO" / effects list

The modals' background blur (`.filter(.blur)`) is still dropped. Unlike shadow, the wasi:canvas
contract has **no general layer/backdrop-blur verb** (only per-paint `mask-blur`).

- Add a WIT verb: an optional blur on `save-layer`, or a `set-backdrop-blur` on the scene `layer`
  (`proposals/wasi-canvas/wit/{canvas,scene}.wit`); implement it in the host Skia sink
  (`SkImageFilters::Blur` / backdrop filter).
- Wire the renderer's `.filter(.blur(style))` case to it (parallel to the `.filter(.shadow)` case).
- This is a shared-WIT change → rebuild all consumers + restart zygote on device
  (`[[feedback_shared_wit_rebuild_all_consumers]]`).

---

## Polish items (independent, do anytime)

- **Shadow contrast** — the blur `radius → sigma` mapping is `sigma = radius` in
  `WandrDisplayListRenderer.wandrApplyProjection`/`CGContext.fillShadowPath`; tune (likely
  `sigma = radius * 0.5` or add spread) for more contrast vs the original.
- **3D-tilt fidelity** — the perspective reads but is "far from original"; compare against the
  reference and tune (anchor/perspective, and add the card's own shadow under the tilt).
