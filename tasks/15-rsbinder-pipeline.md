# Task 15 — rsbinder Pipeline (Infrastructure)

> **Status: ✅ implementation complete (built 2026-05-16), device-verification pending.** Pipeline enables rsbinder to call Android stable AIDL HAL services from the host. **No behavior change** — nothing yet calls the binder pipeline. `cargo build --target aarch64-linux-android --release` succeeds. Tasks 16 (vibrator) and 17 (lights) consume this pipeline.

## Goal

Make `rsbinder` callable from `wart-host` so subsequent tasks can call stable AIDL HAL services (`android.hardware.vibrator.IVibrator`, `android.hardware.light.ILights`) over binder instead of poking sysfs. After this task lands, the build still works exactly as before; nothing yet uses binder. The verification is **negative**: existing PoC features still work; binder init succeeds (or fails gracefully) without panicking the host.

Reference: `~/wart/post-art-roadmap.md` §3 (Boundary B / Pattern 5).

---

## Architecture

```
Rust host (HostState)
  └─ binder::init()  ← One-shot guarded init at App::resumed()
       ├─ checks /dev/binder exists
       ├─ catch_unwind around rsbinder::ProcessState::init_default()
       └─ rsbinder::ProcessState::start_thread_pool()

Build-time codegen (build.rs)
  └─ rsbinder_aidl::Builder
       ├─ source(vendor/aosp-hardware-interfaces/vibrator/aidl)
       ├─ source(vendor/aosp-hardware-interfaces/light/aidl)
       ├─ include_dir(...) for cross-package imports
       └─ output($OUT_DIR/aosp_hal_bindings.rs)

src/binder_aidl.rs
  └─ mod generated { include!($OUT_DIR/aosp_hal_bindings.rs) }
  └─ pub use generated::*
```

Consumers in tasks 16/17 reach binder clients via:
- `crate::binder_aidl::android::hardware::vibrator::IVibrator::IVibrator` (trait)
- `crate::binder_aidl::android::hardware::vibrator::Effect` (enum)
- `crate::binder_aidl::android::hardware::light::ILights::ILights` (trait)
- `crate::binder_aidl::android::hardware::light::HwLightState` (parcelable)

Service lookup uses `rsbinder::hub::get_interface::<dyn IVibrator>("android.hardware.vibrator.IVibrator/default")` — the AOSP convention for the default HAL instance.

---

## Steps

### 1. Vendor AOSP hardware-interfaces submodule

```bash
cd ~/wart/wart-host
git submodule add --depth 1 https://android.googlesource.com/platform/hardware/interfaces vendor/aosp-hardware-interfaces
cd vendor/aosp-hardware-interfaces
git fetch --depth 1 origin refs/tags/android-11.0.0_r48:refs/tags/android-11.0.0_r48
git checkout -f android-11.0.0_r48
git sparse-checkout init --cone
git sparse-checkout set vibrator/aidl light/aidl
```

Mark `shallow = true` in `.gitmodules` to mirror the existing `skia-src` precedent.

Pinned commit: `e7cb492bb835010b3d35496676200250b3b4697e`. Working tree size: ~544 KB.

**Verify:** `ls vendor/aosp-hardware-interfaces/vibrator/aidl/android/hardware/vibrator/IVibrator.aidl` exists; same for `light/aidl/android/hardware/light/ILights.aidl`.

### 2. SDK bump 29 → 30 (atomic, one commit)

Per the bitter lesson in `CLAUDE.md`: `min_sdk_version` and `.cargo/config.toml` linker version must match exactly or cargo cache invalidates and triggers a full rebuild.

- `Cargo.toml`: `min_sdk_version = 30`
- `.cargo/config.toml`: linker → `aarch64-linux-android30-clang`
- `Cargo.toml`: add `[[package.metadata.android.uses_permission]] name = "android.permission.VIBRATE"`

**Verify:** `cargo apk build --release` succeeds (one ~10 min rebuild); `aapt dump permissions <apk> | grep VIBRATE` shows the new permission. Existing PoC features unchanged.

### 3. Add rsbinder dependencies

In `Cargo.toml`:

- `[target.'cfg(target_os = "android")'.dependencies]`:
  - `rsbinder = { version = "=0.7.0", features = ["android_11_plus"] }` — **was `["android_11"]`**; bumped during task-16 device verification because rsbinder's `hub::get_interface` panics with `default: Unsupported Android SDK version: 35` on a Pixel 2 XL running Android 15. SDK 35 is matched against the `android_14` feature inside rsbinder (rsbinder groups SDK 34 + 35 → android_14 feature), and `android_11_plus` includes `android_11..android_14, android_16` so the runtime version dispatch finds a valid arm.
  - `async-trait = "0.1"` — required by rsbinder-aidl's generated code (`#[::async_trait::async_trait]` is emitted unconditionally on the AsyncService traits)
  - **Note:** default features (`tokio`) are kept. An earlier attempt with `default-features = false, features = ["android_11", "async"]` produced `BoxFuture not found` errors from rsbinder's own internal service-manager codegen. The `tokio` feature is needed even though we only make sync HAL calls — the async trait machinery is hard-wired into rsbinder's macros at v0.7.0.
- `[build-dependencies]`:
  - `rsbinder-aidl = "=0.7.0"`

Both versions pinned exactly because generated bindings are not source-stable across minor releases.

**Verify:** `cargo build --target aarch64-linux-android --release` succeeds. Output: `libwasm_android_host.so` ~50 MB, 6 benign warnings (5 pre-existing dead-code, 1 `unused import generated::*` until task 16 uses it).

### 4. ProcessState init helper

