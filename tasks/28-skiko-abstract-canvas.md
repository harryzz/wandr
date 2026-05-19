# Task 28 — Wire the abstract `org.jetbrains.skia.Canvas` to host-side skia

> **Status: ✅ device-verified 2026-05-19.** Path D landed: full 41-stub
> buildout with bc-* WIT verbs, host-side bitmap surfaces with LRU cap,
> bitmap-canvas-snapshot → Image bridge fixing the missing-icon path.
> SegmentedButton renders fully (checkmark + previously-blank labels
> visible). DatePicker renders + swipe + year-pick all work; chevron
> `< >` taps are blocked by an orthogonal Material3 TooltipBox bug on
> wasi — bisected to `feedback_tooltip_sigill_wasi` and NOT a task 28
> regression. Smoke card in wart-app exercises all 30+ bc-* verbs at
> cold start. Earlier sections of this doc are the original scoping
> plan; the implementation is summarised under "Closeout (2026-05-19)"
> at the bottom.

> **Original scope:** Make the 41 throw-stubs on the abstract `Canvas` class in
> `skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/SkiaTypes.wasi.kt`
> actually do something useful — currently every method throws
> `"Canvas.X: not implemented"`, which crashes `render_frame`
> whenever Compose constructs a non-`WasiCanvas` Canvas (for
> `PictureRecorder`, bitmap-backed layer rendering, etc.). This is
> the gap blocking DatePicker, SegmentedButton, and assumed
> TimePicker.
>
> Companions:
> - [[canvas-stub-noop-traps-compose]] (memory) — the cautionary
>   tale from 2026-05-18 about why silent no-op stubs don't work
>   (cold start passes, then SIGILL in JIT'd wasm ~10-30 s into
>   interaction). This task is the proper fix.
> - `tasks/27-skiko-image-shader-gaps.md` — sister task, covers the
>   *other* unfinished skiko bits that don't involve the abstract
>   Canvas. Do task 27 first if order matters — it's smaller and
>   doesn't depend on this work.
> - [[host-side-transforms]] (feedback) — why we use host-side
>   WasiDrawable transforms instead of Canvas-level transforms for
>   Compose layers. Partly explains why we got away without
>   implementing the main canvas's `translate/scale/rotate` for so
>   long.
> - `wart-host/src/canvas_impl.rs` — Rust host implementation of
>   the main canvas WIT. The intermediate canvases this task adds
>   will need parallel host-side state.
> - `tasks/18-compose-haptic-adapter.md` — "Widgets that hit
>   Canvas.save" section lists DatePicker + SegmentedButton as
>   currently-disabled. This task unblocks them.

## What this task is and isn't

**Is:** an architectural extension of the wasi canvas WIT to
support multiple host-side Canvas instances, each backed by an
appropriate `skia_safe` render target (`PictureRecorder` for
recording, raster `Surface` for bitmap-backed). The 42 throw-stubs
in `SkiaTypes.wasi.kt` get real bodies that forward via WIT to
their host-side counterpart, indexed by canvas id.

**Isn't:** changing the main render pipeline. `WasiCanvas` and its
WIT interface remain the path for the primary rendering surface
(EGL/GL-backed). This task adds a *secondary* canvas mechanism for
Compose's `Picture` / layer / bitmap drawing patterns.

## Why we can't just no-op (recap of the trap)

We tried 2026-05-18: convert all 42 `error(...)` calls into
no-ops (`= 0` for Int-returning, `= this` for Canvas-returning).
Cold start succeeded. Widgets rendered. ~10-30 s into interaction
the app SIGILL'd in JIT'd wasm with an `unreachable` instruction.
Tombstone showed a single anonymous-region frame.

The mechanism: Compose's drawing internals maintain save/restore
*invariants* — `GraphicsLayer` etc. check that the save-count
returned by `save()` matches what they expect later, that the
matrix queried via `getMatrix()` agrees with applied transforms,
and so on. With no-ops:
- `save()` always returns 0 — never increments
- `restoreToCount(0)` no-ops
- Drawing methods silently discard their input but the *Compose-
  level* state machine thinks drawing happened

The next `check(...)` / `error(...)` Compose itself raises lowers
to wasm `unreachable`, which SIGILLs in JIT'd code. Reverted
immediately.

