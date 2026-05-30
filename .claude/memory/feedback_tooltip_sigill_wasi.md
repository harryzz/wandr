---
name: tooltip-sigill-wasi
description: "Material3 TooltipBox-on-wasi traps with an uncatchable SIGILL ~5 s after the wrapped widget is tapped. Bisected end of task 28 via ChevronBisectCard A→E. Same Popup-overlay family as [[popup-overlay]]; likely the same root cause as the DropdownMenu \"expand animation freezes\" issue documented there."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 40cc86ad-d80a-4902-af53-a3eaeae80e40
---

**Symptom (bisected 2026-05-19, end of task 28):**
Tapping a Material3 widget whose IconButton is wrapped in
`TooltipBox(positionProvider=…, tooltip={ PlainTooltip{…} }, state=rememberTooltipState())`
triggers a SIGILL in JIT'd wasm ~5 s after the tap. Crash signature:
- `signal 4 (SIGILL), code 1 (ILL_ILLOPC)`
- PC = LR (function-entry trap), single anonymous-region frame
- Register state identical across runs (x21=0xAF5, x22=0x38, …)
- wasmtime's signal handler does NOT intercept it — process aborts to debuggerd
- No exception message extractable from wasmtime store (the trap bypasses
  the wasm exception path that `Engine::wasm_exceptions(true)` would catch)

**Bisect chain (`wart-app/.../ChevronBisectCard.kt`):**
| Layer | + Component | Crash? |
|------|------------|--------|
| A | plain TextButton + state | ✅ |
| B | + AnimatedContent | ✅ |
| C | + IconButton/ripple + AnimatedContent | ✅ |
| D | + inline ImageVector chevrons (IconButton + Icon) | ✅ |
| **E** | **+ TooltipBox(PlainTooltip, rememberTooltipState)** | **💥 SIGILL** |

So: state-update, AnimatedContent, IconButton/ripple, ImageVector
rasterization (the full task 28 bc-* path) are ALL innocent. The trigger
is specifically `TooltipBox`.

**Why this matters beyond DatePicker:**
- Material3 DatePicker's `IconButtonWithTooltip` wraps each chevron in
  exactly this shape — that's why the calendar `<` / `>` taps SIGILL.
- The companion `feedback_popup_overlay` memo describes how DropdownMenu /
  AlertDialog / Tooltip / ExposedDropdownMenu all share the wasi popup-
  overlay path, where state-driven graphicsLayer block re-invocation is
  broken in layer compositions. Tooltip's crash may have the same root
  (popup-owned `NodeCoordinator` recompose path) — fixing one likely
  fixes the other and resolves the long-standing DropdownMenu expand-
  animation issue too.

**What's definitively ruled out as the trap source:**
- Task 28's bitmap-canvas dispatch (`bc-*` verbs): host-side trace shows
  identical per-frame pattern before AND after the tap, host tables
  perfectly stable (`images=2, bitmap_canvases=4, …`), wasm linmem held
  at 32 MB. No leaks.
- Periodic `Store::gc` cadence: same crash with gc off (~4.3 s) and on
  (~4.8 s).
- IconButton ripple/clickable machinery, AnimatedContent transition
  continuations, ImageVector → DrawCache → bitmap-canvas snapshot
  per-frame churn.

**How to apply:**
- If a wasi app needs Material3 widgets that internally wrap children in
  `TooltipBox`, expect a deterministic SIGILL ~5 s after first tap. Known
  consumers in compose-multiplatform-core:
  `DatePicker.IconButtonWithTooltip` (chevrons + year-arrow),
  `SheetDefaults.DragHandleWithTooltip`.
- Avoid `TooltipBox` outright on wasi, OR locally patch consumers to drop
  the wrapper (verified workaround: bypass `IconButtonWithTooltip` →
  plain `IconButton` makes DatePicker stable). Do NOT ship the upstream
  patch — it masks the real bug.
- When the wasi Popup-overlay snapshot-observer issue from
  [[popup-overlay]] is fixed, retest Tooltip — same patch likely closes
  both.

**Why:** task 28 user push (2026-05-19) — chevron `<` / `>` taps crashed
DatePicker. Initial hypothesis was a Path D regression (vector-icon
rasterization). Bisect ruled out every component below TooltipBox over a
single afternoon. Crash address pattern + zero host-side anomalies +
deterministic ~5 s tap-to-trap = consistent with the popup-overlay
machinery accumulating something inside Compose's recomposition that
eventually trips a wasm-GC trap wasmtime can't surface.

