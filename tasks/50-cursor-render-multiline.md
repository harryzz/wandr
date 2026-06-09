# Task 50 — Visual cursor renders wrong position in multi-line BasicTextField

> **Status:** ✅ device-verified 2026-05-27. Bug was a stubbed
> `Paragraph.lineMetrics get() = emptyArray()` on the skiko-wasi
> side. Fix exposes per-line metrics from the host via 14 new
> `paragraph` WIT verbs (`prepare-line-metrics` + 13 per-field
> getters). Cursor now tracks selection across `\n` line breaks for
> both hardware-keyboard Enter and IME Enter paths.

## Why this task exists

Device-verified on Pixel 2 XL after the task-49 Enter fix landed.
Repro:

1. Launch wandr-app + wandr.ime.keyboard via the hybrid stack.
2. Tap a `BasicTextField` with `KeyboardOptions(singleLine = false)`
   (or `TextFieldState` with the multi-line default) containing
   `"hello world"`.
3. From the IME, press Enter (now sends `code-point=10` +
   `key-id=KEY_ENTER=13`).
4. Type a few letters.

**Expected:** new chars appear on line 2 with cursor blink at
end of line 2.

**Observed:** new chars *do* appear on line 2 (the `tfstate` log
shows `text="hello world\nhhhvbbvb"` and `sel=TextRange(19, 19)` —
selection IS at end of line 2), but the visual cursor blink renders
between `hello wo` and `rld` on **line 1**.

So model state is correct; rendering is the bug.

## Likely location

The Compose / skiko-wasi cursor-position calculation goes through
`Paragraph.getRectsForRange(start, end, …)` to find the screen
rect for a given offset. This function is implemented in:

  `skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/paragraph/`

…and was wired through to the host's `paragraph_impl.rs` during
task 14 (`feedback_softkeyboard` mentions exactly these calls).
The host's `skia_safe::textlayout::Paragraph` already handles
`\n` correctly when measuring line breaks — so the bug is most
likely on the *wasi* side, between the wasm-side cursor-offset
calculation and the host's call.

Three candidate spots:

1. **Skiko-wasi `Paragraph.getRectsForRange` returns flat coords.**
   The wasi binding may return a single `Rect[]` without per-line
   y offsets, and BasicTextField's cursor code picks the first.
   Fix: ensure each rect carries the correct line-relative Y.

2. **BasicTextField-on-wasi's cursor-position formula.** Compose
   computes `cursorOffset → (x, y)` itself in some paths. The
   wasi actual for `MultiParagraph.getCursorRect(offset)` may be
   missing or returning a flattened single-line value.

3. **TextLayoutResult `multiParagraph` not splitting on `\n`.**
   If TextLayoutResult treats the whole text as one paragraph
   without a real `MultiParagraph.lineCount` > 1, then any
   "offset > line 1 length" maps to column past end of line 1
   instead of column on line 2.

Most likely candidate is **(1) or (3)** — these are the routine
"missing wasi actual" / "stubbed multi-line support" gaps that
match the project's pattern.

## Steps

### Step 1 — Confirm the model is correct (~30 min)

- Capture a clean smoke transcript: focus a multi-line TextField,
  press Enter, type two more letters, take a screenshot.
- Verify `tfstate` log: `text="...\n..."` + `sel=TextRange(N, N)`
  where N is the offset on line 2 (not line 1).
- Take the same path through a hardware keyboard (push Enter via
  `adb shell input keyevent KEYCODE_ENTER` after focusing) to
  confirm the bug is NOT IME-specific. Both paths should hit the
  same render code in skiko-wasi.

### Step 2 — Find the cursor-render path (~1 h)

- Grep skiko-wasi for `cursorRect`, `getCursorRect`, `lineForOffset`,
  `getLineForOffset`, `MultiParagraph`. Trace from
  `BasicTextField` → `TextFieldDelegate` → `TextLayoutResult` →
  `MultiParagraph` → individual `Paragraph` calls.
- Identify the actual function returning the cursor `Rect` for
  the given offset. Log its inputs / outputs to see whether it
  knows about line 2.

### Step 3 — Compare with the in-canvas keyboard (~30 min)

The in-canvas keyboard (`wandr-app/.../WasiSoftKeyboard.kt`,
retired but still in tree) used the same code path. Verify whether
this same bug was present back then by inserting a `\n` via the
in-canvas keyboard in a multi-line TextField. If yes, this bug
predates task 47 — separate fix lands here regardless. If no, look
at what changed between the in-canvas and dedicated-guest IME
paths (mostly: nothing in the rendering — same skiko, same Compose,
same TextFieldState).

