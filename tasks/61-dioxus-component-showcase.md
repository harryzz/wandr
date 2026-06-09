# Task 61 — dioxus-canvas component showcase + renderer capability expansion

> **Status:** ✅ all 3 phases device-verified 2026-05-29. `wandr.dioxus.demo` is
> now a 5-tab Compose-style gallery (Inputs / Pickers / Calendar / Color / Text)
> covering checkbox, switch, radio, stepper, progress, dropdown, color swatches,
> calendar, **slider (drag)**, **HSV color picker (drag)**, and a **text edit box
> with the real soft keyboard (IME)**. The `dioxus-canvas` renderer grew drag
> (pointer move/up + capture + element-relative coords) and keyboard/focus +
> IME-attach. No new host WIT verbs — reuses the existing `paragraph`, `ime`,
> and `renderer.on-key-event-v2` interfaces.

## Goal

A tabbed component gallery (tabs avoid needing scroll — each page fits a
screen) demonstrating: checkbox, radio group, switch, segmented control / tabs,
stepper, progress bar, **dropdown**, **color picker**, **calendar**,
**slider**, and a **text edit box**. Proves dioxus-canvas is a real Compose
alternative for system guests.

## Renderer capabilities needed (the hard part)

The task-59 renderer only handles pointer **down** + solid-fill paint. The
gallery needs three expansions:

1. **Drag input** (slider, HSV color picker): wire `on_pointer_event_v2` for
   `Move`/`Up` (today only `Down`). Add **pointer capture** — a down on an
   element with a move/up listener captures the pointer so subsequent
   moves/ups route to it even outside its rect (sliders need this). Dispatch
   dioxus `onmousedown`/`onmousemove`/`onmouseup` (convert_mouse_data already
   exists). Event coords are surface-absolute; sliders read them against the
   element's cached layout rect (renderer exposes the hit rect → handler).
2. **Keyboard + focus + IME** (edit box): track a focused element; implement
   `convert_keyboard_data`; route `on_key_event_v2` → dioxus `onkeydown` on the
   focused node. For the on-screen keyboard, the guest participates in the
   `wandr:ime` editor-attached protocol (signal editor focus to the host →
   wandr.ime.keyboard overlay shows → keys route back via the host socket →
   `on_key_event_v2`). Mirrors the Kotlin app's IME path (tasks 47/49).
3. **Gradients / extra draws** (HSV picker): add `draw-oval` + the gradient
   create verbs to the trimmed WIT, and a gradient-fill path in the renderer's
   paint model (today `PaintProps` is solid-only). Saturation×value square +
   hue strip.

## Phases (each device-verified)

| # | Phase | Components | Renderer work |
|---|-------|-----------|---------------|
| 1 | Click-only gallery + tab nav | checkbox, radio, switch, tabs/segmented, stepper, progress bar, dropdown (inline-expand), color **swatch** grid, calendar | none (proven: conditional rendering works, `tests/render.rs::conditional_rendering_toggles`) |
| 2 | Drag inputs | slider (drag), HSV color picker | pointer move/up + capture; `draw-oval`; gradient fill |
| 3 | Text input | edit box (focus, cursor, backspace) + soft keyboard | `convert_keyboard_data`; focus; `on_key_event_v2` routing; `wandr:ime` editor-attached integration |

## Design notes / decisions

- **Tabs as top-level nav** — sidesteps scrolling (no clip/scroll in the
  renderer yet). Each tab's content fits one screen.
- **Reusable dioxus components** — `#[component] fn Checkbox(...)` etc., so the
  gallery is real component code, not one giant `rsx!`.
- **Dropdown = inline-expand** (no popup/overlay/z-layer in the renderer);
  clicking the header expands options below, pushing content down.
- **Circles** (radio, color dots) = `rrect` with radius ≥ half-size (Skia
  clamps to a circle) — handle `border-radius:50%` in the painter by resolving
  against `min(w,h)/2`. No new primitive for round corners.
- Confirmed with user 2026-05-29: full soft-keyboard IME (not hardware-only) +
  real drag (not tap-to-set).

## Status log

- Renderer de-risk: `conditional_rendering_toggles` host test added + green —
  add/remove mutation path works (checkbox/dropdown/tabs depend on it).
