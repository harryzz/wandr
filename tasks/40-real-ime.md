# Task 40 — real IME via rsbinder → IMMS (multi-session arc)

> **Status:** 🟡 in progress — session 2 (vendor + first call) done 2026-05-27.
> Multi-week, multi-session commitment. Standing decision overturned
> 2026-05-27 — user explicitly opted for path (B) over the project
> memory's "default A polish, B on roadmap not next" recommendation
> ([[project-ime-options]]). The trade-off was accepted with eyes
> open: 1-2 weeks of work + ongoing maintenance burden as the price
> of real Gboard / voice / emoji / CJK / autocorrect input.

## Why this is hard

Android's IME protocol is binder-based on the wire, but the wire
surface is **the largest of any system service we'd vendor**:

- IMMS has the highest AIDL-churn rate across Android releases —
  Android 12, 13, 14, 15 each reshape it. Re-vendor per upgrade.
- `IRemoteInputConnection` alone is ~30 calls (the full editor model).
- The framework expects an **Activity-style WindowToken** to track
  focus. Non-Activity clients (standalone wart-host) need to fake
  an IBinder identity that round-trips opaquely through IMMS.
- The IME runs in a separate process with its own SurfaceFlinger
  layer; z-order, ime-on-top, insets handling are all hand-roll-able
  but unsupported edge cases.

Project memory [[project-ime-options]] captured this as "(B) on
roadmap but not next" specifically because of these costs. The user's
2026-05-27 decision to proceed accepts them.

## The five actors

```
wart-app (Compose) ─PlatformContext.startInputMethod──► RootNodeOwner
   │                                                       │
   │                                                       ▼
   │  ┌──────────────── wart-host ─────────────────────────────┐
   │  │  IInputMethodClient (we serve)  ◄── system_server (IMMS)
   │  │  IRemoteInputConnection (we serve)                      ▼
   │  └─────────────IInputMethodManager (we call) ──► IInputMethodSession
   │                                                       │
   │                                                       │ touches in IME slot
   │                                                       ▼
   │                                                  Gboard / IME APK
   │                                                       │
   │ ◄── commitText / sendKeyEvent ◄────────────────────────┘
   │
   ▼
TextFieldState (via EditCommand stream)
```

## Session arc

Sessions are bookmarks; effort estimates are honest minimums, expect
slippage:

### Session 1 (this one) — recon + scope + AIDL inventory + maybe first read-only call

