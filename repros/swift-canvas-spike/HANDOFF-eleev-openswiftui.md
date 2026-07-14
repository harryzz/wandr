# eleev 2048 on OpenSwiftUI (off-Apple / wasm) — session handoff

**Goal:** run the real eleev/swiftui-2048 app (`Sources/T2iles`, UNMODIFIED) on the OpenSwiftUI fork
off-Apple (wasm32-wasip1 → wandr-host, Linux/WSLg + Windows), rendering + interacting correctly.
The app imports `SwiftUI`, which is a thin shim (`Sources/SwiftUI/`, `@_exported import OpenSwiftUI`).

Related memories: `reference_openswiftui_gestures_offapple`, `reference_openswiftui_sfsymbols_rendering`,
`reference_swift_openswiftui_wandr`, `reference_openswiftui_conditional_wasm_metadata`.

---

## Build / run / deploy (WSLg)

- **Guest:** `cd repros/swift-canvas-spike && bash build-t2iles.sh` (→ `--product T2iles` → component →
  deploys `ui.wasm` to `/home/harry/wandr-desktop-apps/apps/wandr.swiftui.demo/0.1.0/components/` and to
  the Windows copy). ~40–90 s. "Corrupted JSON" lines are non-fatal.
- **Host:** `bash tools/scripts/build-host-linux.sh` (~2.5 min). Binary:
  `runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host`.
- **Run (MUST use x11 on WSLg; Wayland/resize crashes the host):**
  ```
  WINIT_UNIX_BACKEND=x11 WANDR_APPS_ROOT=/home/harry/wandr-desktop-apps WANDR_DESKTOP_SIZE=460x920 \
    RUST_LOG=info <host binary> --app wandr.swiftui.demo
  ```
  x11 startup sometimes fails with "Broken pipe" — retry the launch 2–4×. **Never resize the window**
  (`[geom] Resized` → `Connection reset by peer` → host panic at lib.rs:1863 — a WSLg bug, not the app).
- **Assets:** the deploy overwrites only `components/ui.wasm`; `.../0.1.0/assets/` persists across guest
  rebuilds. `Strings.plist` lives there (see below).
- **Gotcha:** SwiftPM did NOT compile a BRAND-NEW `.swift` file added to the OpenSwiftUICore target —
  additions must go into an EXISTING file in that target.

---

## DONE — verified working (Linux + Windows)

1. **SF Symbol icons** (`Image(systemName:)`) render via the `OpenSFSymbols` package + bundled Tabler
   font. Wired at `OpenSwiftUICore` `Image.Resolved._makeView` → `wandrMakeSymbolView`
   (`WandrSymbolGlyph.swift`); resolves name→(font,codepoint), draws as a family-tagged text run the host
   shapes by name. Every OpenSwiftUI app gets it, zero app code.
2. **`Button` was an empty stub** (`body: EmptyView()`, discarded its label) → implemented to render the
   label + `.onTapGesture(action)` (`OpenSwiftUI/.../Button/Button.swift`). Root cause the icons never
   showed.
3. **Resizable symbols** fill their frame: `StyledTextContentView.wasmSymbolFill` (fill proposed size,
   square em box) + glyph sized to the laid-out rect at draw time (`WandrDisplayListRenderer` `.text`).
4. **Gesture routing / arbitration** (no full gesture graph) in `EventBindingManager.bindResponders`:
   bind the SMALLEST-area gesture whose `hitFrame` contains the point + the smallest TAP
   (`producesVoidValue`, co-delivery) so a click self-selects even over a tighter drag. Tap-first bucket
   ordering. GRANULAR reset (`sendDownstream` drops only the terminated responder's binding;
   `WandrRendererHost.didUpdate` is a no-op) — a blanket reset let a co-delivered tap failing mid-swipe
   kill the active drag → swipe freeze.
5. **`animPending=true` after every pointer event** (`WandrReactor.swift onPointer`) — else
   `withAnimation` state changes never advance the clock (menu never slides open).
