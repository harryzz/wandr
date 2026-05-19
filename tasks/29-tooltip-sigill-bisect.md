# Task 29 — Diagnose the Material3 TooltipBox SIGILL on wasi

> **Status: 🔲 scoped 2026-05-19, bisect 80% done — pick up where the
> `TooltipInspectionCard` left off.**

## TL;DR

Tapping any Material3 widget whose anchor is wrapped in
`TooltipBox(...)` on wasi triggers a `SIGILL (ILL_ILLOPC)` in JIT'd
wasm ~5 s after the first tap. **Wasmtime's signal handler does NOT
intercept this** — the process aborts straight to debuggerd without any
Kotlin exception message recoverable from the wasmtime store, so no
`render_frame fatal` log appears. A 16-layer bisect (committed as
`wart-app/src/wasmWasiMain/kotlin/TooltipInspectionCard.kt`) narrowed
the trigger to **the modern `Modifier.clickable` (ClickableElement
Modifier.Node) interacting with `BasicTooltipBox`'s wrapper
machinery**. Neither alone crashes; the combination does. This task
picks up the bisect and fixes (or upstream-patches around) the root
cause.

User-visible impact: Material3 DatePicker's chevron `<` / `>` buttons
crash, as do any other Material3 widgets that internally wrap their
anchor in `TooltipBox` (e.g. `IconButtonWithTooltip` in DatePicker,
`DragHandleWithTooltip` in SheetDefaults). DatePicker's calendar
grid + swipe + year-pick all work; only the Tooltip-wrapped chevrons
crash.

## Companions