New file `wart-host/src/binder.rs`. Single-shot `pub fn init() -> Result<(), &'static str>` using `std::sync::OnceLock`:
- Checks `/dev/binder` exists first (avoids panic in `init_default()` when device has no binder).
- Wraps `init_default()` in `catch_unwind` because rsbinder's API panics on internal failure.
- Calls `start_thread_pool()` after init succeeds.

Wired at the very top of `App::resumed()` in `lib.rs`:

```rust
if let Err(reason) = binder::init() {
    log::warn!("binder init: {reason} — HAL calls will fall back to sysfs");
}
```

Non-fatal: missing `/dev/binder` lets sysfs fallback still work on desktop / weird device states.

**Verify:** App boots without `binder init failed` warning on a real Android 11+ device.

### 5. AIDL codegen + binder_aidl.rs wrapper

In `build.rs`, inside the existing `if target_os == "android"` link block (after the sysroot `rustc-link-lib` lines):

- Add `println!("cargo:rustc-link-lib=binder_ndk");` for runtime symbol resolution.
- Invoke `rsbinder_aidl::Builder::new()` with `.source(<path-to-IFoo.aidl>)` per **interface** (not per directory — passing a dir causes one full package-module emission per AIDL file in the dir, producing duplicate `pub trait`/`pub struct` definitions that won't compile). Use `.include_dir(<aidl-root>)` so the interface .aidl's `import android.hardware.foo.Bar;` resolves the supporting parcelables/enums.
- `.set_async_support(true)` — required. `set_async_support(false)` emits incomplete `declare_binder_interface!` macro calls missing the `adapter:` and `r#async:` fields that the rsbinder macro requires.
- `.output($OUT_DIR/aosp_hal_bindings.rs)`, `.generate()`.
- `cargo:rerun-if-changed` on both AIDL dirs.

Concrete sources for this task: `IVibrator.aidl` and `ILights.aidl` only. `IVibratorCallback.aidl` is pulled in automatically via `IVibrator`'s import.

New file `wart-host/src/binder_aidl.rs`:

```rust
#[cfg(target_os = "android")]
#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, unused_imports, clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/aosp_hal_bindings.rs"));
}
#[cfg(target_os = "android")]
pub use generated::*;
```

`lib.rs:13` — add `mod binder_aidl;` after `mod binder;`.

**Verify:** `cargo build --target aarch64-linux-android --release` succeeds; `find target -path '*/aosp_hal_bindings.rs' | head -1` exists and is non-empty.

### 6. Create rsbinder-triage agent

New file `.claude/agents/rsbinder-triage.md` covering five common failure patterns: AVC denials, service-not-found, parcelable layout drift, `binder init failed`, `libbinder_ndk.so` link errors. Tool restrictions: `Bash, Read, Grep`. Output format: one paragraph with evidence + exactly one suggested next action.

### 7. Commit + verify

Suggested commit boundaries (one commit per sub-step for clean bisect):

1. `vendor: shallow submodule for AOSP hardware-interfaces (android-11.0.0_r48)`
2. `wart-host: bump min_sdk_version 29→30, add VIBRATE permission`
3. `wart-host: add rsbinder + rsbinder-aidl deps`
4. `wart-host: binder ProcessState init helper`
5. `wart-host: rsbinder-aidl codegen pipeline + binder_aidl.rs wrapper`
6. `agent: rsbinder-triage for binder runtime failures`

**End-to-end verify:**
- `cargo apk build --release` succeeds, APK installs, app runs.
- Logcat clean: no `binder init failed` warning on the Pixel 2 XL.
- Existing PoC features unchanged (Compose UI, scroll, TextField, soft keyboard).
- All six files exist where the architecture diagram says they should.

---

## Known issues / risks

1. **rsbinder 0.7.0 hard-wires async machinery into its macros.** We tried `default-features = false, features = ["android_11"]` (no tokio) and got `BoxFuture not found` errors from rsbinder's own internal service-manager codegen. Adding `"async"` produced dyn-incompat traits. The combination that builds: `features = ["android_11"]` with **defaults on** (pulls tokio) + `async-trait = "0.1"` as a direct dep + `set_async_support(true)` on the Builder. Net binary cost: ~0.8 MB. Optimization opportunities for later: patch rsbinder-aidl to emit complete macros under `set_async_support(false)`, or vendor a fork that strips tokio.

2. **AIDL codegen output layout** confirmed: generates nested modules `mod android { mod hardware { mod vibrator { mod IVibrator { pub trait IVibrator: ... } } } }`. The trait lives in a sub-module of the same name as the interface, so call paths are `crate::binder_aidl::android::hardware::vibrator::IVibrator::IVibrator` (module then trait). Same for `ILights::ILights` and parcelables like `Effect`, `HwLightState`.

3. **Build.rs hard-codes `api = 35`** for the sysroot link search path. The linker (`android30-clang`) and the rustc minimum-SDK don't match the sysroot lib dir, but `.so` symbols are stable across API 30+ so this works in practice. Cleanup follow-up: pass `ANDROID_PLATFORM=30` through consistently.

4. **rsbinder transport on Android** uses `libbinder_ndk.so`, which is NDK-sanctioned and present in the sysroot from API 29+. We bumped to API 30 for stable AIDL HAL availability, so this is fine.

5. **SELinux on stock devices** will deny `untrusted_app → hal_*` binder calls. This blocks production use of tasks 16 and 17 until the boot-model work in roadmap §6.1 lands (running WAR as an init.rc service with proper seclabel).

---

## Out of scope

- Calling any HAL service from this task — see tasks 16 (vibrator) and 17 (lights).
- `ISurfaceComposer.getDisplayInfo` smoke test — noted in roadmap §5 as a separate de-risking step.
- Compose `DefaultHapticFeedback.skiko.kt` stub — separate task 18 (Kotlin/Compose, not Rust/binder).
