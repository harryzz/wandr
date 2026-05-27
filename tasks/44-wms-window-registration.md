# Task 44 — WMS window registration for non-Activity processes (path A)

> **Status:** 🔲 scoped 2026-05-27, not started. Spun out of task 40
> session 6's WMS-gate finding. Prerequisite for task 40 to make
> Gboard actually appear; useful on its own for any future
> non-Activity feature that needs to be a first-class window
> (proper focus, status-bar interactions, system-gesture handling,
> activity-manager visibility, etc.).

## Context — why this task exists

Task 40 sessions 2-5 fully wired up the IME binder protocol
(addClient + startInputOrWindowGainedFocus + showSoftInput via
rsbinder → IMMS). Session 5 device-verified the full pipeline:
all three IMMS calls land cleanly, parcels marshal correctly,
client identity is recognized — but **Gboard does not pop up**.
Session 6 confirmed empirically that the blocker is the WMS focus
gate:

```
W InputMethodManagerService: Ignoring showSoftInput of uid 0 : ...
I ImeTracker: null: onFailed at PHASE_SERVER_CLIENT_FOCUSED
```

IMMS gates `showSoftInput` on `mCurFocusedWindowClient == callingClient`.
That field is set by WMS pushing focus updates to IMMS (via
JVM-internal calls, not AIDL). WMS only knows about windows that
came through its `addWindow` flow. Standalone wart-host (task 33)
bypasses WMS — it talks directly to SurfaceFlinger
(`SurfaceComposerClient::createSurface`) and InputFlinger
(`IInputFlinger.createInputChannel` + `gui::WindowInfoHandle` +
`setInputWindowInfo`). Result: we appear in `dumpsys input` as
`Window: wart - wart` and even get InputDispatcher focus, but
remain invisible to WMS → invisible to IMMS's focus check.

**No rsbinder-only or InputFlinger-only path bypasses this**;
SHOW_FORCED doesn't help, spoofing the calling client doesn't help,
the gate is structural in IMMS. The only real fix is to register
a window with WMS through the proper `IWindowManager.openSession`
→ `IWindowSession.addToDisplay` flow.

## Goal

Stand up a WMS-registered window from a non-Activity process
(wart-host running standalone, root via `su`). Specifically:

1. Open a session with WMS via `IWindowManager.openSession`.
2. Serve an `IWindow` Bn-side binder (the per-window callback
   interface; our IWindow's IBinder identity IS the windowToken
   that other system services will track).
3. Add our window to display 0 via
   `IWindowSession.addToDisplay(IWindow, WindowManager.LayoutParams, ...)`.
4. Get focus — either automatically (high z-order + focusable flag)
   or via an explicit setFocusedApp / setFocusedWindow call.
5. Verify in `dumpsys window` and `dumpsys input_method` that
   `mFocusedWindowClient` is now our process.

**Out of scope for task 44**: the actual IME work (task 40 owns
that). Once task 44 ships, task 40 resumes with session 7:
"wire WMS windowToken into showSoftInput probe + summon Gboard".

## Prior art and dependencies

- **Task 40** sessions 2-5 (committed): IMMS binder protocol
  end-to-end. The probe pattern (`probe_addclient`,
  `probe_startinput`, `probe_showsoftinput`) carries directly into
  task 44 — once we have a WMS windowToken, session 5's
  `probe_showsoftinput` should just work.
- **Task 33** (committed): standalone surface bring-up. Already
  uses SurfaceFlinger + InputFlinger directly. Task 44 adds a WMS
  layer ON TOP — likely both registrations need to coexist (WMS
  for focus, InputFlinger for the actual input channel routing).
- **rsbinder 0.8.0** (committed 2026-05-27): pre-Path-A
  maintenance upgrade. FD-UB / deadlock / UAF / obituary fixes
  matter more with the larger AIDL surface this task introduces.
- **Vendored submodule** `wart-host/vendor/aosp-frameworks-base`
  (committed in task 40 session 2): pinned to `android-15.0.0_r36`.
  All AIDL files mentioned below are already in the sparse-checkout
  (verified 2026-05-27 — `IWindowManager.aidl`, `IWindow.aidl`,
  `IWindowSession.aidl`, `IWindowSessionCallback.aidl`).
- **Self-healing build.rs stub pattern**: established by tasks
  22 (`ISurfaceComposer`), 40 sessions 2-5 (IMM + the inputmethod
  callback interfaces). Same pattern applies here for the WMS AIDLs.