**Companion paths to investigate next:**
- Reproduce with plain `TooltipBox` (no IconButton inside, no animation).
  Layer E used the full DatePicker shape; a simpler repro pins the
  trigger more precisely.
- Add wasm `unreachable` instruction-level logging via a custom SIGILL
  handler (current wasmtime version doesn't surface this kind of trap as
  an Err return).
- Check whether the `PlainTooltip` lambda receiver (`TooltipScope`) or
  the `rememberTooltipState()` state machine specifically — not the
  positioning / popup mechanics — is what trips it.

---

## Deeper bisect (2026-05-19, follow-up session)

Continued the bisect via `TooltipInspectionCard` in wart-app. Results
of 16 layered tests (after Layers A-E above):

| Test | Setup | Crash? |
|---|---|---|
| LocalInspectionMode provides true wrap | Layer E + provider | 💥 (workaround does NOT mask) |
| enableUserInput=false | TooltipBox with gestures disabled | ✅ no crash |
| plain `Popup(onDismissRequest){...}` on tap | no TooltipBox at all | ✅ |
| bare `while(true){ awaitPointerEvent() }` | 180 taps | ✅ |
| `awaitEachGesture{ awaitFirstDown(Initial); withTimeoutOrNull(500) }` | long-press detector shape only | ✅ |
| both `handleGestures` pointerInputs chained | bare Box | ✅ |
| `anchorSemantics`-shape only (`semantics(mergeDescendants=true){onLongClick…}`) | bare Box | ✅ |
| handleGestures + anchorSemantics combined | bare Box | ✅ |
| real TooltipBox + bare Box+Text (NO clickable) | full TooltipBox machinery, innocent inner | ✅ |
| real TooltipBox + `IconButton(onClick){ Text }` | no Icon, no AnimatedContent | 💥 |
| real TooltipBox + `Box.clickable{}` | bare Box, just clickable | 💥 |
| hand-built `handleGestures+anchorSemantics` outer + `Box.clickable{}` inner | no real TooltipBox | ✅ |
| TooltipBox + `Box.clickable(indication=null)` | no ripple | 💥 |
| TooltipBox + `Box.pointerInput { detectTapGestures(onTap) }` | same gesture detector clickable uses | ✅ |
| TooltipBox + `Box.semantics{role=Button; onClick…}` only | semantics piece of clickable only | ✅ |
| TooltipBox + `Box.focusable()` only | focusable piece of clickable only | ✅ |

**Net localisation:**
- The trigger is **`ClickableElement` (the modern Modifier.Node) +
  real `BasicTooltipBox` wrapper** simultaneously.
- Neither alone triggers it: TooltipBox + bare content works; bare
  modifier chain + clickable works.
- None of clickable's individual pieces in isolation (gesture detector,
  semantics, focusable, ripple) triggers it when wrapped in TooltipBox.
- `LocalInspectionMode provides true` does NOT mask the crash — so it's
  NOT the same root as the DropdownMenu "expand animation freezes" bug
  in `feedback_popup_overlay` after all. They're related (both popup
  family) but distinct mechanisms.

**Likely culprit (untested but matches the data):**
BasicTooltipBox's wrapper machinery that's absent from the bare
modifier-chain test: `rememberCoroutineScope()` outside, `DisposableEffect(state)`,
and the always-recomposed `if (state.isVisible) { TooltipPopup }` slot.
One of these interacts with `ClickableElement`'s attach/update
lifecycle. ClickableNode's `update()` in particular reacts to changes
in its parameters and may dispose/recreate internal state in a way
that the surrounding BasicTooltipBox recomposition tickles.

**Concrete next steps for whoever picks this up:**
1. Instrument `ClickableElement.update(node)` and `ClickableNode.onAttach/onDetach`
   with `logMessage()` calls to see if they fire repeatedly per tap when
   wrapped in TooltipBox vs alone.
2. Replicate BasicTooltipBox's outer structure (rememberCoroutineScope
   + DisposableEffect(state) + conditional Popup slot) by hand around a
   `Modifier.clickable` to confirm the hand-built version still crashes
   — if it does, narrows further; if not, real BasicTooltipState
   instance is required.
3. The `feedback_popup_overlay` instrumentation plan (early-exit
   guards in `LayoutModifierNode.updateLayerBlock` / `NodeCoordinator.
   updateLayerBlock`) — still worth running for the DropdownMenu
   animation bug, separately from this one.

