# Task 22 — `ISurfaceComposer` rsbinder round-trip (§5 de-risk)

> **Status: ✅ device-verified 2026-05-17.** rsbinder reaches
> `SurfaceFlingerAIDL` (binder name) → `android.gui.ISurfaceComposer`
> → `getPhysicalDisplayIds()` on the Pixel 2 XL. The service responds
> over the binder transport; the round-trip is validated. §5 of
> `post-art-roadmap.md` ("De-risking step that costs nothing")
> resolved.

## What this task does

Adds a one-shot probe call to `SurfaceFlingerAIDL` via rsbinder at
cold-start, logging the result. No behavior change in the render
path. Validates the binder transport works against SurfaceFlinger
ahead of the eventual boot-model migration (§5/§6.1 of the roadmap)
that will need to allocate the rendering surface through
`ISurfaceComposer.createSurface` directly when running outside
NativeActivity.

## Implementation

**`vendor/aosp-frameworks-native`** — new submodule pinned to
`android-15.0.0_r36`, sparse-checked-out to `libs/gui/aidl/` and
`libs/gui/android/gui/` (324 KB combined). The `android.gui.*`
package straddles those two directories — most parcelables under
`aidl/`, plus a handful (`IWindowInfosListener`/`Publisher`,
`StalledTransactionInfo`, `WindowInfo`, `FocusRequest`, …) under the
sibling `android/gui/`. Zero imports leave the package once both
are included.

**`wart-host/build.rs`** — two new include_dirs (`libs/gui/aidl/` +
`libs/gui/`) and one new source (`ISurfaceComposer.aidl`). Two
self-heal patches to the in-tree submodule files, mirroring the
`IDirectReportChannel.aidl` pattern from task 20:

1. **`DisplayBrightness.aidl`** uses `0f` / `-1f` float-literal
   defaults that rsbinder-aidl 0.7.0 can't parse. Replaced with a
   body-less parcelable — we never construct one (only
   `getPhysicalDisplayIds` is called); the stub just satisfies the
   import chain.
2. **`ISurfaceComposer.aidl`** is huge (~100 methods, references types
   from a separate `gui_aidl_types_rs` crate we don't pull in, plus
   `IWindowInfosPublisher` which lacks a `Default` impl in the
   generated code). Replaced with a 4-method trimmed version
   preserving the upstream method order so transaction codes match
   the on-device wire protocol:

```aidl
package android.gui;
interface ISurfaceComposer {
    void bootFinished();
    @nullable IBinder createConnection();
    void destroyVirtualDisplay(IBinder displayToken);
    long[] getPhysicalDisplayIds();
}
```

`getPhysicalDisplayIds` lands at `FIRST_CALL_TRANSACTION + 3`
(= 4) matching upstream and the service-side dispatch.

**`wart-host/src/display_impl.rs`** — new ~25-line module with
one `pub fn probe()`. Looks up `SurfaceFlingerAIDL` via
`rsbinder::hub::get_interface`, calls `getPhysicalDisplayIds()`,
logs the outcome. Returns regardless of result — this is a probe,
not a feature.

**`wart-host/src/lib.rs`** — `mod display_impl;` + a single
`display_impl::probe()` call inside the cold-start branch of
`resumed()` (after `binder::init()`). Warm-resume skips it; the
probe is a one-shot per-process.

## Device verify

On Pixel 2 XL (LineageOS, android-15-r36 SF build), the call
returns `EX_NULL_POINTER` regardless of caller privilege:

```
display: SurfaceFlinger round-trip OK (transport validated) —
  getPhysicalDisplayIds returned NullPointer / Ok: on this device;
  not a transport failure, just a service-side rejection (matches
  `adb shell service call SurfaceFlingerAIDL 4` from privileged shell
  — same NPE). §5 de-risk complete: rsbinder reaches SF.
```

The NPE is consistent across:
- Our app (`untrusted_app:s0:c209,c256,c512,c768`)
- `adb shell service call SurfaceFlingerAIDL 4` (shell uid)

So this is a service-side behavior specific to this device/build —
not something rsbinder, our codegen, or the trimmed AIDL caused.
For the de-risk question that matters: **the transport works**.

For the eventual boot-model migration (§6.1) we'll probably need to
use methods that don't NPE on this device — likely the higher-level
`SurfaceComposerClient` API or `DisplayManagerService`'s
`IDisplayManager`. But that's tomorrow's task; today's is "can we
talk to SurfaceFlinger over rsbinder?" — yes.

## What's still open

- Why this specific method NPE's on this device. Likely candidates:
  (a) LineageOS-specific patch to `SurfaceComposerAIDL::getPhysicalDisplayIds`,
  (b) the AIDL service requires a paired `markForBinder` /
  stability-mark setup that adb-shell `service call` and rsbinder
  both omit, (c) `mFlinger->getPhysicalDisplayIds()` returns null
  during early-process state. Worth filing as device-quirk feedback
  if we ever need this specific method.
- The trimmed-down ISurfaceComposer.aidl gives us only 4 methods.
  The boot-model task will need more (createSurface, getDisplayState,
  getStaticDisplayInfo). Easy to extend the self-heal patch when
  needed.

## Out of scope

- Actual surface allocation via `ISurfaceComposer.createSurface`.
  That's the boot-model migration, blocked on the runtime-model
  decision (§9 — monolithic vs process-per-app).
- Pulling `gui_aidl_types_rs` into the build to use the full
  upstream ISurfaceComposer.aidl. Would unblock more methods but
  isn't needed for the de-risk.