- `feedback_tooltip_sigill_wasi.md` — full bisect table + observations
  (this task's source of truth)
- `feedback_popup_overlay.md` — related but **distinct** wasi popup
  bug (DropdownMenu / AlertDialog expand-animation freeze). The
  Tooltip SIGILL is in the same popup family but is NOT masked by the
  DropdownMenu workaround (`LocalInspectionMode provides true`) — see
  test #1 in the bisect.
- `feedback_transition_animate_to_bug.md` — resolved 2026-05-13;
  rules out the identityHashCode path as a candidate root.
- `tasks/28-skiko-abstract-canvas.md` — the bisect was triggered by
  task 28's DatePicker re-enablement; bc-* dispatch is verified
  innocent (host tables stable, no leaks).
- `wart-app/src/wasmWasiMain/kotlin/TooltipInspectionCard.kt` — the
  16-layer bisect harness. Currently set to a non-crashing layer
  (test #16) so the demo boots clean; swap the body to test #11 to
  reproduce the crash deterministically.

## What's already known

### Crash signature

- `Fatal signal 4 (SIGILL), code 1 (ILL_ILLOPC), fault addr 0x7c…a9ec`
- PC = LR (function-entry trap), single anonymous-region frame in
  the tombstone — JIT'd wasm code
- Identical register state across runs (`x21=0xAF5`, `x22=0x38`,
  `x4=1`, `x5=2`, `x6=1`, `x7=4`, `x12=0x1001`, …) → a deterministic
  code path, not memory corruption
- Wasmtime does NOT return Err from `render_frame.call(...)` — the
  always-on Kotlin exception extractor never fires. So the trap is
  one wasmtime's signal handler refuses (or fails) to intercept.
  Candidate explanations: wasm-GC heap allocation failure in a
  trampoline wasmtime doesn't track; cranelift codegen for a
  specific GC op emits a UDF instruction whose PC isn't in
  wasmtime's registered code range; or a host-call boundary
  produces it.
- ~5 s tap-to-crash (was 4.3–4.8 s in measured runs). Same regardless
  of periodic `Store::gc` cadence — confirmed by toggling gc off in
  one bisect cycle.

### Bisect table (16 layered tests in `TooltipInspectionCard.kt`)

| # | Setup | Crash |
|---|---|---|
| A | plain TextButton + state | ✅ |
| B | + AnimatedContent | ✅ |
| C | + IconButton/ripple + AnimatedContent | ✅ |
| D | + inline ImageVector chevrons (IconButton + Icon) | ✅ |
| E | + `TooltipBox(PlainTooltip, rememberTooltipState)` | 💥 |
| 1 | E + `CompositionLocalProvider(LocalInspectionMode provides true)` | 💥 |
| 2 | `enableUserInput = false` on TooltipBox | ✅ |
| 3 | plain `Popup(onDismissRequest){…}` on tap, no TooltipBox | ✅ (180 taps) |
| 4 | bare `while(true){ awaitPointerEvent() }` on a Box | ✅ |
| 5 | `awaitEachGesture { awaitFirstDown(Initial); withTimeoutOrNull(500){…} }` | ✅ |
| 6 | both `handleGestures` pointerInputs chained, bare Box | ✅ |
| 7 | `anchorSemantics` only (`semantics(mergeDescendants=true){onLongClick…}`) | ✅ |
| 8 | handleGestures + anchorSemantics combined on bare Box | ✅ |
| 9 | real TooltipBox + bare `Box.pointerInput { awaitFirstDown }` + Text | ✅ |
| 10 | real TooltipBox + `IconButton(onClick){ Text }` | 💥 |
| 11 | real TooltipBox + `Box.clickable { taps++ }` | 💥 |
| 12 | hand-built handleGestures+anchorSemantics outer + Box.clickable inner | ✅ |
| 13 | TooltipBox + `Box.clickable(indication = null)` | 💥 |
| 14 | TooltipBox + `Box.pointerInput { detectTapGestures(onTap) }` | ✅ |
| 15 | TooltipBox + `Box.semantics { role = Role.Button; onClick(…) }` | ✅ |
| 16 | TooltipBox + `Box.focusable()` | ✅ ← current |

### Ruled out

- Task 28's bitmap-canvas dispatch (host tables stable, no leaks
  during the entire crash window — instrumented bc_trace + dimensions
  + counts all showed steady state).
- Periodic `Store::gc` cadence (toggled off; same crash timing).
- `LocalInspectionMode provides true` does NOT mask it → distinct
  from the popup-overlay layer-block-not-re-invoked bug.
- IconButton ripple / clickable's gesture detector
  (`detectTapGestures`) alone.
- AnimatedContent / Material3 ripple / ImageVector rasterization /
  any individual modifier piece of clickable.
- The popup substrate itself — `androidx.compose.ui.window.Popup`
  triggered on tap survived 180 taps.

### Localized to

**`ClickableElement` Modifier.Node + `BasicTooltipBox` wrapper
machinery, simultaneously.** Neither alone crashes. The bare modifier
chain replicating `handleGestures + anchorSemantics` works fine even
with `Modifier.clickable` inside. The real `BasicTooltipBox` wrapper
that's absent from the bare test:

```kotlin
@Composable
fun BasicTooltipBox(..., content: @Composable () -> Unit) {
    val scope = rememberCoroutineScope()
    WrappedAnchor(...) {
        if (state.isVisible) { TooltipPopup(...) }   // ← always-recomposed slot
        content()                                     // ← user's content
    }
    DisposableEffect(state) { onDispose { state.onDispose() } }
}
```

Likely a `ClickableNode.update()` or `onAttach/onDetach` interaction
with the outer recomposition driven by `state.isVisible` snapshot
reads (even when state never flips visible).

## Steps

### Step 1 — Re-localize on the BasicTooltipBox wrapper

Reproduce BasicTooltipBox's outer structure by hand around a
`Modifier.clickable`-bearing Box: explicit `rememberCoroutineScope()`,
explicit `DisposableEffect(SomeState) { onDispose { … } }`, and an
always-recomposed `if (someVisibleState.value) { Popup(…) }` slot.
Use a `mutableStateOf(false)` that's never flipped, so the Popup
branch is dead.

**Decision point:**
- If THIS reproduces the SIGILL → the wrapper structure itself is
  enough; no real `BasicTooltipState` instance needed. The fix
  candidate is one of: the always-recomposed conditional slot, the
  DisposableEffect, the scope-capture pattern. Go to Step 2.
- If NOT → real `BasicTooltipState` (its `isVisible: MutableState`
  + `MutatorMutex` + `suspendCancellableCoroutine` machinery) is part
  of the trigger. Add the real state and bisect what part of it.

### Step 2 — Inside-clickable instrumentation

Add `WitCanvas.Import.logMessage(...)` calls inside
`ClickableElement.update(node)`, `ClickableNode.onAttach()`,
`ClickableNode.onDetach()`, and `ClickableNode.onPointerEvent(...)`
in `compose-multiplatform-core/.../Clickable.kt`. Rebuild
compose-foundation-wasi. Re-run the minimal repro (test #11) and
compare to the non-crashing case (test #9) to see what call sequence
is unique to the crash path.

Expect to see: ClickableNode getting `update()`'d on every
recomposition (because BasicTooltipBox always recomposes when
state.isVisible is even tracked as a snapshot read). Each `update()`
may rebind delegated nodes (pointerInput, focusable, semantics, …).
If the rebind path leaks a Job or a continuation, that accumulates.

### Step 3 — wasmtime trap surface

Independent of the Compose investigation: figure out why wasmtime's
signal handler doesn't catch this SIGILL. Two possibilities:
1. Cranelift emits a `udf` for some GC-related runtime check whose
   PC isn't registered with wasmtime's trap table. Confirm by
   reading the AOT cwasm at the fault offset (the JIT base randomises
   per-launch but the offset within the region is stable; capture it
   from one crash, then disassemble the cwasm at that offset).
2. The trap fires on a non-main thread (binder thread, sensor poll
   thread) where wasmtime's signal handler isn't installed.
   Inspect the SIGILL `tid` in tombstone — confirmed to be
   `android_main` in all observed crashes, so this is unlikely.

If (1) is confirmed, file a wasmtime upstream bug AND consider
installing a project-side SIGILL handler that logs PC + a few
registers so the next crash gives us the exact wasm instruction.

### Step 4 — Workaround until upstream fix

Patch `compose-multiplatform-core` consumers of `TooltipBox` to
bypass it on wasi. Already validated mid-bisect that replacing
`IconButtonWithTooltip` with a plain `IconButton` in
`material3/DatePicker.kt` makes the calendar fully usable. Document
the patch list (DatePicker, SheetDefaults, anywhere else) and add a
local skiko-side override. Do NOT ship the upstream patches — they
mask the bug; keep them in a local patches/ directory or as a
conditional `expect/actual` on wasi.

### Step 5 — Verify + commit

- Reproduce the crash with `TooltipInspectionCard` at test #11 →
  fix → re-run test #11 → must survive 100+ taps.
- Re-enable the affected widgets in `RealComposeApp.kt` (Material3
  DatePicker chevrons should now be safe to tap; verify no
  regression in SegmentedButton, Task27/Task28 smoke cards).
- Soak test 10 min with chevron taps every ~2 s → no SIGILL.
- Update `feedback_tooltip_sigill_wasi` memory with the resolution.
- Cross-update `feedback_popup_overlay` if the same fix closes the
  DropdownMenu animation issue.

## Reproducer (immediate, no rebuild needed)

```bash
# The currently-pushed cwasm has the bisect harness in non-crashing
# mode (test #16). To repro the SIGILL:

# 1. Edit wart-app/src/wasmWasiMain/kotlin/TooltipInspectionCard.kt:
#    change the @Composable fun TooltipInspectionCard body to the
#    test #11 layer:
#       TooltipBox(positionProvider = …, tooltip = { Text("hello") },
#                  state = rememberTooltipState()) {
#           Box(Modifier.fillMaxWidth().size(…).background(…)
#                       .clickable { taps++ }) { Text("tap me") }
#       }

# 2. Recompile + AOT + push:
cd ~/wart/wart-app && ./gradlew compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon
wasm-tools component embed --world my:skiko-gfx/skiko-ui ~/wart/wit/skiko-gfx.wit \
    build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm -o /tmp/embedded.wasm
wasm-tools component new /tmp/embedded.wasm \
    --adapt ~/wart/skiko/wasi_snapshot_preview1.reactor.wasm -o /tmp/skiko-component.wasm
wasmtime compile --target aarch64-linux-android --wasm component-model --wasm gc \
    --wasm function-references --wasm exceptions -o /tmp/skiko-component.cwasm /tmp/skiko-component.wasm
adb shell am force-stop com.example.wasmruntime
adb push /tmp/skiko-component.cwasm \
    "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
adb logcat -c && adb shell am start -n com.example.wasmruntime/android.app.NativeActivity

# 3. Tap the orange box. Crash within ~5 s.
```

Alternative repro on the currently-deployed cwasm: tap the `< >`
chevrons in the DatePicker card. Same crash, same signature.

## Out of scope

- Fixing the `DropdownMenu` expand-animation bug from
  `feedback_popup_overlay` — separate issue, separate root cause
  (the test #1 result definitively ruled out shared root). Track in
  its own task if anyone picks it up.
- Bumping wasmtime version — the host is on wasmtime 44 (see
  `wart-host/Cargo.toml`); upgrading may incidentally fix the trap
  visibility but doesn't help diagnose if the trap stays.
- Compose Multiplatform version bump — the upstream Tooltip code is
  reasonably stable across recent versions; behaviour likely
  unchanged.

## Risks

1. **The bug may be in wasmtime / cranelift GC codegen**, not in
   Compose at all. The user-visible symptom (TooltipBox-only crash)
   would still be Compose-shaped because TooltipBox happens to be
   the only Material3 widget that exercises whatever wasm GC pattern
   trips it. If Step 2's Compose instrumentation shows nothing
   anomalous, pivot to Step 3 (wasmtime trap surface) and treat the
   Compose side as a victim.

2. **The minimum repro may shrink further** after Step 1 — currently
   "real TooltipBox + clickable" but Step 1 may reproduce with a
   hand-built wrapper. If it shrinks to just "DisposableEffect +
   clickable + recompose", the bug isn't TooltipBox-specific and
   the fix lands in much broader infrastructure.

3. **Recompiling compose-multiplatform-core's foundation module to
   add logging takes 15-25 min per cycle** (see
   `scripts/rebuild-compose-wasi-skiko-depend.sh`). Budget for
   that — at least 4-5 cycles of instrumentation refinement.

## Verification checklist

- [ ] Step 1 outcome documented (does hand-built wrapper reproduce
      the crash?)
- [ ] At least one ClickableNode instrumentation cycle landed,
      `logMessage` output captured
- [ ] PC offset from one fresh crash decoded against the cwasm
      function index (`wasm-tools dump` or equivalent)
- [ ] Either fix landed OR documented decision to ship workaround
      patches with a clear upstream blocker note
- [ ] `TooltipInspectionCard` updated to confirm the fix at test
      #11 (100+ taps survive)
- [ ] `feedback_tooltip_sigill_wasi` memory updated with resolution
- [ ] CLAUDE.md task index row for #29 flipped to ✅

## Estimates

| Step | Wall time |
|------|-----------|
| 1. Re-localize on wrapper | 1 day |
| 2. Inside-clickable instrumentation | 2 days |
| 3. wasmtime trap surface | 1 day |
| 4. Workaround if needed | 0.5 day |
| 5. Verify + commit | 0.5 day |
| **Total** | **~5 days focused** |
