---
name: In-canvas soft keyboard — FULLY WORKING after paragraph WIT fixes
description: WasiSoftKeyboard composable + bridge to realScene.sendKeyEvent works end-to-end. Cursor positioning, tap-to-move-cursor, long-press-to-select-word ALL work as of 2026-05-13 once the upstream SkiaParagraph chain got real layout queries (getRectsForRange, getGlyphPositionAtCoordinate, getWordBoundary) routed via new paragraph WIT functions to the host's skia-safe Paragraph instance. Previous cursor-stays-at-0 / re-tap-doesn't-move bugs were stubbed-empty-array issues, not snapshot-observer bugs.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
## What works (end-to-end)

A pure-Kotlin/Compose in-canvas soft keyboard, modeled after egui_keyboard's approach:

- **`WasiSoftKeyboard.kt`** in test-app: layout-pluggable composable with built-in QWERTY-English, Cyrillic-Bulgarian, Symbols, Symbols2, Emoji-starter pages. The `KeyboardLayout` / `KeyDef` / `KeyAction` API is designed so new languages or emoji pages are just one more data declaration each.
- **Key delivery** from soft-key tap → `wasiSoftKeyboardKeyHandler` (top-level `var` in RealComposeApp.kt) → `realScene.sendKeyEvent(KeyEvent(Key, KeyDown/Up, codePoint))` + `wasiFrameDispatcher.flush()`. Same path the hardware `WasiInput.setKeyHandler` uses.
- **Avoid focus theft on key tap**: use `Modifier.pointerInput(Unit) { detectTapGestures(onTap = …) }` instead of `Modifier.clickable { … }`. `Modifier.clickable` installs a `FocusableNode` that steals focus from the BasicTextField on each key tap → the focused field loses focus → KeyEvents go to no recipient. `detectTapGestures` doesn't request focus, so the BasicTextField stays focused.
- **Cursor color**: default `BasicTextFieldDefaults.CursorBrush = SolidColor(Color.Black)` is invisible against dark Material theme background. Set `cursorBrush = SolidColor(MaterialTheme.colorScheme.primary)` explicitly.
- **Cursor thickness**: `DefaultCursorThickness` desktopMain default is `1.dp` (≈3-4 raw pixels at 3.5x density — barely visible). compose-foundation-wasi overrides via `commonReplacements/.../text/TextFieldCursor.wasi.kt` → `3.dp` (≈10 raw pixels, matches Android xxhdpi look).
- **Layout**: place soft keyboard as a Box at `Alignment.BottomCenter` overlaying the scrollable content. Add `bottom = keyboardHeight + 16.dp` padding to the scroll Column so the last card isn't permanently hidden.

## Cursor positioning / tap-to-move / long-press all work

Root cause of "cursor stays at position 0", "re-tap doesn't move cursor", and "long-press doesn't select word" was the same: **our wasi `org.jetbrains.skia.paragraph.Paragraph` stub returned `emptyArray()` / `PositionWithAffinity(0)` / `IRange(offset, offset)` for the layout query methods**. Compose's upstream `SkiaParagraph` then fell through to `TextAlign.Start → 0f` or offset=0 across the board.

The fix was three host-side queries exposed through the WIT `paragraph` interface, all dispatching to skia-safe's textlayout API on the host:

| WIT function | Backed by | Used by |
|---|---|---|
| `prepare-rects-for-range(id, start, end, hm, wm) -> count` + `get-cached-rect-{left,top,right,bottom,direction}(index)` | `paragraph.get_rects_for_range(start..end, RectHeightStyle, RectWidthStyle)` returns `Vec<TextBox>` cached on the renderer | SkiaParagraph.getHorizontalPosition (cursor x position), getPathForRange (selection highlight) |
| `get-glyph-position-at-coordinate(id, x, y) -> u32` | `paragraph.get_glyph_position_at_coordinate((x, y))` returns `PositionWithAffinity` | SkiaParagraph.getOffsetForPosition (tap-to-position-cursor, long-press start) |
| `get-word-boundary-start/end(id, offset) -> u32` | `paragraph.get_word_boundary(offset)` returns `Range<usize>` | SelectionAdjustment.Word path during long-press |

The prepare-+-indexed-getter pattern is used because the existing WIT/Kotlin bindings don't support `list<f32>`-style return marshaling. Host stores `Vec<TextBox>` in `SkiaRenderer.para_rect_cache` (one-slot, refreshed per prepare call).

## Touch selection — how it actually triggers

Confirmed working gestures (matches Android / iOS standard):