- **Phase 1 ✅ device-verified 2026-05-29.** Tabbed gallery (Inputs / Pickers /
  Calendar) with checkbox (✓), switch (pill + circular knob), radio group,
  stepper, progress bar, dropdown (inline-expand), color swatch picker, and a
  month calendar — all rendering correctly on the Pixel 2 XL (3 screenshots).
  Built as reusable `#[component]`s; `match tab()` swaps panels (the tested
  conditional-rendering path). Renderer change: percent `border-radius` (e.g.
  `50%`) resolved against the laid-out `min(w,h)` at paint time → circles/pills
  via Skia rrect clamping (no new draw primitive). Glyph note: the resolved
  Roboto font lacks the geometric triangles (▼▲◀▶ → tofu); used ASCII (`v`/`^`,
  `<`/`>`) — a host glyph-fallback to NotoSansSymbols would let us use the nice
  arrows (future polish). 564 KB. Demo defaults to the Inputs tab.
- **Phase 2 ✅ device-verified 2026-05-29.** Drag inputs: a **slider** (Inputs
  tab) and a **HSV color picker** (new Color tab). Renderer grew pointer
  **move/up + capture + element-relative coords**: `on_pointer_{down,move,up}`
  dispatch dioxus `mousedown`/`mousemove`/`mouseup` (a down on an element
  listening for move captures the pointer; moves/ups route to it, clamped to its
  box); the dispatched `MouseData` carries element-relative `element_coordinates()`
  (what sliders/pickers read). New host test `drag_reports_element_relative_coords`
  (capture + relative coords + release). **No gradient primitive** — the HSV
  hue strip (24 segments) + saturation/value grid (12×8) are discretized into
  solid cells; drag updates the selected indices, picked colour computed +
  previewed. **On-device drag verified**: an `adb input swipe` across the hue
  strip moved the selection blue→magenta (preview + hex + grid recolor all
  updated) — confirming the full InputFlinger → `on_pointer_event_v2` →
  capture → element-relative → re-render path. (InputDispatcher focus is on
  wandr even when WMS `mCurrentFocus` shows the launcher, so injected pointer
  events reach the guest.) Renderer change: `border-radius:50%` percent handling
  (phase 1) covers the round thumbs/dots. 592 KB.
- **Phase 3 ✅ device-verified 2026-05-29.** Text edit box + soft keyboard.
  Renderer: `convert_keyboard_data` (maps the host's `(code-point, key-id)` —
  Compose-Key ids: 8=Backspace, 13=Enter, … — to a dioxus `Key`); focus tracking
  (`F_KEY` flag → tap a keydown-listening element focuses it); `on_key(down,
  code_point, key_id)` dispatches dioxus `keydown`/`keyup` to the focused field.
  New host test `keyboard_types_into_focused_field` (focus → type → backspace).
  Guest: a `TextPanel` whose field tap calls `ime::notify_editor_attached` (the
  guest imports the host `ime` interface — same one the Kotlin app uses; the host
  forwards to the arbiter which shows `wandr.ime.keyboard`) and whose `onkeydown`
  edits the String (Character/Backspace/Enter); a Done button calls
  `notify_editor_detached`. **Full loop device-verified**: tap the dioxus field →
  arbiter logs `attach-editor … → route to wandr.ime.keyboard delivered=true` →
  the QWERTY overlay appears → tapping keys routes `ime-send-key-event <cp> 0` →
  the demo's per-host socket → `on_key_event_v2` → `on_key` → field updated
  ("edit me" → "edit mewart"). 624 KB. Note: raw `adb input text` (hardware-key
  path) does NOT reach the arbiter-launched guest — it needs InputDispatcher key
  focus / periodic `sf_request_focus` — but the IME→arbiter→socket path is
  independent of that and works.
- **UI scale factor (follow-up, 2026-05-29).** Added a renderer `set_scale(f32)`:
  guest styles are authored in **logical px**, and the renderer multiplies all
  taffy lengths + font sizes + px radii by the scale (layout/paint in scaled
  physical px) while **dividing element-relative input coords by the scale**
  (so slider/HSV math stays logical — host test `drag_coords_stay_logical_under_scale`).
  The demo drives it at runtime: a header `−`/`+` control writes a global
  `UI_SCALE` atomic that `render_frame` reads and pushes via `set_scale` each
  frame (avoids re-entering the borrowed renderer from a button handler).
  Default **1.5×** (the panel is hi-dpi). Device-verified: `+` took it
  1.50×→1.75×→2.00× live, whole UI scaling. 6 host tests total.
