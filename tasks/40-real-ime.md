# Task 40 — real IME via rsbinder → IMMS (multi-session arc)

> **Status:** 🟡 in progress — session 4 (startInputOrWindowGainedFocus) done 2026-05-27.
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

## Session 3 results (2026-05-27)

**Outcome: ✅ device-verified.** IMMS accepted our non-Activity client
registration. Pixel 2 XL `dumpsys input_method` shows our process as
a first-class `ClientState` entry alongside system_server + user apps:

```
ClientState{a425746 mUid=0 mPid=4404 mSelfReportedDisplayId=0}:
  client=com.android.server.inputmethod.IInputMethodClientInvoker@afaad07
  fallbackInputConnection=com.android.internal.inputmethod.IRemoteInputConnection$Stub$Proxy@7ddfe34
  sessionRequested=false
  ...
  pid=4404
```

`pid=4404` is our wart-host probe process (`adb shell ps` correlation).
`mUid=0` because the probe ran via `su`; production wart-host running
as a normal app would show its app uid.

### Deliverables

1. **Stubbed `IInputMethodClient.aidl`** (12 oneway `slot_NN_<orig-name>()`
   methods, no transitive imports) — auto-patched into the vendored
   submodule on every build, same self-healing pattern as the
   IInputMethodManager stub. Bn-side server (`ImeClient`) in
   `wart-host/src/ime_impl.rs` logs each dispatch and returns Ok.
   IMMS may fire `setActive` / `setInteractive` etc. asynchronously
   after registration; oneway means our stubs swallow the calls
   without breaking the protocol.
2. **Stubbed `IRemoteInputConnection.aidl`** (1 oneway placeholder method)
   — full ~36-method editor interface deferred to session 4. addClient
   does NOT call methods on this binder synchronously; IMMS just
   stores the reference for later use when an IME asks for editor
   text.