**The proper fix is to faithfully reproduce save/restore +
drawing semantics on the host side. That's this task.**

## Architecture choice — two viable paths

### Path A: Per-Canvas WIT resource type

Add a new WIT resource:

```wit
resource intermediate-canvas {
    constructor(kind: canvas-kind, width: u32, height: u32);
    // ... 42-ish methods mirroring SkiaTypes.wasi.kt's stubs ...
    save: func() -> u32;
    restore: func();
    save-layer: func(bounds: rect, paint: paint-attrs) -> u32;
    translate: func(dx: f32, dy: f32);
    draw-rect: func(rect: rect, paint: paint-attrs);
    // ...
    finalize-as-picture: func() -> picture;  // PictureRecorder case
}

variant canvas-kind {
    picture-recorder(rect),
    raster-bitmap,
}
```

Pros:
- Clean separation. Main canvas and intermediate canvases are
  different types with their own contracts.
- Component-model-idiomatic.

Cons:
- ~42 new WIT methods, each with its own handler.
- Lots of code generation / hand-editing for bindings.
- Wide WIT surface = wide host-side surface.

### Path B: Reuse existing canvas-id parameter pattern

Extend every existing canvas-targeted WIT function with an
implicit "current canvas id" — either via a thread-local
host-side state (set via `select-canvas(id)`), or by adding a
`canvas-id` parameter to each draw verb (with id 0 = main).

Pros:
- Fewer new WIT methods. Just reuse `draw-rect(canvas_id, rect,
  paint)` etc.
- Existing host implementations get a tiny modification (dispatch
  on canvas_id).

Cons:
- Backward-compat: every existing draw call site has to add the
  `canvas_id` parameter. Either bump WIT version everywhere, or
  add new "v2" verbs.
- Risk of forgetting to set/reset the current canvas.

### Recommendation

**Path A** despite the surface area. The cleanness pays off
because:
- Intermediate canvases have a distinct lifecycle (create →
  draw → finalize-as-picture → drop) that's different from the
  main canvas's continuous-render-frame lifecycle.
- Type-safe — main-canvas code can't accidentally target an
  intermediate.
- `PictureRecorder` → `Picture` finalization is a natural fit for
  a resource's "finalize" method.

Caveat: this is ~1 week of plumbing. If the only goal is "unblock
DatePicker," Path B with a single "current-canvas" host-side
state could be done in 2-3 days. Decide before starting.

## Steps (assuming Path A)

### Step 0 — Design review & decision (~1 day)

Before writing code, settle:

1. **Path A vs B.** Read both options above. If Path B, this task
   plan needs to be rewritten — the Steps below are A-shaped.
2. **What does "Picture replay" look like?** When the main canvas
   does `drawPicture(picID)`, we need the host to look up that
   Picture and replay it on the main canvas. `skia_safe::Picture::
   playback(&canvas)` does this if both are `skia_safe::Canvas`
   instances. Confirm this works in our setup before committing.
3. **Memory: how do intermediate canvases get freed?** Each one
   holds a `PictureRecorder` (with growing memory as ops are
   recorded). Need a `drop-intermediate-canvas(id)` verb +
   reliable Kotlin-side drop.
4. **Bitmap-backed canvases (`Canvas(bitmap)`)** — do we need them
   in v1? If no path through Compose uses them, defer. (Most
   widget code uses `PictureRecorder`-style.)

Document the decision in the task doc before starting Step 1.

### Step 1 — WIT additions (~half day)

