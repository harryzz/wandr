---
name: project-standalone-keys
description: "Hardware-key events into standalone (no-NativeActivity) wandr-host — RESOLVED 2026-05-26, device-verified"
metadata: 
  node_type: memory
  type: project
  originSessionId: be47cfff-188f-4f12-989d-c09046736d6a
---

Hardware-key delivery into `wandr-host --standalone` — **RESOLVED 2026-05-26**,
device-verified: `adb shell input keyevent KEYCODE_{A,B,C}` types `abc` into
the focused BasicTextField; `KEYCODE_DEL` backspaces; `KEYCODE_DPAD_LEFT/…`
moves the cursor. Touch already worked from task 33 Step 3.

**Two pieces of wiring landed:**

1. **Event plumbing.** `cpp/sf_surface.{cpp,h}` `SfInputEvent` gained a
   `meta_state: int32_t` field at the end; `sf_input_poll` extended to
   handle `InputEventType::KEY` and emit `kind=10` (down) / `11` (up)
   with the AKEYCODE in `key_code` and AMETA bitmask in `meta_state`.
   `src/sf_surface.rs` mirrors the struct + bumped layout. Render loop
   in `src/standalone.rs` got a new `kind == 10 || 11` arm calling
   `input::dispatch_android_key`. New `input::dispatch_android_key` +
   `map_android_keycode` translate Android `AKEYCODE_*` + shift to
   the same (code-point, key-id) tuple the winit NativeActivity path
   sends to Compose — so the guest sees identical numeric IDs across
   both modes. Covers letters (29..54), digits (7..16), space (62),
   editing keys (DEL/ENTER/TAB/ESC/arrows/page/home/end/insert/forward-del),
   and common punctuation.

2. **Focus retention.** Standalone has no `Activity`, so any
   activity-backed window AMS resumes (`com.android.launcher3`,
   Messaging, the last app) immediately steals InputDispatcher focus
   even though wandr owns the z-top SurfaceFlinger layer. Fix: new
   `sf_request_focus()` shim export that re-applies
   `setFocusedWindow(wandr)`; `standalone.rs` calls it every 60 frames
   (~once/second). With this, `dumpsys input | grep FocusedWindows`
   stays pinned on `name='wandr'` for the session.

**Out of scope (deferred):**
- KeyCharacterMap for non-ASCII / dead keys / IME — the host-side
  `map_android_keycode` covers the common subset that on-device dev
  testing exercises.
- Persistent focus via window-config flag (e.g. STEAL_FOCUS) — the
  periodic re-request is good enough for dev and matches the
  "monolithic, single app" model; init.rc/sepolicy work later may
  obviate it.
- AMETA_CTRL/ALT mappings — code-point already accounts for shift;
  Ctrl-key combos not yet wired (Compose's keyboard shortcuts on
  wasi mostly happen via key-id anyway).

Related: [[project-standalone-input]] (touch, that this extends),
[[feedback-basictextfield-freeze]] (the `on-key-event-v2` WIT call
this drives), [[project-standalone-orientation]].