3. **Un-stubbed `addClient(client, inputConn, displayId)`** in the
   IInputMethodManager AIDL stub. Real signature, transaction code
   FIRST_CALL_TRANSACTION + 0 (matches IMMS's dispatch).
4. **`probe_addclient()`** in `wart-host/src/ime_impl.rs` — wraps
   both server impls in `BnInputMethodClient::new_async_binder` /
   `BnRemoteInputConnection::new_async_binder` (with a tokio
   current-thread runtime adapter copied from `sensors_impl.rs`),
   calls `addClient(&client, &input_conn, 0)`, holds the binders
   alive for 5 s so `dumpsys input_method` shows the entry.
5. **`wart-host --probe-ime-addclient` CLI flag** in `main.rs`.

### What we learned

- **Non-Activity clients can register with IMMS without permission gating.**
  `addClient` has no `@RequiresPermission` annotation in
  `IInputMethodManager.aidl` (unlike most other methods, which gate on
  `INTERACT_ACROSS_USERS_FULL`). Empirically confirmed: addClient
  succeeded with no SELinux relaxation needed (device was already in
  Permissive mode, but the dispatch shows no framework-level identity
  check either).
- **`displayId=0` works** for the primary display. No need for any
  WMS/SF token registration ahead of time.
- **The Bn-side server pattern from `sensors_impl.rs` ports cleanly.**
  `BnFoo::new_async_binder(server, TokioRuntime) -> Strong<dyn Foo>`,
  same recipe.
- **rsbinder-aidl handles `oneway interface` cleanly.** The 12 oneway
  stub methods on IInputMethodClient generate an
  `IInputMethodClientAsyncService` trait with 12 `async fn ... ->
  Result<()>` — same shape as a non-oneway interface from the trait's
  perspective.

### Open de-risks carried into session 4

- **WindowToken validation against WMS** — still the biggest unknown.
  `startInputOrWindowGainedFocus(...)` takes an `IBinder windowToken`
  that IMMS treats as a focus key. The session-1 recon suggested IMMS
  doesn't validate it against WMS, but we haven't called the method
  yet. If WMS validation kicks in, we'd need to register a token via
  SurfaceComposerClient (task 33's infrastructure) before the IMMS
  call.
- **`EditorInfo` and related parcelables** — `startInputOrWindowGainedFocus`
  takes a complex `EditorInfo` describing the text field. Vendoring
  the full hierarchy (EditorInfo, ExtractedTextRequest, …) is the
  bulk of session 4's vendoring work.
- **`InputBindResult` return type** — `startInputOrWindowGainedFocus`
  returns `InputBindResult` containing the IME's
  `IInputMethodSession` binder. Sessions 4-5 need to actually drive
  that session to make the soft keyboard appear.

## Session 4 results (2026-05-27)

**Outcome: ✅ device-verified.** `startInputOrWindowGainedFocus(...)`
landed cleanly at IMMS in both windowToken modes (None and
fabricated-non-null). IMMS returned a null InputBindResult — the
documented response for a client that isn't the focused window and
hasn't passed an EditorInfo.

```
ime: attempt-A windowToken=None  OK — IMMS returned a null InputBindResult ...
ime: attempt-B windowToken=Some  OK — IMMS returned a null InputBindResult ...
```

**Both biggest open unknowns are resolved in our favor:**

1. **WindowToken validation against WMS does NOT block us.** A
   fabricated IBinder (extracted from a fresh `BnRemoteInputConnection`
   we never registered with WMS) is accepted by IMMS. Recon was right
   — IMMS uses the IBinder as an opaque focus key, doesn't cross-check
   with WindowManagerService.
2. **The empty `ImeOnBackInvokedDispatcher` parcelable stub serializes
   acceptably.** The wire bytes (non-null marker + 0 field bytes)
   match what IMMS's readFromParcel tolerates — the Java side
   apparently accepts a zero-field unmarshal as a default
   ImeOnBackInvokedDispatcher.

### Deliverables

1. **Two more stubs** auto-patched into the vendored submodule by
   `wart-host/build.rs`:
   - `IRemoteAccessibilityInputConnection.aidl` — 1 placeholder oneway
     method (we pass null for the corresponding @nullable parameter).
   - The IMM AIDL `import` list grew to pick up `EditorInfo`,
     `InputBindResult`, `ImeOnBackInvokedDispatcher` (all already
     forward-declared `parcelable Foo;` upstream — rsbinder emits
     zero-field Rust structs for them, which is what we need to serve
     as opaque arguments).
2. **Un-stubbed `slot_11_startInputOrWindowGainedFocus`** in the IMM
   AIDL stub. Full 12-arg signature, transaction code
   FIRST_CALL_TRANSACTION + 11.
3. **`probe_startinput()`** in `wart-host/src/ime_impl.rs`:
   - Calls `addClient` first (session-3 pre-req for `startInputOrWindowGainedFocus`).
   - Constructs a windowToken carrier (`BnRemoteInputConnection` with a
     stub server), extracts its IBinder via `as_binder()`. IMMS treats
     this as the focus key.
   - Constructs `ImeOnBackInvokedDispatcher::default()` (empty stub).
   - Runs the call twice — once with windowToken=None, once with
     windowToken=Some(carrier) — to isolate the windowToken question
     from the parcel-marshaling question.
   - Maps rsbinder's `UnexpectedNull` response status to "success —
     IMMS returned null", since `impl_deserialize_for_parcelable`
     returns `UnexpectedNull` whenever the wire data carries the
     null-parcelable flag (which is exactly what IMMS sends for a
     non-bind response).
4. **`wart-host --probe-ime-startinput` CLI flag** in `main.rs`.

### What we learned

- **rsbinder's `Status::transaction_error()` only returns the
  StatusCode for `TransactionFailed` exceptions** — for
  `NullPointer / UnexpectedNull:` (which is what IMMS's null
  InputBindResult deserializes to), you have to consume the Status
  via `Into<StatusCode>` to get the actual code. Documented in our
  probe with a comment for future sessions.
- **Empty-struct forward-declared parcelables work as inputs to IMMS**
  via rsbinder — the non-null marker + 0 field bytes is enough for the
  Java side's readFromParcel default path.
- **rsbinder logs a `dumpsys` warning** — `W dumpsys: Thread Pool max
  thread count is 0. Cannot cache binder as linkToDeath cannot be
  implemented. serviceName: input_method` — when shelling out to
  `dumpsys input_method` during a probe. Cosmetic, no impact on our
  call.

### Open de-risks carried into session 5

- **Actually triggering an IME bind.** Sessions 2-4 prove the
  binder plumbing works end-to-end, but no IME (Gboard, LatinIME)
  has appeared on screen yet. To trigger a real bind, we need:
  (a) a non-null `EditorInfo` describing a text field with proper
  field type / input type / IME options, AND (b) probably a
  focused-window state that matches what IMMS expects (it may
  require us to have called showSoftInput first, or to be the
  current focused-window client per WMS state — which we are not).
- **`EditorInfo` parcel layout.** It's a Java parcelable with ~20
  fields (mInputType, mImeOptions, mPrivateImeOptions, mPackageName,
  mFieldId, mFieldName, mInitialSelStart, mInitialSelEnd,
  mInitialCapsMode, mExtras, mHintText, mLabel, mActionLabel, …).
  Hand-rolling a writeToParcel sequence in Rust is the bulk of
  session 5's vendoring work.
- **The `InputBindResult` payload.** When IMMS DOES return a non-null
  bind result, it contains the IME's `IInputMethodSession` binder and
  surrounding metadata. Session 5 needs to parse this to actually
  drive the IME (showSoftInput, sendKeyEvent etc).
- **`IInputMethodSession` + `IRemoteInputConnection` real methods.**
  Session 5 also un-stubs the actual editor-side methods that the
  IME calls back to read/write text (commitText, getTextBeforeCursor,
  …) — the ~36-method surface deferred from session 3.

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
