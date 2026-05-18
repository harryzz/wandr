# Task 28 — Wire the abstract `org.jetbrains.skia.Canvas` to host-side skia

> **Status: 🔲 scoped 2026-05-18.** Make the 42 throw-stubs on the
> abstract `Canvas` class in
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
