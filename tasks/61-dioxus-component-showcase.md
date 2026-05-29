# Task 61 — dioxus-canvas component showcase + renderer capability expansion

> **Status:** ✅ all 3 phases device-verified 2026-05-29. `war.dioxus.demo` is
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
   `war:ime` editor-attached protocol (signal editor focus to the host →
   war.ime.keyboard overlay shows → keys route back via the host socket →
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
| 3 | Text input | edit box (focus, cursor, backspace) + soft keyboard | `convert_keyboard_data`; focus; `on_key_event_v2` routing; `war:ime` editor-attached integration |

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
  wart even when WMS `mCurrentFocus` shows the launcher, so injected pointer
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
  forwards to the arbiter which shows `war.ime.keyboard`) and whose `onkeydown`
  edits the String (Character/Backspace/Enter); a Done button calls
  `notify_editor_detached`. **Full loop device-verified**: tap the dioxus field →
  arbiter logs `attach-editor … → route to war.ime.keyboard delivered=true` →
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

## Known limitations / follow-ups (text input, observed on device 2026-05-29)

The phase-3 edit box is a minimal first cut. Outstanding:

1. **No text selection** in the edit box — there's no caret-drag / long-press
   selection or highlighted range; input is append + backspace at the end only.
   Needs a selection model in the renderer's text-input handling (and richer
   `war:ime` `commit-text`/`set-selection` verbs, currently stubbed — see
   `wit/ime.wit` "step 3b ships send-key-event only").
2. **The IME "hide keyboard" (⌄) key doesn't dismiss the keyboard** for the
   dioxus editor. Only the demo's own **Done** button / **Enter** detaches (via
   `ime::notify_editor_detached`). The keyboard's hide key needs to route a
   detach/hide back to the focused guest (arbiter → editor) — that path isn't
   wired for the dioxus guest yet.
3. **Editor input-type variants untested** — the demo declares only
   `input-type = "text"`. The IME's per-type layouts (numeric / phone / email /
   url / password / multiline, task 49 step 2) are NOT exercised by this guest;
   `notify_editor_attached` would need to pass the right `input-type` per field
   and the demo would need fields of each kind.

## Related

- `tasks/59-dioxus-canvas-renderer.md` (the renderer), `crates/dioxus-canvas/`,
  `apps/user/war.dioxus.demo/`.
- `tasks/47-ime-via-guest-app.md`, `tasks/49-ime-content-control.md`,
  `wit/ime.wit` — the IME protocol phase 3 plugs into.
- [[reference_dioxus_taffy_rust_ui]].