### Step 4 — Patch the wasi actual (~1 h)

Most likely fix shape: extend / add a wasi-side `MultiParagraph`
implementation that splits the text on `\n`, creates one Skia
`Paragraph` per line, and tracks per-line Y offsets so
`getCursorRect(offset)` returns `(line-x, line-y)` from the
right line.

Reference: the host already has `paragraph_impl.rs` with a real
`skia_safe::textlayout::Paragraph` that handles `\n` natively —
the question is whether the wasi binding forwards `getRectsForRange`
across the `\n` boundary correctly.

If the host side is already correct and the bug is purely in
how the wasi side aggregates results across lines, the fix is
~10-30 LoC in `skiko/.../wasmWasiMain/.../paragraph/` actuals.

### Step 5 — Smoke + memory (~30 min)

Verify on device:
- Multi-line TextField, IME-press-Enter, type → cursor blinks at
  end of line 2.
- Same with hardware keyboard Enter (should already work, but
  validate).
- Tap at column X of line 2 → cursor blinks at that column of
  line 2 (covers the `getGlyphPositionAtCoordinate` reverse
  direction too).

Memory: append a note to `feedback_softkeyboard` noting that
the in-canvas-keyboard fixes there did NOT cover multi-line
cursor positioning; the missing piece is now this task.

## File-touch map

| File | Why |
|---|---|
| `skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/paragraph/...` | wasi actual for cursor-rect-by-offset, likely the fix site |
| `wandr-host/src/paragraph_impl.rs` | Possible host-side adjustment if the existing API doesn't expose what skiko-wasi needs |
| `compose-multiplatform-core/.../wasmWasiMain/...` | If `MultiParagraph` itself needs a wasi-side implementation, that lives here |
| `tasks/50-cursor-render-multiline.md` | This doc; append results section on close |
| `MEMORY.md` → append to `feedback_softkeyboard` | Capture the multi-line gap |

## Considerations / risks

- **Single-line TextField uninvolved.** This task is multi-line
  only. Single-line fields don't have `\n` in their model so the
  cursor calc is trivially correct.

- **Touch tap on line 2 may exhibit the same bug.** If tapping at
  pixel (X, Y) on line 2 sets selection to a line-1 offset instead,
  that's the inverse of the cursor-rect bug —
  `getGlyphPositionAtCoordinate` not splitting on lines. Same
  fix area; verify both directions.

- **Compose's `TextLayoutResult.multiParagraph`** may be the
  abstraction layer that needs to know about line breaks. Check
  whether wasi has a real `MultiParagraph` actual or just the
  single-`Paragraph` shortcut.

- **Selection / drag handles.** Multi-line selection extending
  across lines may exhibit the same misalignment. Out of scope for
  the cursor-position fix but might fall out for free.

- **Hardware keyboard Enter.** Same dispatch_key_v2 path; the
  host sees `(code_point=0, key_id=13)` for `KEY_ENTER` from the
  winit branch. The IME now sends `(code_point=10, key_id=13)`.
  If the host's hardware path doesn't insert `\n`, the bug is
  WIDER than just the IME and we may need to make the hardware
  path send `\n` too. Verify in step 1.

## Related

- `tasks/47-ime-via-guest-app.md` step 3c — IME overlay where this
  bug was first observed.
- `tasks/49-ime-content-control.md` — the Enter-fix that surfaced
  this (IME now correctly produces `\n`; rendering of that `\n`
  in the field is the gap this task closes).
- `tasks/14-paragraph.md` — original wiring of `Paragraph` /
  `getRectsForRange` through the WIT. Touched the same code area.
- `MEMORY.md` → `feedback_softkeyboard` — the in-canvas-keyboard
  work that fixed `getRectsForRange` / `getGlyphPositionAtCoordinate`
  / `getWordBoundary` routing. Multi-line cursor was outside that
  scope.

## Results (2026-05-27)

**Outcome:** ✅ device-verified on Pixel 2 XL across both input
paths.

**Diagnosis:** Step 1 confirmed the bug was NOT IME-specific —
hardware-keyboard Enter (`adb shell input keyevent KEYCODE_ENTER`)
into wandr-app's `BasicTextField` produced the same symptom: text
correctly multi-line in the model (`tfstate` log shows
`"hello world\nhhhvbbhellohardware\nline2vb"` with selection
TextRange at end of line 3), but cursor blink stuck on line 1.