- **Vertical scrolling (follow-up, 2026-05-29).** Long pages / large scales now
  scroll. Content lays out at **natural height** (root height `Auto`, so the flex
  column can exceed the surface); `scroll_y` is applied at paint-replay
  (`y - scroll_y`) and hit-test (`y + scroll_y`) time — **scrolling needs no
  relayout**, just a repaint. Reworked the touch model into tap/scroll/slider:
  a press on a draggable element (slider/HSV) captures it as before; otherwise
  the press starts a gesture that becomes a **scroll** once it moves past
  `SCROLL_THRESHOLD` (16px), or a **tap → click on release** if it doesn't move
  (click moved from down→up; existing buttons unaffected). Found+fixed a latent
  bug: the slider-down path wasn't marking the renderer dirty, so a slider value
  change didn't repaint. Device-verified: at 2.25× a swipe scrolled the Slider
  card (below the fold) into view.
- **Sticky-header scroll regions via a clip-rect verb (follow-up, 2026-05-29).**
  Upgraded root-scroll → a proper `overflow:scroll` **region** so the tab bar
  stays sticky. Added `save`/`restore`/`clip-rect` to `CanvasSink` + the demo's
  trimmed WIT (forwarding to the host canvas verbs). `style.rs` maps
  `overflow:scroll` → `taffy Overflow::Scroll` (+ `flex-shrink`). The painter
  brackets the region's ops in `PushClip`/`PopClip` (replay = save + clip-rect +
  offset by `scroll_y`); hit-testing maps scrolled hits to their visible rect and
  clips to the viewport; a press inside the region scrolls *it* (header presses
  don't); a scrollbar thumb is drawn. **Three layout gotchas fixed** (host tests
  `scroll_region_scrolls`, `nested_scroll_region_overflows`, `gallery_exact_scrolls`):
  (a) scroll content must not shrink — laid out at natural height (the demo wraps
  the panel in a `flex-shrink:0` content wrapper); (b) taffy's `content_size` is
  unreliable for the nested flex, so content height is computed from laid-out
  child bottoms during paint_walk; (c) **the scroll-region ancestor needs a FIXED
  height** (the gallery root uses `height:100%`, not `flex-grow:1`) — else it
  grows to content and the viewport never caps. Device-verified: at 2.25× a swipe
  over a card background scrolled the panel (Slider card came in, Checkbox
  scrolled off) **while the title + tabs stayed fixed**.
- **Task complete** — all 3 phases + scale + sticky-header scrolling shipped.

## Text-input editing — selection + hide-key (device-verified 2026-05-29)

The phase-3 edit box gained a real selection model + a working hide-key
(resolving the first two known limitations from the initial cut):

- **Caret + selection.** The renderer now treats any `data-input` element as an
  editable field: `paint_input` draws the value text-blob, a selection-highlight
  rect (`0x8042_85F4`, behind the text) for the `[anchor..caret]` range, and a
  caret bar (`0xFF42_85F4`) when the field is focused. A new `measure_w` (via the
  host paragraph interface, cached) maps tap/drag x → caret index in the demo's
  `char_at`. **Tap = caret**, **drag = select** (the field dispatches `mousemove`
  during a press because the renderer focuses `F_KEY` elements on pointer-down and
  routes drag to them), **type = replace selection / insert at caret**, plus
  Backspace (delete selection or char) and ArrowLeft/Right (move/collapse).
  Device-verified: `"edit me"` → tap-start put the caret at index 0 → typed `h` →
  `"hedit me"`; drag selected `[1..7]` (highlight) → typed `z` → `"hze"`.
- **Hide key (⌄) dismisses the keyboard.** The IME's hide key already emits
  Escape (key_id=27) via `send-key-event`; the demo now handles `"Escape"` (and
  `"Enter"`) in `onkeydown` by calling `ime::notify_editor_detached()` +
  collapsing the selection. Device-verified: tapping ⌄ closes the keyboard and the
  field reverts to the "Tap the field to type" hint.