6. **THE crash fixed — `ViewTransform.forEach(inverted:)` non-inverted buffer OFF-BY-ONE**
   (`Layout/View/ViewTransform.swift`): loop restarted `index=0`, overwrote head, left the last buffer
   slot uninitialized → `reversed()` retained garbage → "uninitialized element"/OOB the FIRST time a
   MULTI-element transform (open menu = offset+affine) was hit-tested → poisoned the whole guest ("cannot
   enter component instance" on every later frame). This was misdiagnosed for many iterations as
   co-delivery / producesVoidValue / animPending. Lesson: the host swallowed the on_pointer trap
   (`let _ = dispatch_pointer_routed`); once it LOGGED it, the real backtrace appeared instantly.
7. **`Bundle.main` crash fixed** (`NamedImage.swift`): `Image("Icon")` → `.bundle(bundle ?? Bundle.main)`;
   `Bundle.main` is a Foundation lazy global that traps on wasm. `Image.Location.bundle` is now `Bundle?`;
   inits use `Image._resolvedMainBundle` (nil off-Apple).
8. **Affine INVERSE in `ViewTransform.convert(point:)`** — the shim's `CGAffineTransform.inverted()` is
   real; the `#else` wrongly left it unimplemented.
9. **Tile numbers** render on one line: single-word text (no space) gets effectively unbounded paragraph
   maxWidth in `WandrDisplayListRenderer` `.text` (Skia was breaking "16"→"1"/"6").
10. **`.allowsHitTesting(_:)` implemented for real** (was a shim no-op): env key + `View.allowsHitTesting`
    + `_ViewInputs.allowsHitTesting` — APPENDED to `Data/EnvironmentKeys/Enabled.swift` (new file wasn't
    compiled). `GestureFilter` reads it → `GestureResponder.hitTestable`; `bindResponders` skips
    `!hitTestable`. Removed the shim stub. VERIFIED: swiping over an open menu no longer moves the board.
11. **`.buttonStyle(_:)` implemented for real** (was a shim no-op) in `Button.swift`: env-stored
    type-erased applier; `Button` runs `style.makeBody(configuration:)` and binds the config's `label`
    ViewAlias via `.viewAlias(ButtonStyleConfiguration.Label.self){ label }`. VERIFIED: modal buttons draw
    their filled backgrounds + text.
12. **`Strings.plist` reconstructed + deployed** to `/assets/Strings.plist` (was missing entirely). eleev
    reads it via `WandrPlist.swift` (POSIX `/assets/<name>.plist`, since Bundle.main is dead). Contains
    top keys `gameState` / `about` / `settings`. VERIFIED: game-over & reset modal titles/subtitles now
    render. NOTE: only in the DEPLOYED assets dir — should also be added to the app source assets for a
    fresh install.
13. **Host now LOGS on_pointer traps** (`runtime/wandr-host/src/input.rs`) instead of swallowing —
    essential; leave it in.

**Currently WORKS:** icons; open menu / reset; swipe moves+merges tiles (2 & 3 digit on one line);
tap-anywhere closes overlays; `.allowsHitTesting` gates the background; new game; modal text; modal
buttons VISIBLE; no crash in the game path.

---

## REMAINING PROBLEMS (with root causes)

1. **Modal buttons (Cancel/OK/New Game) are VISIBLE but NOT tappable.** They sit inside
   `BottomSlidableModalModifier`'s `.offset(y:)`. The gesture `hitFrame` is the LAYOUT (un-offset) frame,
   so the visible button ignores taps and its stale frame lands elsewhere (was triggering New Game from
   the hamburger area). **FAILED FIX (reverted):** setting
   `hitFrame = CGRect(transform.convert(.localToSpace(.global), point: position), size)` using
   `inputs.transform` — it DESTABILIZED the board (its transform is not identity) and did NOT place the
   modal buttons correctly. Conclusion: `inputs.transform` is NOT the offset-inclusive transform;
   `.offset`/`_OffsetEffect` (a GeometryEffect) likely feeds the DisplayList/render transform, not the
   responder-input transform. NEXT: READ end-to-end how `.offset`→GeometryEffect→(render vs event)
   transform flows and how leaf `ViewResponder.containsGlobalPoints` already accounts for transforms
   (the responder tree does proper geometric hit-testing) BEFORE editing. Possibly stop using the flat
   `hitFrame` and route through the responder tree's transform-aware containment for offset subtrees.

2. **Menu → Settings / About shows nothing (just closes the menu).** The tap FIRES (menu closes via
   onMenuChangeHandler, which is in the same Button action as `selectedView = ...`), and the plist works
   (modals prove it), yet `FactoryContentView`'s `@ViewBuilder switch selectedView` doesn't visibly swap
   to the new case. → a CONDITIONAL/switch view-list UPDATE issue on wasm (state changes but the shown
   subtree isn't rebuilt). Not gestures, not the plist. See
   `reference_openswiftui_conditional_wasm_metadata` (2 metadata-ABI bugs were fixed there; a switch with
   3 cases updating on `@Binding` change may have a residual issue).

3. **Board-area swipe intermittently "freezes"** (a header-area swipe on the same greedy board-drag NEVER
   freezes, and recovers). Suspect the board subtree's responders rebuild each tile-animation frame,
   dropping an in-flight gesture. Granular reset (#4 above) mitigated but didn't eliminate it.

4. **board-drag hitFrame is greedy into the header** (`(0,0,460,734)`) — swiping the upper area moves the
   board (commit c7a0a71b "bind swipe to the board square" didn't fully constrain it).

5. **`render(style:)` unimplemented** (`ShapeStyleRendering.swift:203`, fires ~4200×/run) for non-color
   fills (gradients/Materials) → white/blank backgrounds. Modal bg is a plain color so it renders; but
   any Material/gradient draws nothing.

---

## Key files touched (all under `swift/OpenSwiftUIProject/` unless noted)
- `OpenSwiftUICore/View/Image/{WandrSymbolGlyph.swift(new), Image.swift, ResolvedImage.swift, NamedImage.swift}`
- `OpenSwiftUICore/Layout/View/ViewTransform.swift` (buffer fix + affine inverse)
- `OpenSwiftUICore/Event/Gesture/GestureViewModifier.swift` (GestureFilter: hitFrame/hitTestable/producesVoidValue)
- `OpenSwiftUICore/Event/Event/EventBindingManager.swift` (bindResponders, granular reset)
- `OpenSwiftUICore/Render/DisplayList/{WandrDisplayListRenderer.swift, WandrRendererHost.swift}`
- `OpenSwiftUICore/View/Text/Text/Text+View.swift` (StyledTextContentView.wasmSymbolFill)
- `OpenSwiftUICore/Data/EnvironmentKeys/Enabled.swift` (allowsHitTesting appended)
- `OpenSwiftUI/View/Control/Button/Button.swift` (label + buttonStyle)
- `OpenSFSymbols/` (whole package) + `OpenSwiftUI/Package.swift` (dep)
- repro: `Sources/SwiftUI/{PreviewAndModifiers.swift, Containers.swift}` (removed allowsHitTesting/buttonStyle stubs);
  `Sources/T2iles/WandrReactor.swift` (animPending); `WandrPlist.swift`
- host: `runtime/wandr-host/src/input.rs` (on_pointer trap logging)
- deployed: `/home/harry/wandr-desktop-apps/apps/wandr.swiftui.demo/0.1.0/assets/Strings.plist`

## STATE OF THE TREE right now
Source is reverted to the last good state (ButtonStyle + plist; board stable; modal buttons
visible-not-tappable). The DEPLOYED `ui.wasm` is still the FAILED transform build (worse board freeze) —
**rebuild `build-t2iles.sh` once to redeploy the stable guest** (no need to re-test it).