---

## Step 1 sub-bisect (2026-05-19, task 29 step 1)

Added test #17 to `TooltipInspectionCard.kt`: hand-replicated
BasicTooltipBox's outer wrapper structure around `Modifier.clickable`.
Wrapper components present in #17:

- outer `val scope = rememberCoroutineScope()`
- `Box { if (sentinel.value) Popup(onDismissRequest = {}) { Text("dead") }; content() }`
  — `sentinel` is `remember { mutableStateOf(false) }`, never flipped;
  the `sentinel.value` snapshot read happens on every recompose
- trailing `DisposableEffect(sentinel) { onDispose { } }`
- inner content: `Box(Modifier.clickable { taps++ })`

**Result: ✅ no crash. 61 taps over ~45 s soak, process alive, logcat
clean.** Decisive — Step 1's decision tree fires the "hand-built
wrapper does NOT reproduce" branch.

| Test | Setup | Crash? |
|---|---|---|
| 17 | HandBuiltTooltipWrapperWithScope(popup=t, dispEff=t) + Box.clickable | ✅ |
| 18 | #17 minus DisposableEffect | (moot — base ✅) |
| 19 | #17 minus conditional Popup slot | (moot — base ✅) |
| 20 | #17 minus rememberCoroutineScope | (moot — base ✅) |
| 21 | bare clickable + DisposableEffect only | (moot — base ✅) |
| 22 | bare clickable + conditional Popup only | (moot — base ✅) |

**Step 1 outcome:**
The wrapper structure ALONE (scope + always-recomposed conditional
Popup-on-State<Boolean> read + trailing DisposableEffect) is
INSUFFICIENT to reproduce the SIGILL. Real `BasicTooltipState`
machinery is part of the trigger — its `isVisible: MutableState` +
`MutatorMutex` + `suspendCancellableCoroutine` interaction with
`ClickableNode` lifecycle is what closes the loop. Tests #18-#22
deleted as moot.