Step 2 located the bug at one specific line:
`skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/paragraph/Paragraph.kt:71`

```kotlin
val lineMetrics: Array<LineMetrics> get() = emptyArray()
```

Compose's `SkiaParagraph.getCursorRect(offset)` calls
`lineMetricsForOffset(offset)!!.baseline / ascent / descent` to
compute the cursor Y; with an empty array, the binary-search
helper degenerates to "always line 0" and the cursor renders on
line 1 regardless of offset.

**Fix:** 14 new WIT verbs on `my:skiko-gfx/paragraph` — one
`prepare-line-metrics` + 13 per-field getters mirroring the
existing `prepare-rects-for-range` two-stage cache pattern.
Implemented host-side in `wandr-host/src/paragraph_impl.rs`
(reading `skia_safe::textlayout::Paragraph::get_line_metrics()`,
which natively handles `\n`); wired through skiko-wasi's
`Paragraph.kt` so `lineMetrics` now returns a real
`Array<LineMetrics>` matching upstream skiko's data class
field-for-field. The host caches the array in a
`Vec<CachedLineMetrics>` (plain-data copy, no lifetime on the
parent Paragraph).

**Smoke transcript** on Pixel 2 XL (`tfstate` log):

```
text="hello world\njjjjjj\ntralalahi\nline2"  sel=TextRange(34, 34)
```

Screenshot confirms cursor blink at end of line 4 (`line2|`),
matching offset 34 (= 11 + 1 + 6 + 1 + 9 + 1 + 5).

**Files changed:**

- `wit/skiko-gfx.wit` + 3 mirrors — 14 new paragraph verbs.
- `wandr-host/src/canvas_impl.rs` — `CachedLineMetrics` struct +
  `para_line_metrics_cache: Vec<CachedLineMetrics>` field.
- `wandr-host/src/paragraph_impl.rs` — 14 new Host trait methods
  (`prepare_line_metrics` + 13 `get_cached_line_*` getters).
- `skiko/.../generated/{Internal,}SkikoUi.kt` — 14
  `@WasmImport` extern fun declarations + Import companion
  wrappers + interface method declarations.
- `skiko/.../paragraph/Paragraph.kt:71` — replaced
  `emptyArray()` stub with the real lookup.

**Commits:** wandr-host `8b16ab9`, skiko `09ab3d53`, wandr-app
`b3a73be`, wandr.ime.keyboard `4ccb590`, wandr (top) (this).

**Out of scope (lands later if observed):**

- The companion `Paragraph::getGlyphPositionAtCoordinate`
  (reverse direction — tap → offset) also potentially mishandles
  multi-line. Existing host impl uses
  `Paragraph::get_glyph_position_at_coordinate((x,y))` which
  Skia handles correctly across lines; if a smoke shows
  tap-on-line-2 placing the cursor on line 1, that's a separate
  small fix.
- Multi-line text selection (long-press + drag handles across
  lines) was not exercised by this fix. The same `lineMetrics`
  data unlocks correct selection-handle rendering on multi-line
  — should fall out for free.
- Compose's `Paragraph` callers that use `width` / `left`
  fields for alignment (e.g. centered multi-line text) now
  work too — same code path.

## Resume hints for fresh sessions

1. **Reproducer is in task 49's smoke flow.** Run
   `scripts/run-hybrid-stack.sh`; `wandr-arbiter launch
   com.example.wandr-app && launch-overlay wandr.ime.keyboard &&
   overlay wandr.ime.keyboard`; tap a TextField; press Enter on the
   IME; type a letter.
2. **Compare with hardware Enter first** (step 1). If hardware
   Enter has the same bug, the wider rendering path is broken
   independently of the IME — fix that and the IME inherits the
   fix.
3. **The host's `paragraph_impl.rs` likely already handles `\n`**
   in Skia. The gap is most likely on the wasi side aggregating
   per-line results.
4. **Don't touch BasicTextField source** in
   `compose-multiplatform-core/`. The fix is in
   `skiko/.../wasmWasiMain/.../paragraph/` (the
   `Paragraph` / `MultiParagraph` wasi actuals). Editing Compose
   core risks dragging in cross-platform changes; the actuals
   model is where wasi-specific gaps belong.
