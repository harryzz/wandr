# Task 42 — system clipboard via rsbinder to IClipboard

> **Status:** 🔲 scoped 2026-05-26, not started. Scoped during the
> theme + clipboard arc — theme shipped same session; clipboard
> deferred because the work is substantively bigger than expected
> (~4-6 hours vs theme's ~1.5h).

## Why

`wart-host/src/clipboard_impl.rs` currently maintains
`HostState::clipboard: Option<String>` — intra-app only. Copying in
wart-app works (TextField selections survive across cards within the
process), but pasting from another Android app (browser, messenger,
etc.) returns whatever wart-app last set, not the actual system
clipboard. Same direction the other way.

Real sync to the system clipboard requires talking to Android's
`ClipboardManager` service. In standalone mode (no NativeActivity,
no JVM), JNI is unavailable; the only path is **rsbinder to the
`clipboard` system service** (IClipboard AIDL).

## What needs to land

1. **AIDL surface** — `android.content.IClipboard` from
   `frameworks/base/core/java/android/content/IClipboard.aidl`. Plus
   transitive parcelables:
   - `android.content.ClipData` (text / uri / intent variants)
   - `android.content.ClipDescription`
   - `android.content.AttributionSourceState` — **recursive parcelable** —
     see [[rsbinder-aidl-recursive-limitation]]. Will hit the same
     workaround as task 15: hand-write the parcel encoder or stub the
     `next` field as `int[]`.
2. **Submodule update** — pull these into the existing AOSP AIDL
   submodule alongside vibrator/sensors/power/etc. (task 15
   foundation).
3. **rsbinder-aidl codegen** — `clipboard.rsbinder` build step. May
   need exclusion list for the recursive parcelables (manual encoding).
4. **wart-host/src/clipboard_impl.rs** — drop the HashMap; replace
   getText/setText with binder calls to `IClipboard::getPrimaryClip()`
   / `setPrimaryClip()`. ClipData ↔ String conversion (extract first
   text item; build ClipData from text on set).
5. **SELinux** — `untrusted_app → app:fd:use` and equivalent for the
   clipboard service may be denied. May need `setenforce 0` for testing
   like other HALs. Production needs sepolicy update (deferred).
6. **hasText / clear** — `IClipboard::hasPrimaryClip()` + `IClipboard::clearPrimaryClip()`.

## Effort estimate

~4-6 hours, dominated by:
- AIDL submodule wrangling + parcelable codegen (1-2h, can be tricky if recursive types bite).
- ClipData parcelable handling (1-2h — multiple variants, attribution).
- On-device debugging (SELinux denials, transaction code mismatches).

Pattern is established (tasks 15/16/17/19/20/21 all used rsbinder
successfully); just bigger surface than e.g. vibrator.

## Why deferred

- In-app clipboard already works (Material3 TextField selection survives across cards within wart-app).
- No demo today *needs* cross-app paste.
- Theme + IME are both higher per-hour visibility wins. IME especially.
- ~half-session of work; better tackled when there's a concrete user-facing trigger ("I want to paste from Chrome").

## Related

- [[rsbinder-aidl-recursive-limitation]] — known blocker for
  AttributionSourceState; hand-encode workaround needed.
- [[feedback_rsbinder_nullable_callback]] — earlier rsbinder pattern
  reference.
- `tasks/15-rsbinder-pipeline.md` — submodule + codegen foundation.
- `wart-host/src/clipboard_impl.rs` — the stub being replaced.