1. **Double-tap + small slide** — double-tap arms word-selection at the tapped offset, the subsequent slide expands the highlight one word at a time (clicksCounter > 1 in `awaitSelectionGestures` takes the `touchSelectionSubsequentPress` path, which starts looking for drag immediately).
2. **Press-and-hold ~500ms+ + small slide** — long-press at first click takes the `touchSelectionFirstPress` path: `awaitLongPressOrCancellation` fires the long-press, then `drag(longPress.id) { observer.onDrag(...) }` waits for finger motion to expand the selection.
3. **Without any slide** — nothing visible happens. This is *expected*. The first long-press alone doesn't draw the word highlight; the drag is what calls `observer.onDrag` → `updateSelection(adjustment = Word)` → `getWordBoundary` → highlight path. Same on Android and iOS.

Long-press dwell threshold: 500ms (`viewConfiguration.longPressTimeoutMillis`). Pointer-event-scope's `withTimeout` schedules `delay(492) + delay(8)` via our `WasiFrameDispatcher`. The dispatcher fires delays ~10ms EARLY relative to wall clock because `nowMillis()` reads `cachedNanoTime()` which is set at frame start, not "now" — so deadlines computed from frame-start time elapse on the very next frame that crosses them. Long-press effectively triggers at ~480-490ms wall-clock.

`hapticFeedBack?.performHapticFeedback(HapticFeedbackType.LongPress)` is wired in TextFieldTextDragObserver.onStart but our wasi HapticFeedback is a stub, so users don't get the usual buzz cue that the long-press fired. Worth wiring up a real haptic via WIT one day.

## Visibility / dismiss (added 2026-05-26)

The keyboard is no longer always visible — it auto-shows on TextField
focus, auto-hides on blur, has a ⌄ key + responds to ESC.

- **`WasiKeyboardController`** (wandr-app side, implements
  `SoftwareKeyboardController`): holds a `MutableState<Boolean> isVisible`;
  `show()`/`hide()` toggle it. Provided via
  `CompositionLocalProvider(LocalSoftwareKeyboardController provides ...)`
  in `MaterialDemoApp`, which also reads `isVisible.value` to decide
  whether to render the keyboard. Net effect: Compose's internal
  `requireKeyboardController().show()` / `.hide()` calls from
  `BasicTextField` (tap-while-focused, `ImeAction.Done`) now drive the
  in-canvas keyboard naturally.
- **Auto-show on focus**: `TextFieldCard`'s `BasicTextField` has
  `Modifier.onFocusChanged { fs -> if (fs.isFocused) ctrl.show() else
  ctrl.hide() }`.
- **⌄ Hide key**: added to every layout's bottom row. Wired via the
  existing `KeyAction.Hide` enum case to `onHide = { ctrl.hide() }` on
  `WasiSoftKeyboard`.
- **Hardware ESC**: `Main.kt`'s `WasiInput.setKeyHandler` peeks at
  `KeyDown(keyId=27)` and calls `wasiHideKeyboardRequest()` (a small
  top-level `var` bridge — sibling to `wasiSoftKeyboardKeyHandler` —
  that `MaterialDemoApp` binds to `ctrl.hide()`).
- **Limitation accepted**: tap-on-non-field-non-button doesn't blur the
  field, so the keyboard stays up until ESC/⌄/another focus target.
  Matches Android's default `BasicTextField` behavior; a
  `Modifier.clickable {}` on the Surface would clear focus on outside
  tap if ever desired.

## How to apply

Don't blame the snapshot observer for "state-changes-but-view-doesn't" bugs in text rendering. The wasi `org.jetbrains.skia.paragraph.Paragraph` is a thin handle around a host-side skia Paragraph; ALL its query methods must route through WIT, not return defaults. If a Compose text-related feature breaks, check the Paragraph stub first.

The fix pattern for adding more Paragraph query methods:
1. Add WIT function in `wit/skiko-gfx.wit` `interface paragraph { … }`. Sync to `/home/harry/skiko/skiko/wit/skiko-gfx.wit`.
2. Implement in `host/src/paragraph_impl.rs` against `skia_safe::textlayout::Paragraph`.
3. Hand-extend `skiko/src/wasmWasiMain/kotlin/generated/InternalSkikoUi.kt` (the `@WasmImport` decls) and `SkikoUi.kt` (the `Companion object Import` + interface `fun` decls).
4. Update `org/jetbrains/skia/paragraph/Paragraph.kt` to call the new WIT instead of returning a stub.
5. Use prepare-+-indexed-getter to avoid `list<T>` return marshaling.