## Input-type variants + keyboard-avoidance + tap-blur (device-verified 2026-05-29)

The single text field became the `EditField` component (per-field value/caret/
anchor + a shared `active: Signal<i32>` so only one field is focused at a time),
and the Text tab now hosts four fields — `text` / `number` / `phone` / `email`:

- **Per-type IME layouts.** Each field passes its `input-type` string to
  `ime::notify_editor_attached`; the host maps it to the `wandr:ime` enum and the
  IME swaps layout. Device-verified: Number → numeric keypad (`. , -`), Phone →
  ITU-T dial pad (`+ * #`), Email → QWERTY with the `@` / `.com` row. Typing
  routes into the field for all of them (`"42"` → `"4256"`).
- **Keyboard-avoidance (renderer).** The IME runs on a bottom overlay surface
  (~`INITIAL_OVERLAY_PX`=1200) and the focused app gets no inset signal, so a low
  field used to sit *behind* the keyboard. The renderer now applies a keyboard
  inset while an input is focused (`kb_inset_px`, default 0.45 × surface height,
  overridable via `set_keyboard_inset`): `max_scroll` gains that much bottom
  padding and, on focus, `ensure_focused_visible` scrolls the field above the
  keyboard line. Device-verified: tapping the Email field (below the keyboard)
  scrolls it up into view.
- **Tap-outside blurs + hides the keyboard.** Focus is reconciled from a DOM
  `focused` attr on `data-input` elements (so detach is authoritative), and
  `on_pointer_down` dispatches `onfocusout` (a new data-less focus event in
  `events.rs`) to the focused field when a tap lands on a non-field (empty space,
  a button, a tab, a slider). The demo's `onfocusout` calls
  `ime::notify_editor_detached()`. Device-verified: tapping the title hides the
  keyboard, unfocuses the field, and the content scrolls back.

Locked by `keyboard_avoidance_scrolls_focused_field_then_blurs` (host test).

## Known limitations / follow-ups (text input)

1. **No text selection of the IME's own composing region / autocorrect** — input
   is direct key insertion; there's no composing-text / suggestion-bar path. Fine
   for the English/dial-pad layouts here.
2. **Overlay surfaces don't rotate with device orientation** — a *general* wandr
   limitation, not dioxus-specific: task 43 rotates only the fullscreen app
   (`enable_rotation` gated to `OverlayMode::None` in `standalone.rs:352`), so in
   landscape the IME keyboard (and status bar / taskbar) stay portrait. Scoped as
   a follow-up (`tasks/62-overlay-orientation.md`).
3. **Single-pointer only — no multi-touch / multi-finger gestures.** The WIT
   boundary is multi-touch-shaped (`on-pointer-event-v2` carries `pointer-id` +
   `pressure`), but the renderer collapses to one pointer (single `captured` /
   `down` / `focused`, dispatches `MouseData`; `convert_pointer_data` /
   `convert_touch_data` are `unimplemented!`), the demo drops `_pid`/`_pressure`
   (`lib.rs:148`), and the shim only emits `ACTION_MOVE` for pointer `idx=0`
   (`cpp/sf_surface.cpp:399`). Single-finger tap/drag/scroll work (hand-rolled in
   the renderer's `Down` state machine); richer single-finger gestures
   (long-press, swipe, double-tap, fling) are a renderer-only addition (timing via
   the `render_frame(nanos)` clock). Multi-finger (pinch/rotate/two-finger pan)
   needs all three layers: shim `0..getPointerCount()` loop on MOVE (a-03
   rebuild), a per-`pointer-id` map in the renderer + `pointer*`/`touch*` dioxus
   events, and the demo passing `_pid` through. Not blocked; just unimplemented
   above the WIT line. Note only — not scoped to a task yet.

## Related

- `tasks/59-dioxus-canvas-renderer.md` (the renderer), `crates/dioxus-canvas/`,
  `apps/user/wandr.dioxus.demo/`.
- `tasks/47-ime-via-guest-app.md`, `tasks/49-ime-content-control.md`,
  `wit/ime.wit` — the IME protocol phase 3 plugs into.
- [[reference_dioxus_taffy_rust_ui]].