- **Output:** this doc + the next 4 task entries scoped to subsystem.
- Identify exact AIDLs from frameworks/base/core/java/com/android/internal/inputmethod/.
- Pick + start a sparse-checkout submodule layout (frameworks/base is
  enormous; can't vendor whole thing).
- Add codegen to build.rs.
- If time: make first read-only call to IMMS (e.g.
  `getInputMethodList` or `getCurrentInputMethodInfoAsUser`) — proves
  transport.

### Session 2 — client registration + WindowToken fakery

- Implement `IInputMethodClient` as a binder receiver in wart-host.
- Fake an IBinder identity that IMMS will accept as a focus key.
- Call `startInputOrWindowGainedFocus` and observe the
  `InputBindResult` it returns.

### Session 3 — IRemoteInputConnection editor model (the bulk)

- Implement the ~30 editor calls: `commitText`, `setComposingText`,
  `getTextBeforeCursor`, `getTextAfterCursor`, `getSelectedText`,
  `deleteSurroundingText`, `sendKeyEvent`, `performEditorAction`,
  `finishComposingText`, ...
- Wire to Compose's `EditCommand` stream — the `request.editText { }`
  scope from `PlatformTextInputMethodRequest`.

### Session 4 — Compose integration

- Custom `PlatformContext.startInputMethod` impl that:
  - Triggers IMMS show-input on focus
  - Hands the IME a serving `IRemoteInputConnection` over binder
  - Translates the EditCommand stream back from the IME's callbacks
- WindowInsets.ime handling so text fields don't hide behind Gboard.

### Session 5+ — polish + per-version drift testing

- Test on Android 13 / 14 / 15 IME variants if available.
- Handle the per-Android-release AIDL drift (start lock-in to AOSP 15
  on taimen target; document re-vendor process for next Android).

## AIDL surface (session-1 inventory target)

Files in `frameworks/base/core/java/com/android/internal/inputmethod/`:

| File | Role |
|---|---|
| `IInputMethodManager.aidl` | The big one — we CALL these |
| `IInputMethodClient.aidl` | We RECEIVE these from IMMS |
| `IInputMethod.aidl` | What an IME APK implements (we don't) |
| `IInputMethodSession.aidl` | We RECEIVE these |
| `IRemoteInputConnection.aidl` | We SERVE these (editor model — ~30 calls) |
| `IRemoteAccessibilityInputConnection.aidl` | Same but for a11y; may stub for v1 |
| `InputBindResult.aidl` | Parcelable — what startInput returns |
| `InputMethodSubtype.aidl` | Parcelable — locale variants |
| `IInputContentUriToken.aidl` | Parcelable for clipboard-style content sharing — may stub |

Plus from `frameworks/base/core/java/android/view/inputmethod/`:

| File | Role |
|---|---|
| `EditorInfo.aidl` | Parcelable describing the text field |
| `InputMethodInfo.aidl` | Parcelable — IME metadata |
| `InputBinding.aidl` | Parcelable — the IPC binding session |
| `CursorAnchorInfo.aidl` | Parcelable — selection + composing region |
| `CompletionInfo.aidl` | Parcelable — autocomplete suggestions |
| `ExtractedText.aidl` | Parcelable — visible text excerpt |
| `ExtractedTextRequest.aidl` | Parcelable |

Plus transitives from `frameworks/base/core/java/com/android/internal/`:

| File | Role |
|---|---|
| `InputMethodInfoSafeList.aidl` | New in Android 14+ |
| `IRemoteCallback.aidl` | Generic callback |
| `view/InputConnectionCommandHeader.aidl` | New in 14+ |

**Approach:** sparse-checkout `frameworks/base` to just these AIDL
files. Submodule will be `vendor/aosp-frameworks-base` pinned to the
same `android-15.0.0_r36` tag the other AOSP submodules use.

## Knowns / unknowns this task carries

**Knowns:**
- rsbinder pipeline works for HAL-level AIDLs (vibrator, lights,
  power, thermal, sensors, audio).
- frameworks-layer AIDLs work too (sensorservice via task 20).
- The recursive-parcelable workaround pattern is established
  ([[rsbinder-aidl-recursive-limitation]]) — will likely apply to
  `InputBinding` or similar.
- WIT integration point on the Compose side is clear:
  `PlatformContext.startInputMethod` (per [[feedback-ime-options]]).

**Unknowns we'll discover:**
- Whether `startInputOrWindowGainedFocus` accepts a non-Activity
  WindowToken at all. If IMMS rejects with `INVALID_TOKEN`, we may
  need a Java helper APK or alternate trust path. **This is the
  biggest risk.**
- SELinux: `untrusted_app → system_server:input_method_service`
  policy probably denies binder calls from non-Activity clients.
  Will need `setenforce 0` for testing; production needs sepolicy
  update (deferred).
- Whether the IME displays correctly given we're a standalone
  SurfaceFlinger client rather than a windowed Activity.

## Session 1 recon findings (2026-05-27)

Encouraging signs from `dumpsys input_method` + shell probes:

- IMMS DOES track non-Activity clients — `ClientState` entries are
  keyed by `(uid, pid, IBinder client identity)`, NOT by an Activity-
  bound token. The system_server itself appears as `mUid=1000 mPid=1756`.
- The active IME on the dev device is LatinIME (`com.android.inputmethod.latin`)
  — Gboard-equivalent, will render when we trigger show-soft-input.
- `mFocusedWindow=android.os.BinderProxy@9fd35e4` shows that focused
  windows are tracked as opaque IBinder proxies. IMMS doesn't verify
  the IBinder is Activity-owned — it just uses it as a focus key.
- Settings: `settings get secure default_input_method` returns the
  active IME's component name — useful for diagnostics.

Remaining open de-risks (session 2 will hit them):
- Does IMMS query WMS to validate the WindowToken? If yes, our non-
  WMS-registered IBinder will fail validation. Possible workaround:
  register a token with WMS via SurfaceComposerClient first (we
  already have that infrastructure from task 33).
- SELinux: existing HAL work needed `setenforce 0` — IMMS likely the
  same. Production sepolicy is task-43-level deferred.
- `IInputMethodManager.addClient` (or its 2026 name in AOSP 15) is
  the very first call to make — if it returns success, we're past
  the riskiest registration hurdle.

## Session 1 deliverable (this session)

- ✅ This document.
- ✅ Sub-task tracking entries for sessions 2/3/4/5+ (in the in-session
  task list).
- ⏸ Vendor submodule pickup deferred to session 2 — frameworks/base
  is enormous (~200MB+ even sparse) and best added as a focused chunk
  with the first real binder call in the same session.
- ⏸ First read-only IMMS call also deferred to session 2 — the call
  itself needs the AIDL bindings, which needs the vendor work.

Session 1 was honest recon + plan-of-record. Session 2 = "vendor +
first call." See in-session task tracker.

## Session 2 results (2026-05-27)

**Outcome: ✅ device-verified.** Round-trip from rsbinder reaches IMMS
and unmarshals a boolean response cleanly:

```
ime: IMMS round-trip OK — isImeTraceEnabled() = false.
Transport validated against com.android.internal.view.IInputMethodManager.
Session 2 first-call milestone reached.
```

### Deliverables

1. **`vendor/aosp-frameworks-base` submodule** added at
   `wart-host/vendor/aosp-frameworks-base`, pinned to
   `android-15.0.0_r36` (same tag as the other AOSP submodules).
   Sparse-checkout reduces the working tree from ~1.9 GB (full shallow
   clone) to ~17 MB. The sparse-checkout config is per-clone (not in
   the repo) — re-establish on a fresh checkout via:
   ```bash
   cd wart-host/vendor/aosp-frameworks-base
   git sparse-checkout init --cone
   git sparse-checkout set \
     core/java/com/android/internal/inputmethod \
     core/java/com/android/internal/view \
     core/java/android/view/inputmethod \
     core/java/android/os \
     core/java/com/android/internal/os
   ```

2. **Stripped `IInputMethodManager.aidl` stub** in `wart-host/build.rs`
   — matches the task-22 `ISurfaceComposer.aidl` self-healing pattern.
   Drops all 15 transitive AIDL imports (IInputMethodClient, EditorInfo,
   ResultReceiver, ImeTracker, …) by stubbing 25 preceding methods as
   `void slot_NN_<orig-name>()` and keeping `isImeTraceEnabled` at its
   real position (transaction code 26 = FIRST_CALL_TRANSACTION + 25).
   Sessions 3-5 will re-vendor the methods + parcelables incrementally
   as new IMMS calls are needed.

3. **First read-only call:** `isImeTraceEnabled()` chosen over
   `getCurrentInputMethodInfoAsUser` (the plan-doc default) after seeing
   the full IInputMethodManager.aidl — `isImeTraceEnabled` is the only
   inputmethod method that is (a) `@RequiresNoPermission` (no
   `INTERACT_ACROSS_USERS_FULL` gate), (b) takes zero arguments, and
   (c) returns a primitive (`bool`) so no parcelable shape needs to be
   replicated client-side. Result: a clean unambiguous transport
   validation rather than a "parse error proves transport" muddy
   signal.

4. **`wart-host/src/ime_impl.rs`** mirrors `display_impl.rs` (the
   task-22 SurfaceFlinger probe): `rsbinder::hub::get_interface("input_method")`
   → `Strong<dyn IInputMethodManager>` → `isImeTraceEnabled()` → log.
   Wired behind a new `wart-host --probe-ime` CLI flag in `main.rs`
   (one-shot, no display, no wasm component loaded).

### What we learned

- **Transport works.** `rsbinder::hub::get_interface("input_method")`
  resolves correctly to IMMS, `.r#isImeTraceEnabled()` makes a real
  binder transaction, and the boolean response unmarshals to a Rust
  `bool` cleanly.
- **No SELinux relaxation needed for this call.** `setenforce 0` was
  NOT required — `@RequiresNoPermission` methods bypass the
  framework-level permission gate AND fit within the existing
  `untrusted_app → system_server` policy on the dev device. Session 3+
  calls that need `addClient` / `startInputOrWindowGainedFocus` will
  hit `INTERACT_ACROSS_USERS_FULL` and may need `setenforce 0`.
- **The 25-slot stub pattern works without per-method import shims** —
  rsbinder-aidl accepts `void slot_NN()` declarations as no-import
  placeholders. This is cheaper than stubbing 15 transitive
  parcelables upfront.

### Open de-risks carried into session 3 (unchanged from session 1)

- Does IMMS query WMS to validate the WindowToken passed to
  `startInputOrWindowGainedFocus`? Still the biggest unknown.
- Does the `addClient` call from a non-Activity client accept a
  fabricated IBinder as the client identity? Recon suggested yes, but
  must be empirically tested.
- SELinux denial on `INTERACT_ACROSS_USERS_FULL`-gated calls —
  expected. `setenforce 0` for dev, production sepolicy deferred.

## Related

- [[project-ime-options]] — the standing-decision memo (now overturned for path B).
- [[feedback-ime-options]] — the older multi-path summary.
- [[feedback-softkeyboard]] — current in-canvas keyboard state (the path B replaces).
- [[rsbinder-aidl-recursive-limitation]] — known codegen issue.
- [[feedback-rsbinder-nullable-callback]] — earlier rsbinder pattern reference.
- `tasks/15-rsbinder-pipeline.md` — submodule + codegen foundation.
- `tasks/20-sensors-hal.md` — first frameworks-layer AIDL via rsbinder.

## Out of scope (any session)

- Voice input — works via Gboard once IME wiring is in.
- Swipe-typing — same; Gboard's responsibility.
- Custom IME UI — we're the CLIENT, not the IME.
- IME insets visuals during the initial sessions — wire the binder
  path first; insets come last.