**Implications for Step 2:**
Compare the call sequence on ClickableNode between the crashing path
(real TooltipBox + clickable, test #11) and the non-crashing path
(hand-built wrapper + clickable, test #17). The difference is solely
the state-machine inside `BasicTooltipState`, so whatever fires
uniquely in #11 is most likely an update() or rebind triggered by
something BasicTooltipState reads/writes during a long-press timeout
that clickable's gesture detector races with.

**Approach order (per [[prefer-wart-app-edits]]):** try wart-app-side
diagnostics first — a custom logging Modifier.Node wrapped around the
clickable, a hand-rolled clickable equivalent with built-in logging,
or `SideEffect`/`DisposableEffect`-based per-recompose counters. Only
if those prove insufficient ask the user before instrumenting
`ClickableElement.update/onAttach/onDetach/onPointerEvent` in
`compose-multiplatform-core/compose/foundation/foundation/src/
commonMain/kotlin/androidx/compose/foundation/Clickable.kt` (republish
compose-foundation-wasi to pick up edits). See [[compose-wasi-srcdirs]]
for the bundler vs. source distinction.

---

## Step 2 sub-bisect (2026-05-19, task 29 step 2)

Six new tests added to `TooltipInspectionCard.kt`, all wart-app-side
(no compose-multiplatform-core edits):

| Test | Setup | Crash | Note |
|---|---|---|---|
| 23 | TooltipBox + clickable + composition probe (DisposableEffect + SideEffect inside inner Box) | 💥 ~immediate on first Press | Only initial attach + recompose-#1; composition is stable |
| 24 | hand-built wrapper + clickable + same composition probe | ✅ (30 taps) | Identical attach + recompose-#1 signal as #23 → composition lifecycle is uninformative |
| 25 | TooltipBox + hand-rolled (pointerInput + semantics + focusable) | ✅ (11 manual taps) | Bug is NOT the feature combination |
| 26 | TooltipBox + clickable + custom Modifier.Node lifecycle probe (lifecycleProbe pre/post) | 💥 on first Press | Probes attach once; no Modifier.Node detach/reattach/update before crash |
| 27 | TooltipBox + clickable + passive `pointerInput(awaitPointerEvent at Initial pass)` | 💥 within 10 ms of first Press event | Crash window pinpointed to Press dispatch |
| 28 | TooltipBox + `clickable(enabled = false)` | ✅ on short tap, 💥 on long tap | Disabled clickable + short tap survives; long tap still triggers — proves trigger is NOT the active Press handler exclusively |

**The common path between every crashing variant:** all the crashes
reach **`BasicTooltipState.show()`** in `Tooltip.kt:1055-1078`. There
are two ways it gets called from a `TooltipBox`-wrapped tree:

1. **Enabled clickable + short tap path** (test #11, #23, #26, #27):
   `clickable(enabled = true)` calls `requestFocus()` on Press in
   touch mode. Focus arrives at the anchor Box → `keyboardBehavior`'s
   `onFocusChanged { if (isFocused) { state.show(...) } }` (in
   `BasicTooltip.kt:315-321`) launches `state.show()` from the scope.

2. **Long-press path** (test #28 long tap, and original test #11
   pre-step-2 measurements showed ~5 s tap-to-crash matching this
   timer): `handleGestures` `withTimeout(longPressTimeout) {…}`
   throws `PointerEventTimeoutCancellationException`, the catch
   block calls `state.show(MutatePriority.PreventUserInput)`
   directly (`BasicTooltip.kt:239`).

The disabled-clickable + short-tap case survives because:
- `requestFocus()` isn't called → no focus path
- Short UP cancels handleGestures' long-press timer → no long-press path
- So `state.show()` is never reached.

**Net Step 2 outcome:**
The crash trigger is `BasicTooltipState.show()` —
specifically the `suspendCancellableCoroutine` inside
`mutatorMutex.mutate { … withTimeout { cancellableShow() } }` at
`Tooltip.kt:1057`. This connects directly to the
[[kotlin-wasm-suspendcoroutine-leak]] family: `suspendCancellableCoroutine`
is the same suspension primitive whose Kotlin/Wasm codegen + wasmtime
DRC interaction has been the root of every prior wasm-GC anomaly on
wasi. The SIGILL is the harder failure mode of that family — a
deterministic trap on the wasm-GC op at the suspension point — rather
than the slow live-set growth from the indeterminate-progress / leak-
repro case.

**For Step 3/4:**
- (Step 3 — diagnostic) Decode the cwasm at the SIGILL fault PC
  (offsets observed: `0x7c4d5aa1ec`, `0x7c51d7ed2c`, `0x7c523b542c`).
  Likely a `udf` emitted by cranelift for a structref op (allocation
  or null-check) whose PC isn't registered with wasmtime's trap table.
- (Step 4 — workaround) Block reaching `state.show()` in the wasi
  build. Cheapest path: a wasi-flavor override of `BasicTooltipBox`
  that hard-stubs `keyboardBehavior.onFocusChanged → state.show()`
  AND `handleGestures` long-press timeout (or stubs `state.show()`
  itself). Lands either in `compose-material3-wasi/src/wasmWasiActuals`
  (override) or as a local patch in `compose-multiplatform-core/.../
  BasicTooltip.kt`. Strongly prefer the override path per
  [[prefer-wart-app-edits]] — keeps upstream clean.

---

## Step 3 cwasm decode (2026-05-19, task 29 step 3)

Pulled tombstones from `/data/tombstones/` and decoded fault PCs
against `wasmtime objdump` of the deployed cwasm:

| Tombstone | Test variant | Fault PC offset | Region base | Note |
|---|---|---|---|---|
| 39 | #26 (Modifier.Node probe) | `0x2669d2c` | `0x7c4f715000` | |
| 40 | #27 (pointer observer) | `0x266b42c` | `0x7c4fd4a000` | |
| 41-43 | #28 long-press (×3) | `0x266804c` (stable) | varies per launch | same cwasm, same op |

All three fault offsets land inside one wasi-adapter function:

```
02667f80 wasm[1]::function[32]::wasi_snapshot_preview1::macros::assert_fail:
   2667f80: stp x29, x30, [sp, #-0x10]!
   ...
   2668038: bl #0x2667e80   ; macros::print (writes assert msg to stderr)
   ...
   2668048: bl #0x266b240   ; macros::eprint_u32 (writes line number)
   266804c: .byte 0x1f, 0xc1, 0x00, 0x00
            ╰─╼ trap: UnreachableCodeReached   ← the SIGILL
```

The trap is the **`unreachable` instruction at the end of
`wasi_snapshot_preview1::macros::assert_fail`** — Rust's `panic!`/
`assert!` lowering inside the WASI P1→P2 reactor adapter
(`~/wart/skiko/wasi_snapshot_preview1.reactor.wasm`).

**Important:** wasmtime knows about this trap site — `wasmtime
objdump` correctly labels it as `trap: UnreachableCodeReached`. So
the original Step 3 hypothesis ("cranelift emits a UDF whose PC isn't
registered in wasmtime's trap table") is FALSE — the trap is
registered. wasmtime's signal handler simply **isn't catching it at
runtime** on this Android build, even though tid is `android_main`
where the handler should be installed. Likely cause: signal-handler
shadowing between wart-host's winit/Android NDK setup and wasmtime's
sigaction registration. Needs deeper poking at `sigaction` on the
process.

**Caller of `assert_fail` is `poll_oneoff`** (most likely): of the
16 `bl 0x2667f80` sites in the adapter, 6 are inside
`wasi_snapshot_preview1::poll_oneoff` (function[52] at 0x266a2c0) —
more than any other caller. The next most-frequent is
`cabi_import_realloc` (4 sites), then `random_get` (3), `fd_write`
(2), `poll_oneoff` (6), `State::new` (1).

Why `poll_oneoff`: the crashing path runs
`state.show()` → `mutatorMutex.mutate { withTimeout {
suspendCancellableCoroutine } }` → kotlinx-coroutines `Delay` on
wasmWasi. The Delay implementation on wasmWasi schedules sleep via
the WASI P1 adapter's `poll_oneoff` (sleep on a clock subscription).
An assertion inside `poll_oneoff` matches: timer setup validation
(subscription tag, fd, duration overflow, etc.) likely trips when
called from inside `MutatorMutex.mutate`'s coroutine scope.

**Conclusion:**
The SIGILL is not a wasm-GC bug; it is a real Rust `assert!` /
`panic!` inside the WASI P1 reactor adapter's `poll_oneoff` (most
likely), triggered by the way the Compose Tooltip uses
`withTimeout`/`Delay` in `state.show()`. Wasmtime's signal handler
fails to convert this `unreachable` trap into a returnable Err on
Android — process aborts to debuggerd. Two-front fix: (a) Step 4
guest-side workaround that avoids `state.show()` on wasi; (b) host-
side investigation into wasmtime's signal-handler installation on
Android (probably a separate task once #29 is closed).

---

## Step 4 decision: NOT implemented (2026-05-19)

The user decided to leave the workaround unimplemented for now. The
bug is fully characterized; the workaround paths considered:

1. **`compose-material3-wasi/src/wasmWasiActuals/` override** — rejected,
   `compose-*-wasi` is out of scope (will be deleted; see
   [[compose-wasi-out-of-scope]]).
2. **`compose-multiplatform-core/.../wasmWasiMain/` override with
   `kotlin.exclude("**/internal/BasicTooltip.kt")`** — TRIED, build
   failed. `kotlin.exclude` only filters the source set's own
   srcDirs, not files inherited from commonMain via `dependsOn`. Both
   `BasicTooltip.kt` and `BasicTooltip.wasi.kt` compiled together →
   "Conflicting overloads" + "Redeclaration" on every public symbol.
3. **Edit commonMain `BasicTooltip.kt` directly** — single-file
   change but breaks tooltips on every target (jvm/native/js + wasi).
   Even though wart only ships wasi, diverging the fork that broadly
   was rejected by the user.
4. **Convert the 3 modifier extensions to `expect`/`actual`** — clean
   KMP but needs 5-7 per-target actuals (jvmAndAndroid, desktop,
   darwin, native, js, wasmJs, plus the wasi no-op). Considered too
   heavy for the symptom.

**Current mitigation** (already in place from task 28 closeout):
wart-app smoke-card menu omits widgets that internally wrap their
anchor in `TooltipBox` — DatePicker chevrons and friends are off the
UI. Plain DatePicker (calendar grid, swipe, year-pick) + non-Tooltip
Material3 widgets continue to work fine.

**Pre-conditions to revisit Step 4:**
- wasmtime's Android signal-handler bug gets diagnosed/fixed
  (turning the trap into a recoverable `Err`).
- OR a wasi-clean `Delay` implementation that bypasses the
  `poll_oneoff` assertion lands in kotlinx-coroutines or in our
  wart-host's WASI adapter glue.
- OR the cost-benefit shifts (e.g. someone needs a working Tooltip
  on wasi for a shippable feature, justifying the per-target actual
  set or a commonMain patch).