- **`MEMORY.md` references**:
  - `feedback_rsbinder_aidl_recursive` — recursive parcelable
    limitation in rsbinder-aidl. WMS parcelables likely hit this
    (Configuration, LayoutParams may reference themselves).
  - `feedback_rsbinder_nullable_callback` — @nullable @nullable
    workaround pattern.
  - `project_standalone_input` — task 33's input registration; we
    interact with the same layer.

## The AIDL surface (preliminary inventory)

Counted in `android-15.0.0_r36`:

| File | Methods | Notes |
|---|---|---|
| `android/view/IWindowManager.aidl` | 146 | The headline interface. We need exactly ONE: `openSession`. Stub the other 145 as `void slot_NN_<name>()` no-import placeholders, same as task 40 IMM. |
| `android/view/IWindowSession.aidl` | 16 | We need `addToDisplay` + `remove`; maybe `relayout`. Stub the rest. |
| `android/view/IWindow.aidl` | ?? | We SERVE this. Bn-side server. WMS calls back on it (resize events, focus events, etc.). Stub all methods as logging no-ops initially. |
| `android/view/IWindowSessionCallback.aidl` | ?? | We SERVE this. Passed to `openSession`. |

### Transitive parcelables (heavy)

These are the big ones — Java parcelables with non-trivial
on-wire formats. Order roughly by "how much we have to actually
implement":

| Parcelable | Where | Difficulty | Notes |
|---|---|---|---|
| `WindowManager.LayoutParams` | `android/view/WindowManager.aidl` | 🔴 Critical, multi-day | ~30 fields (type, flags, format, gravity, x, y, width, height, softInputMode, …). MUST construct a real one for addToDisplay. Cannot be empty-stubbed. |
| `IBinder windowContextToken` | many places | 🟢 Easy | Either a SIBinder fabricated from a fresh Bn, or null where @nullable. |
| `InputChannel` | `android/view/InputChannel.aidl` | 🟡 C++-backed parcelable | The connection token we got from InputFlinger in task 33. May need to round-trip through rsbinder somehow — investigate. |
| `InsetsState` | `android/view/InsetsState.aidl` | 🟡 Returned by addToDisplay (output param) | Likely safe to ignore on output if we only care about whether the call succeeded. |
| `InsetsSourceControl` | array, output | 🟡 Output param | Same as InsetsState — safe to ignore. |
| `MergedConfiguration` | output param | 🟡 Output param | Wraps two Configuration objects; ignore on output. |
| `ClientWindowFrames` | output param | 🟡 Output param | Frame info; ignore on output. |
| `InputTransferToken` | `android/window/InputTransferToken.aidl` | 🟢 Probably forward-declared parcelable | Stub. |
| `Rect`, `Region`, `Point` | `android/graphics/` | 🟡 Simple structs | Real shape needed if non-default values matter. Construct empty defaults first. |
| `Configuration` | `android/content/res/Configuration.aidl` | 🔴 Heavy if needed | Forward-declared parcelable upstream; if we don't construct one, fine. |
| `IApplicationThread`, `IAssistDataReceiver`, etc. | imports of IWindowManager | 🟢 Stub | We don't call methods that use these; stub the interfaces as 1-placeholder oneway interfaces (same pattern as session 3's IRemoteInputConnection). |

### What we serve (Bn-side servers)

- **`IWindow`** — WMS calls back when display/insets/focus changes.
  Stub-all-methods server is fine for session 7-8; sessions 9+ may
  need to handle resized() / dispatchAppVisibility() / etc. to keep
  WMS happy.
- **`IWindowSessionCallback`** — passed to `openSession`. Likely
  just a few methods.

## Multi-session arc (estimated)

Honest minimums, expect slippage on LayoutParams and any first
parcel-shape mismatches:

### Session 7 — vendor + first read-only call (~4-6h)

- Add stubs for `IWindowManager.aidl` (145 of 146 slot_NN
  placeholders + `openSession`), `IWindowSession.aidl`
  (most slot_NN + `addToDisplay` + `remove`), `IWindow.aidl`
  (all slot_NN logging no-ops), `IWindowSessionCallback.aidl`
  (placeholder).
- Stub all transitive `parcelable Foo;` for the IMM-import set.
- Hand `WindowManager.LayoutParams` as a forward-declared
  parcelable initially — we'll un-stub it in session 9.
- New `probe_wms_opensession()`:
  - Get `IWindowManager` via `rsbinder::hub::get_interface("window")`.
  - Stand up `IWindowSessionCallback` Bn server.
  - Call `openSession(callback)`.
  - Log the returned `IWindowSession` (just `Strong<dyn IWindowSession>`
    presence is enough for session 7).