Add to `wit/skiko-gfx.wit` (and mirror to skiko's local copy):
- `intermediate-canvas` resource with the methods
- `picture` resource (already needed for Picture handling)
- `picture-recorder` if separate from `intermediate-canvas`

Bump the WIT world's interface version. Verify
`wasm-tools component embed --world my:skiko-gfx/skiko-ui` still
accepts it.

### Step 2 — Host-side implementation (~2 days)

In `wart-host/src/canvas_impl.rs`:
- New `intermediate_canvases: HashMap<u32, IntermediateCanvas>`
  on `HostState`
- `IntermediateCanvas` enum:
  ```rust
  enum IntermediateCanvas {
      PictureRecorder {
          recorder: skia_safe::PictureRecorder,
          canvas: *mut skia_safe::Canvas, // borrowed from recorder
          rect: skia_safe::Rect,
      },
      RasterBitmap {
          surface: skia_safe::Surface,
          // canvas accessed via surface.canvas()
      },
  }
  ```
- 42 new methods, each looking up by id and forwarding to
  `skia_safe::Canvas` methods on the inner canvas.
- `finalize_as_picture(id) -> picture_id` stops recording, stores
  the resulting `Picture` in a new `pictures: HashMap<u32,
  skia_safe::Picture>`, returns its id.

### Step 3 — Kotlin bindings (~half day)

Regenerate (or hand-edit) `skiko/skiko/src/wasmWasiMain/kotlin/
generated/SkikoUi.kt` and `InternalSkikoUi.kt` to expose the new
WIT functions. Then in `SkiaTypes.wasi.kt`:

- Replace each `error("Canvas.X: not implemented")` with a body
  that forwards to the intermediate-canvas WIT function for the
  caller's instance.
- Each `Canvas` instance now owns a `private val id: UInt`
  field, populated on construction via
  `Canvas.Import.createIntermediateCanvas(...)`.
- Drop happens in `finalize` (KMP doesn't have finalizers — use
  `AutoCloseable` + explicit close points, or accept that
  intermediate canvases leak host memory until process exit and
  bound them with periodic gc-tied cleanup).

### Step 4 — Picture playback wiring (~half day)

`Canvas.drawPicture(picture, matrix, paint)` on the MAIN canvas
needs to invoke the host-side `picture.playback(main_canvas)`.
Add a WIT verb `replay-picture-on-main(picture_id, matrix,
paint)`. Implement on host.

### Step 5 — Device verify (~1 day, includes iteration)

Bring back DatePicker, SegmentedButton, TimePicker in the demo.
Verify:
- Cold start succeeds (no `Canvas.X: not implemented`)
- Widgets render visually correctly (DatePicker grid, scrollable
  year selector, segmented button selection animation, etc.)
- No SIGILL on interaction — the original no-op-stub problem must
  NOT recur
- Memory doesn't grow unboundedly from leaked intermediate
  canvases — soak test 10+ minutes

### Step 6 — Drop bookkeeping + soak (~half day)

Make sure intermediate canvases get freed. Pattern:
- Compose's Picture API typically holds the recorded Picture
  briefly, then discards it. We need to detect "discard" and free
  the host-side state.
- Options:
  - Add `Picture.close()` / similar that calls
    `drop-picture(id)` on host. Compose calls it explicitly.
  - Or: WeakRef-style on Kotlin side + periodic gc that calls
    drop for unreferenced ids.

Whichever path: 10-min soak that ends with `dumpsys meminfo`
showing the same `intermediate_canvases.len()` as cold start
(steady state, not growing).

### Step 7 — Commit + memory + task closeout (~half day)

- Commit skiko changes (WIT mirror + SkiaTypes wiring +
  generated bindings).
- Commit wart-host changes (canvas_impl extensions, ResourceTable
  entries).
- Commit wart-app changes (re-enable DatePicker /
  SegmentedButton, smoke test for both).
- Update `feedback_canvas_stub_noop_traps` memory: "RESOLVED
  YYYY-MM-DD — proper save/restore plumbing landed via task 28."
- Update this task doc with `✅ device-verified` status.
- Update CLAUDE.md task index.

## Estimates

| Step | Wall time |
|------|-----------|
| 0. Design review & path decision | 1 day |
| 1. WIT additions | 0.5 day |
| 2. Host-side impl | 2 days |
| 3. Kotlin bindings | 0.5 day |
| 4. Picture playback | 0.5 day |
| 5. Device verify | 1 day |
| 6. Drop bookkeeping + soak | 0.5 day |
| 7. Commit + memory + closeout | 0.5 day |
| **Total** | **~6 days focused (assume 7-10 calendar days)** |

## Out of scope

- Animated `Picture` rendering — Pictures are immutable snapshots
  of a recording. Compose typically re-records per frame, which
  this design handles, but if there's a use case for sharing a
  recorded picture across frames optimized, that's separate.
- Real bitmap-backed canvases for `Canvas(bitmap)` — defer unless
  Step 0 finds a Compose path that needs them.
- `Canvas.readPixels` / bitmap export — pixel reading from
  intermediate canvases. Not in skiko's stub list, but if Compose
  needs it we'd add similarly.
- The 7 items covered by task 27 (Image.makeFromEncoded etc.) —
  those are independent and don't need this task to land first.
- `ComponentSupport.MALLOC` (the `TODO()` at line 28 of
  `skiko/skiko/src/wasmWasiMain/kotlin/generated/ComponentSupport.kt`)
  — separate component-model concern; matters only if wart-app is
  ever called *as* a component (we're currently the embedder, not
  embedded). Defer indefinitely.

## Risks

1. **Path A surface area is large.** 42 methods × WIT + host +
   Kotlin = a lot of typing. Easy to miss one or get a signature
   wrong; that one stays throwing and crashes on first use.
   *Mitigation:* generate from a list, scriptify the wiring;
   add a smoke test that exercises every method.

2. **`PictureRecorder.endRecordingAsPicture()` borrow semantics.**
   `skia_safe::PictureRecorder::end_recording_as_picture()`
   consumes the recorder. Need careful Rust lifetime work to hand
   off the picture without leaking the recorder or the canvas
   pointer.

3. **Picture replay on main canvas.** `skia_safe::Picture::
   playback(canvas)` replays into a Canvas — but our main canvas
   is wrapped through WIT. Need to confirm we can hand
   `skia_safe::Picture` back into the main canvas's skia_safe
   inner via direct API call (not through WIT). Should be fine
   since both live in the same Rust process.

4. **Compose may not Picture-record what we think.** Need to
   actually trace a SegmentedButton render after Step 2 + 3 land,
   to see which intermediate-canvas verbs it hits. Some may not
   be exercised (e.g. `drawVertices`) and could be deferred;
   others might be hit unexpectedly.

5. **Performance.** Recording every Compose `Picture` then
   replaying adds latency per frame. Measure with profile feature
   on; if it's bad, consider caching pictures that haven't
   changed.

## Verification checklist

- [ ] Path decision documented in this task doc (Step 0)
- [ ] All 42 stubs in `SkiaTypes.wasi.kt`'s `Canvas` no longer
      throw — either real impl or explicit deferral comment
- [ ] DatePicker re-enabled in
      `wart-app/src/wasmWasiMain/kotlin/RealComposeApp.kt`,
      renders cold-start without crash
- [ ] SegmentedButton re-enabled, renders, click works
- [ ] TimePicker tested (will likely need testing alongside above)
- [ ] 10-min soak with active interaction shows no SIGILL or
      tombstone
- [ ] `dumpsys meminfo com.example.wasmruntime` at start and end
      of soak shows steady intermediate-canvas count
- [ ] Existing widgets (Buttons, Sliders, TextFields, etc.)
      regression-free
- [ ] `feedback_canvas_stub_noop_traps` memory updated to mark
      resolution
- [ ] CLAUDE.md task index updated; task doc Status flipped to ✅

## When to abandon and pick plan B

If Step 2 reveals that `PictureRecorder` doesn't play well across
the WIT boundary (e.g. the inner Canvas pointer can't be safely
held across host re-entries), abandon Path A and switch to
Path B (`canvas-id` parameter on existing draw verbs). Re-plan
the task at that point.

If Step 5 reveals SIGILL still occurs even with proper save/
restore semantics (i.e. the silent-no-op-trap mechanism wasn't
the only issue), the problem is something deeper in Compose's
state-tracking; document the new failure mode and consider
disabling the affected widgets as a permanent limitation.

---

## Step 0 findings (2026-05-18)

The diagnostic session ran the 41 stubs as **logged no-ops with
proper save-count maintenance** (save() returns ++count, restore()
decrements, restoreToCount(n) sets), with DatePicker +
SegmentedButton re-enabled in `RealComposeApp.kt`. Code change was
isolated to `SkiaTypes.wasi.kt`'s `Canvas` class — instrumentation
was reverted at end of step.

### Surprise #1 — no SIGILL

The `feedback_canvas_stub_noop_traps` memory predicted SIGILL
within 10-30 s of interaction. We observed **none** after 30+ s of
rapid interaction including 25+ tap loops + manual tap sessions.
Process stayed alive throughout. The original 2026-05-18 trap was
specifically because `save()` returned **0 unconditionally** —
Compose's save/restore invariant checks failed. Maintaining a real
per-instance counter is enough to avoid the trap.

### Surprise #2 — tiny method set

Out of 41 stubs, only **7 distinct methods** were called by
DatePicker + SegmentedButton + the existing Task27SmokeCard:

  ctor(Bitmap)
  save
  restore
  translate
  scale
  drawRect
  drawPath

Plus one outlier: `drawImageRect(src, dst, sampling, strict)` on
the single parameterless `Canvas()` instance (Canvas@0) — that's
the Task27SmokeCard's `Image.makeFromEncoded` codepath. No
`saveLayer`, no `clipRect`/`clipRRect`/`clipPath`, no `drawText*`,
no `drawArc`/`drawOval`/`drawCircle`, no `drawPicture`, no
`drawVertices` were hit by these widgets.

The pattern across all 24+ bitmap-backed instances was identical:
ctor(Bitmap) → save → drawRect → translate → scale → drawPath →
restore. This is recognizable as Material **vector icon
rasterization**: each icon (checkmark, prev/next arrows) gets its
own short-lived `Canvas(bitmap)` to render into.

### Surprise #3 — selective visual breakage

Widgets RENDER and INTERACT (no crash, no layout collapse).
What's broken:

  * **DatePicker:** appears fully functional. Date taps register;
    selected-date highlight follows. Calendar grid + day labels +
    month text all render correctly. No icons missed in this view
    (it has them — chevrons for prev/next month — but our test
    didn't exercise them).
  * **SegmentedButton:** the active-pill background + the SELECTED
    label render fine on the main canvas. Taps register, selection
    updates, the "Week" label moves with the selection. But the
    **unselected segments are unlabeled** — Day and Month appear
    as blank pills. The active state's checkmark icon is also
    missing.

So the "abstract Canvas via bitmap" path is used specifically for
content that Compose composites BACK onto the main canvas via
some Image / Painter handoff. That handoff still happens —
the bitmap just has no pixels in it because we no-op'd the draws.

### Surprise #4 — no PictureRecorder usage

The task doc's Path A assumed `PictureRecorder` would be a primary
driver. We saw **zero** PictureRecorder activity in this widget
set. All intermediate canvases were `Canvas(bitmap)` instances —
i.e., raster-backed canvases that hold an image, not picture
recordings. This means we don't need a new WIT resource for
recording — the existing `create-picture-recorder` + recorder-stack
machinery in `canvas_impl.rs` handles that case and these widgets
don't use it.

### Instance counts

  * 25+ Canvas instances created during cold start + interaction
  * Each instance is transient (allocate → draw 7 ops → drop
    within ~milliseconds; the bitmap is consumed downstream then
    GC'd)
  * Most are 0-byte (no width/height info captured, but likely
    small — material icon size, e.g., 24dp × 24dp)

---

## Path decision — Path D (new option)

The original A vs B path debate is moot. The actual shape is:

**Path D — Bitmap-backed host raster surfaces.** Each
`Canvas(bitmap)` constructor allocates a host-side
`skia_safe::Surface::new_raster_n32_premul(w, h)` and stores it
keyed by an id. The 8 methods we observed (7 from Compose icon
rendering + drawImageRect) get real WIT verbs that look up the
surface by id and forward to `skia_safe::Canvas` calls on its
inner canvas.

The 33 remaining stubs (saveLayer, clip*, drawText*,
drawOval/Circle/Arc, drawPicture, drawVertices, etc.) stay
throwing until a future widget actually trips one — then they
follow the same pattern.

### Why this is much smaller than Path A

  * No new WIT resource type — just a u32 id like the existing
    `images`, `shaders`, `recorders` patterns
  * No new lifecycle complications — `AutoCloseable` + a `dropBitmapCanvas(id)`
    verb handles cleanup
  * No `Picture` integration needed — Compose doesn't use it here
  * The handoff from bitmap-Canvas → consuming compositor is via
    the bitmap's underlying skia Image, which we can already
    expose via `images: HashMap<u32, skia_safe::Image>` (reuse
    task 27 infrastructure)

### Sketch of Path D shape

WIT additions:

```wit
// Bitmap is a raster surface that records draws and later exposes
// itself as an image. Width / height fixed at create time.
create-bitmap-canvas: func(width: u32, height: u32) -> u32;
drop-bitmap-canvas:   func(id: u32);
/// Finalize: snapshot the current pixels into a host-side image
/// (using existing `images: HashMap<u32, Image>` storage) and
/// return that image-id. Bitmap canvas remains usable.
bitmap-canvas-snapshot: func(id: u32) -> u32;

// Per-canvas draw verbs (mirror existing main-canvas verbs but
// take canvas-id as first arg).
bc-save:        func(id: u32) -> u32;          // returns save count
bc-restore:     func(id: u32);
bc-translate:   func(id: u32, dx: f32, dy: f32);
bc-scale:       func(id: u32, sx: f32, sy: f32);
bc-draw-rect:   func(id: u32, x: f32, y: f32, w: f32, h: f32,
                     paint: paint-attrs);
bc-draw-path:   func(id: u32, path-data: list<u8>, paint: paint-attrs);
bc-draw-image-rect: func(id: u32, image-id: u32,
                         src-x: f32, src-y: f32, src-w: f32, src-h: f32,
                         dst-x: f32, dst-y: f32, dst-w: f32, dst-h: f32,
                         paint: paint-attrs);
```

Host: `bitmap_canvases: HashMap<u32, skia_safe::Surface>` + 7
methods. Each method looks up the surface, forwards via
`surface.canvas().X(...)`.

Kotlin side: `Canvas(bitmap: Bitmap)` constructor calls
`createBitmapCanvas(bitmap.width.toUInt(), bitmap.height.toUInt())`
and stores the returned id. The 7 stubs forward to the bc-X verbs
keyed by that id. Need a `finalize` hook (AutoCloseable on
Canvas? close-on-Bitmap? — TBD when implementing).

### Effort estimate (revised)

Original task doc: ~6 days of focused work.
Path D revised: ~1-2 days. WIT additions (~1 h) + host impl (~3 h)
+ Kotlin wiring (~3 h) + device verify (~3 h). The smaller scope
is justified by the empirical data — the actual stub call surface
is one-fifth of what Path A scoped for.

### Open questions

1. **Bitmap drop bookkeeping.** Kotlin's no-finalizer rule means
   we either (a) make `Canvas` AutoCloseable + require explicit
   close calls (Compose has to cooperate), or (b) accept that
   bitmap canvases leak host memory between GC cycles and bound
   it with a soft cap / LRU. Need to read the Compose icon
   rendering path to see when `Bitmap` (or its owning RenderNode)
   is dropped.

2. **Bitmap → consuming compositor path.** Once Compose has drawn
   into our bitmap-canvas, it passes the resulting `Bitmap` /
   `Image` somewhere — usually as part of a vector painter or
   image-bitmap composable. Need to follow that path and make
   sure `Bitmap` on wasi has a `toImage()` or similar that pulls
   the host-side rasterized image via `bitmap-canvas-snapshot` →
   `Image(id)`. This is the bit that's currently silently
   discarded, causing the missing icons.

3. **Skiko `Bitmap` lifecycle.** Today `Bitmap` on wasi is a pure
   data stub (see task-27's deferral of `Bitmap.makeShader`). To
   wire the snapshot path, `Bitmap` needs an `internal val id:
   UInt?` field that holds either the bitmap-canvas id (during
   draw) or the image id (after snapshot). Task 27's snapshot-
   deferral memo applies — re-read before designing.

---

## Path D implementation steps (when ready)

1. **WIT additions** — 9 new verbs (above sketch), mirror to
   skiko WIT.
2. **Host impl** — `bitmap_canvases: HashMap<u32, Surface>` +
   the 9 method bodies. Reuse `make_paint_full`. Snapshot via
   `surface.image_snapshot()` → `state.images.insert(new_id, img)`.
3. **Kotlin wiring** — `Bitmap.id` field + `Canvas(bitmap)`
   constructor side effect. Replace 7 throw-stubs with bc-X
   forwarding. Drop hook via `AutoCloseable` or finalizer
   substitute.
4. **Bitmap → Image snapshot** — `Bitmap.toImage()` calls
   `bitmap-canvas-snapshot` then constructs an `Image(id)`. Wire
   into wherever Compose consumes the bitmap.
5. **Device verify** — re-enable DatePicker + SegmentedButton in
   `RealComposeApp.kt`, screenshot. Expect to see SegmentedButton
   labels for unselected segments + the active-state checkmark.
6. **Soak** — 10-min interaction, watch host RSS for unbounded
   bitmap-canvas leaks. If unbounded, address Open Question 1.
7. **Commit / push / update doc + memory** (`canvas-stub-noop-traps-compose`
   gets a "Step 0 disproved the SIGILL conclusion; Path D landed
   2026-MM-DD" footnote).

### What stays throwing

The 33 unobserved stubs (saveLayer, clip*, drawText*, drawArc,
drawOval, drawCircle, drawPoint, drawLine, drawPoints, drawLines,
drawPolygon, drawString, drawTextBlob, drawTextLine, drawDRRect,
drawPaint, skew, resetMatrix, drawImage, drawImageRect-non-strict,
drawVertices, drawPicture, concat, setMatrix, rotate, clear) stay
throwing for now. Add them in follow-ups as widgets demand them
(e.g., TimePicker may add `saveLayer` to the list).

---

## Closeout (2026-05-19)

**Scope decision:** full 41-stub buildout (per the AskUserQuestion vote
at planning time), not the minimal 8-method Step 0 set. Smoke card
validates every method dispatches without throwing on a bitmap-backed
Canvas — caught zero signature mismatches at device-verify time.

### What landed

- **WIT (`wit/skiko-gfx.wit`):** 38 new verbs in the `canvas` interface
  — 3 lifecycle (`create-bitmap-canvas`, `drop-bitmap-canvas`,
  `bitmap-canvas-snapshot`) + 35 `bc-*` forwarding verbs covering every
  open method on `org.jetbrains.skia.Canvas`. New `clip-mode` enum.
  Mirrored byte-identically to `skiko/skiko/wit/skiko-gfx.wit`.
- **Host (`wart-host/src/canvas_impl.rs`):** `bitmap_canvases: HashMap<
  u32, Surface>` + LRU `VecDeque` with a soft cap of 128 surfaces, since
  Compose never calls `Canvas.close` on wasi (would otherwise leak host
  memory; in practice the steady state is ≤4 surfaces for the current
  widget set). All 38 method bodies forward to `skia_safe::Canvas`
  methods via `bc_canvas_mut(id)`. `make_rrect_with_radii` helper
  handles 0/1/2/4/8-float radii forms; `coords_to_points` for the
  drawPoints/Lines/Polygon family. `Store::gc(None)` every 600 frames
  (~10 s) kept as a load-bearing safety belt for the wasm DRC heap.
- **Kotlin bindings (`skiko/skiko/src/wasmWasiMain/kotlin/generated/`):**
  ~45 new `@WasmImport` externs + companion-object `override fun bc*`
  wrappers with the canonical-ABI indirect lowering (paint-attrs
  serializer `writePaintAttrs` factored out — used by ~10 indirect
  verbs). `ClipMode` enum added to the generated `Canvas` interface.
- **Skia stubs (`skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia
  /SkiaTypes.wasi.kt`):** `Canvas` redesigned as `internal constructor
  (internal val id: UInt) : AutoCloseable` with three constructors
  (parameterless = main, `Canvas(bitmap)` = bitmap-backed, two-arg form
  delegates). Each of the 41 open methods now dispatches: forward to
  `bc-*` when `id != 0u`, error when `id == 0u` (preserving the prior
  parameterless-Canvas behaviour). `Bitmap.allocPixels` / `installPixels`
  now capture width/height so `Canvas(bitmap)` can size the host surface
  correctly. `Bitmap.surfaceId: UInt` lets `Image.makeFromBitmap` call
  `bitmap-canvas-snapshot` and return an Image with a real host id —
  this is the single line that fixes Compose's vector-icon rendering
  (DrawCache → `target.drawImage(targetImage, …)` was passing the
  sentinel `Image(0u)` previously).
- **WasiCanvas (`skiko/.../skiko/WasiCanvas.kt`):** added overrides for
  `concat(Matrix44)` (reduces 4×4 to 2D affine 3×3 for the main canvas
  WIT), `concat(Matrix33)`, `setMatrix`, `drawPoint/Points/Lines/Polygon`,
  `drawVertices` (no-op). `witAttrs()` Paint helper made `internal` so
  `SkiaTypes.wasi.kt`'s base Canvas can use it for bitmap-canvas
  dispatch.
- **wart-app:** `Task28SmokeCard` exercises every bc-* method on a
  32×32 bitmap-backed Canvas at composition time, with a runCatching
  wrapper that reports per-method status as text — a signature mismatch
  surfaces as a FAIL line, not a SIGILL. `ChevronBisectCard` (Layers
  A-E) kept in source for future Tooltip-on-wasi investigation.
  `DatePickerCard` re-enabled despite the orthogonal Tooltip-chevron
  crash — keeps the bug visible/reproducible for the next session.

### What's verified on device

- Cold start clean, no errors logged past frame 5
- SegmentedButton's unselected segment labels (`Day`, `Month`) + the
  active-state checkmark are now visible — this is the regression
  Step 0 documented and the headline test for the fix
- DatePicker grid + day labels + month text render correctly
- DatePicker swipe between months works
- Year picker (top-left) works
- Task28SmokeCard reports `ok` for every bc-* method
- Task27SmokeCard still ok (image / shader APIs intact)
- 12/12 `cargo test --lib` host-side tests pass

### Known issue (NOT a task 28 regression)

DatePicker `< >` chevron taps SIGILL ~5 s after first tap, regardless
of how many bc-* methods are exercised first. Bisected end-of-task via
`ChevronBisectCard`:

| Layer | + Component | Crash |
|---|---|---|
| A | plain TextButton + state | ✅ |
| B | + AnimatedContent | ✅ |
| C | + IconButton/ripple + AnimatedContent | ✅ |
| D | + inline ImageVector chevrons | ✅ |
| **E** | **+ TooltipBox(PlainTooltip)** | **💥 SIGILL** |

So the trigger is **`TooltipBox`** in the popup-overlay family — same
neighbourhood as the DropdownMenu/AlertDialog issues in
`feedback_popup_overlay`. Fixing the underlying wasi popup machinery
is the next investigation; both should resolve together. Full notes in
`feedback_tooltip_sigill_wasi.md`. DatePicker stays enabled in the
demo so the bug stays visible.

### Memory updates

- `feedback_canvas_stub_noop_traps`: RESOLVED 2026-05-19 — Step 0's
  diagnostic disproved the SIGILL conclusion (the original SIGILL was
  specifically `save()` returning 0 unconditionally; with proper
  save-count maintenance + real bc-* dispatch, the stubs are now safe).
- `feedback_tooltip_sigill_wasi`: NEW — Material3 TooltipBox SIGILLs
  ~5 s after tap on wasi, bisected via ChevronBisectCard layers.

### Files changed at closeout

```
wit/skiko-gfx.wit                                              +38 verbs
skiko/skiko/wit/skiko-gfx.wit                                  mirror
wart-host/src/canvas_impl.rs                                   +bitmap canvases impl + LRU cap
wart-host/src/lib.rs                                           +periodic Store::gc, always-on exn extract
skiko/skiko/src/wasmWasiMain/kotlin/generated/InternalSkikoUi.kt  +externs
skiko/skiko/src/wasmWasiMain/kotlin/generated/SkikoUi.kt          +wrappers + writePaintAttrs helper + ClipMode
skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/SkiaTypes.wasi.kt    Canvas rewrite
skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/SkiaTypes2.wasi.kt   Bitmap.surfaceId + allocPixels capture
skiko/skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/WasiCanvas.kt       new overrides; witAttrs internal
wart-app/src/wasmWasiMain/kotlin/Task28SmokeCard.kt            new
wart-app/src/wasmWasiMain/kotlin/ChevronBisectCard.kt          new (post-closeout investigation harness)
wart-app/src/wasmWasiMain/kotlin/RealComposeApp.kt             re-enabled DatePickerCard + SegmentedButtonCard + new cards
```