- Device-verify: `wart-host --probe-wms-opensession` returns OK.

Success criterion: a real `IWindowSession` binder arrives back from
WMS. (Even if we have no permission for `addToDisplay`, openSession
typically doesn't gate.)

### Session 8 — add Bn IWindow + try addToDisplay with empty LayoutParams (~4-8h)

- Stand up Bn server for `IWindow` (stub-all-methods).
- Call `IWindowSession.addToDisplay(window, LayoutParams::default(), ...)`.
- Empty LayoutParams: expect WMS to reject with
  `EX_ILLEGAL_ARGUMENT` or similar — but a clean rejection proves
  the call landed.
- If WMS NPEs on our empty LayoutParams (likely), narrow to which
  fields it needs.

Success criterion: a structured error (not a transport failure)
from WMS, identifying which LayoutParams fields are required.

### Session 9 — real LayoutParams (the multi-day one, ~1-2 days)

This is the hardest session. WindowManager.LayoutParams is a
JavaParcelable with ~30 fields. Approach:

- Read the upstream `WindowManager.java`'s
  `LayoutParams.writeToParcel(Parcel, int)` to find the exact field
  order and types.
- Hand-roll a Rust `WmLayoutParams` struct mirroring those fields.
- Provide `impl Parcelable for WmLayoutParams` with the
  byte-for-byte same writeToParcel sequence (int writes, CharSequence
  writes via `Parcel.writeString8`, IBinder writes, Bundle writes).
- Construct a "real enough" LayoutParams: `type = TYPE_APPLICATION_OVERLAY`
  (2038) or similar non-privileged window type, `width = MATCH_PARENT`,
  `height = MATCH_PARENT`, `flags = FLAG_NOT_TOUCH_MODAL | FLAG_LAYOUT_NO_LIMITS`,
  `format = PixelFormat.TRANSLUCENT`. Get something WMS will accept
  for a root-running probe process.
- Retry addToDisplay. Goal: WMS accepts the window.

Success criterion: addToDisplay returns success (0 = no error per
ADD_OKAY const). Window appears in `dumpsys window`.

### Session 10 — focus the window (~2-4h)

Once addToDisplay succeeds, the window exists but may not have
focus. Investigate:
- Does WMS auto-focus the top window automatically? Test by checking
  `dumpsys window | grep mFocusedWindow` after addToDisplay.
- If not: call `IWindowManager.setFocusedApp(IBinder, boolean)` or
  `IWindowManager.requestFocus(...)` — un-stub the slot for
  whichever method WMS needs.
- Alternative: set LayoutParams flags `FLAG_KEEP_SCREEN_ON | not FLAG_NOT_FOCUSABLE`
  to make our window auto-focusable.

Success criterion: `dumpsys input_method | grep mFocusedWindowClient`
shows OUR process (uid=0 pid=<our>) instead of the launcher.

### Session 11 — verify Gboard appears (~1-2h)

- Combine WMS registration (sessions 7-10) with task 40 session 5's
  `probe_showsoftinput`.
- In the same probe process: openSession → addToDisplay → focus →
  addClient → startInputOrWindowGainedFocus → showSoftInput.
- Watch the device screen.

Success criterion: Gboard pops up. (`dumpsys input_method` shows
`mImeWindowVis != 0`, `mInputShown == true`.)

### Session 12+ — productionize (~unscoped)

- Wire WMS registration into the standalone path so task 33's
  standalone wart-host is always a WMS window when running. Hand
  the windowToken to the Compose-side text input integration.
- Real `EditorInfo` parceling (task 40 carryover, ~30 fields, same
  shape as LayoutParams session 9).
- Real `IRemoteInputConnection` editor methods (~36, task 40
  carryover) so Gboard's commitText / sendKeyEvent / etc. actually
  drive the focused text field.
- Permissions hardening: drop root, run as a normal app with the
  right manifest declarations. (Probably needs SYSTEM_ALERT_WINDOW
  for TYPE_APPLICATION_OVERLAY usage outside system_uid.)

## Known risks and unknowns

- **`addToDisplay` permission check.** WMS may require
  `SYSTEM_ALERT_WINDOW` or `INTERNAL_SYSTEM_WINDOW` for non-Activity
  callers. Root-via-`su` should bypass `untrusted_app` SELinux
  restrictions and the framework-level checks based on `Binder.getCallingUid()`,
  but not certain. If it fails: try TYPE_APPLICATION_PANEL or other
  non-privileged window types first.
- **`WindowManager.LayoutParams` parcel layout drift.** Has changed
  multiple times across Android releases. Pin to `android-15.0.0_r36`
  (matches our submodule). Re-vendoring per Android release is a
  carrying cost.
- **`InputChannel` round-tripping through rsbinder.** Task 33 already
  has an InputChannel from InputFlinger; addToDisplay may want one
  too OR may issue its own. Investigate session 8.
- **`Configuration` and other big-imports.** IWindowManager imports
  ~30 types; many are returned in output params we don't care about.
  Stub all of them as `parcelable Foo;` forward declarations to
  start — see how far we get before we need real shapes.
- **Recursive parcelables.** `AttributionSourceState` recursive
  issue (per `feedback_rsbinder_aidl_recursive`) may recur here —
  some WMS parcelables may have self-referential fields. Apply the
  same workaround (empty stub or hand-encoded body).
- **`IWindow.aidl` callback frequency.** WMS calls IWindow methods
  on display config changes, insets updates, etc. Our stub-all-Ok
  server may be enough, but if WMS expects async responses to come
  back via the IWindow methods we serve, we'd need real impls.

## File-touch map (anticipated)

- `wart-host/Cargo.toml` — no new deps (rsbinder 0.8.0 already in)
- `wart-host/build.rs` — add WMS AIDL stubs in the existing
  self-healing pattern; extend the `rsbinder_aidl::Builder` chain
- `wart-host/src/wms_impl.rs` (new) — modeled on `ime_impl.rs`.
  Sessions 7-11 add `probe_wms_*` functions incrementally.
- `wart-host/src/main.rs` — new `--probe-wms-*` CLI flags
- `wart-host/src/lib.rs` — `pub mod wms_impl;`
- `wart-host/src/standalone.rs` — session 12+, wire WMS registration
  into the standalone bring-up sequence
- `tasks/44-wms-window-registration.md` — this doc; update with
  per-session results
- `tasks/40-real-ime.md` — when task 44 lands, mark task 40 ready
  for session 7 resumption
- `CLAUDE.md` — add a status table row when session 7 starts

## Resume hints for fresh sessions

If picking this up cold:

1. `cat .task-state` — if it says `TASK=44 STEP=...`, resume from
   that step.
2. Read **task 40 session 5 results** in `tasks/40-real-ime.md` to
   understand the WMS gate failure mode (it's the precise reason
   this task exists).
3. Read **task 33 step 1** + `cpp/sf_surface.cpp` for prior art on
   non-Activity input window registration via the InputFlinger
   path (which task 44 sits on top of).
4. Read **the existing `wart-host/src/ime_impl.rs`** for the
   Bn-side server + probe pattern that this task replicates for the
   WMS surface.
5. Read **task 40 session 3** for how slot_NN stubbing works on a
   146-method interface.
6. The vendored frameworks-base submodule is already pinned to
   `android-15.0.0_r36`. Sparse-checkout includes `core/java/android/view/`
   — all needed AIDLs are present (verified 2026-05-27).

## Recommendation on when to start

- **Not urgent.** The in-canvas Compose keyboard (per
  `feedback_softkeyboard`) works for current user needs (English
  typing, autocorrect-free).
- **Start when one of these becomes true:**
  - The standalone-mode wart-host (task 33) becomes the primary
    deployment target (vs the NativeActivity-wrapped APK).
  - The in-canvas keyboard's limitations (no voice input, no emoji
    picker, English-only practical) become real user pain.
  - We need WMS-tracked window state for something unrelated to
    IME (e.g., system-gesture interactions, status-bar dimming,
    proper z-order management).
- **Estimated total effort:** 2-3 weeks of focused work spread
  across 5-7 sessions, dominated by session 9 (LayoutParams parcel
  layout) and any per-Android-version drift hunting.

## Related

- `tasks/40-real-ime.md` — the task that motivates this one;
  resume there with session 7 once task 44 ships.
- `tasks/33-boot-model-bringup.md` — standalone surface +
  InputFlinger registration; task 44 layers WMS on top.
- `post-art-roadmap.md` §11 — the boot-model migration this all
  feeds.
- `feedback_rsbinder_aidl_recursive` — workaround for recursive
  parcelables likely needed here.
- `feedback_rsbinder_nullable_callback` — @nullable parcelable
  workaround pattern.
- `project_standalone_input` — InputFlinger-side registration in
  task 33.
