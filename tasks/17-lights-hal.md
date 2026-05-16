# Task 17 — Lights HAL (new WIT interface)

> **Status: ✅ implementation + Kotlin gradle verification complete (2026-05-16), device verification pending.** Adds a brand-new `lights` WIT interface backed by `android.hardware.light.ILights` via rsbinder. Hand-edited Kotlin bindings (WIT mirror + SkikoUi.kt + InternalSkikoUi.kt) compile cleanly: skiko klib republishes (1m 37s), `compileProductionExecutableKotlinWasmWasi` in wart-app succeeds (1m 53s) producing `wart-app.wasm` 11.3 MB. compose-multiplatform-core did NOT need recompilation (additive change). Production access likely blocked by SELinux on stock devices until boot-model work — see Known issues.

## Goal

Expose Android's stable AIDL `ILights` HAL to WASM guests so a Compose app can turn on the notification LED, change keyboard backlight, etc. The WIT is hand-authored in domain terms (`set(notifications, green)`) — guest never sees AIDL `HwLightState` parcelables or per-physical-light `id` numbering.

Reference: `~/wart/post-art-roadmap.md` §3 + Task 15 (rsbinder pipeline) + Task 16 (vibrator pattern reused here).

---

## Architecture

```
WIT lights.set(kind, state)                          ← guest call
  └─ Rust Host::set()
       └─ binder_path::set(kind, state)
            ├─ rsbinder::hub::get_interface("android.hardware.light.ILights/default")
            ├─ svc.getLights() → Vec<HwLight>
            ├─ filter by AidlLightType matching kind
            └─ for each: svc.setLightState(hw.id, &HwLightState { color, flash_*, ... })
```

The service Strong is cached in a `OnceLock` — one binder lookup per process.

Compared to Task 16 (vibrator), `ILights` has no `@nullable` parameter pitfalls — both methods are simple, no callbacks. The implementation is ~90 lines including the WIT↔AIDL enum mapping helpers.

---

## WIT design

```wit
interface lights {
    enum light-type {
        backlight, keyboard, buttons, battery, notifications,
        attention, bluetooth, wifi, microphone,
    }
    enum flash-mode { %none, timed, hardware }
    record light-state {
        color-argb:    u32,
        flash-on-ms:   u32,
        flash-off-ms:  u32,
        flash-mode:    flash-mode,
    }
    set:      func(kind: light-type, state: light-state) -> bool;
    supports: func(kind: light-type) -> bool;
}
```

**Design decisions:**

- **`light-type` order matches AIDL `LightType` ordinals 0..8.** WIT enum ordinal `Backlight=0 ... Microphone=8` maps 1:1 to AOSP r48's `android.hardware.light.LightType`. The Rust host can almost cast; we use an explicit `match` for safety against future enum drift (camera was added later in some AOSP branches — keep AIDL alignment explicit, not implicit).
- **9 values, not 10.** AOSP `android-11.0.0_r48` ships `LightType` with `MICROPHONE=8` as the last variant. Camera, added in later releases, is intentionally absent — adding it now would break our `wit_to_aidl_type` cast on Android 11 devices.
- **`brightness-mode` not exposed.** AIDL `HwLightState` has a `brightnessMode` field (`USER`/`SENSOR`/`LOW_PERSISTENCE`). It's device-specific OEM behavior. Default to `USER` (0) on the Rust side; guest doesn't get to override. Add to WIT later if a real use case shows up.
- **`%none` escape.** `none` is a WIT reserved word. `%none` is the WIT identifier-escape syntax. Translates to `FlashMode.NONE` on the Kotlin side as expected.
- **No `get-lights()` exposed.** AIDL `getLights()` returns per-physical-light info (id, ordinal, type). Exposing this would force guests to think about which `id` corresponds to which physical LED — unnecessary complexity. The host iterates internally.

---

## Steps

### 1. Inspect generated ILights API

Confirm signatures + types in `$OUT_DIR/aosp_hal_bindings.rs`:

- `ILights::ILights` trait: `setLightState(id: i32, &HwLightState) -> Result<()>`, `getLights() -> Result<Vec<HwLight>>`
- `HwLightState { color: i32, flashMode: FlashMode, flashOnMs: i32, flashOffMs: i32, brightnessMode: BrightnessMode }`
- `HwLight { id: i32, ordinal: i32, type: LightType }`
- `LightType` newtype `(pub i8)` with consts `BACKLIGHT=0`..`MICROPHONE=8`
- `FlashMode` newtype `(pub i8)` with `NONE=0`/`TIMED=1`/`HARDWARE=2`
- `BrightnessMode` newtype `(pub i8)` with `USER=0`/`SENSOR=1`/`LOW_PERSISTENCE=2`

### 2. Add WIT interface + sync mirror

- Append `interface lights { ... }` to `wit/skiko-gfx.wit` after the haptics block.
- Add `import lights;` inside `world skiko-ui`.
- `cp wit/skiko-gfx.wit skiko/skiko/wit/skiko-gfx.wit` — mirror rule, byte-identical.

### 3. Hand-edit Kotlin bindings

Two files, mirror the existing `Haptics` pattern:

- `skiko/skiko/src/wasmWasiMain/kotlin/generated/InternalSkikoUi.kt`:
  ```kotlin
  @WasmImport("my:skiko-gfx/lights@0.1.0", "set")
  internal external fun __wasm_import_lights_set(p0: Int, p1: Int, p2: Int, p3: Int, p4: Int): Int

  @WasmImport("my:skiko-gfx/lights@0.1.0", "supports")
  internal external fun __wasm_import_lights_supports(p0: Int): Int
  ```
  5 args for `set`: `(kind, color-argb, flash-on-ms, flash-off-ms, flash-mode)`. Component-model canonical ABI flattens the `light-state` record's 4 fields inline because the total arg-count (1 + 4 = 5 i32s) is well under the flatten limit.

- `skiko/skiko/src/wasmWasiMain/kotlin/generated/SkikoUi.kt`:
  Insert a `@WitInterface("my:skiko-gfx/lights@0.1.0") interface Lights { ... }` block with `LightType` + `FlashMode` enum classes, a `LightState` data class, and the `companion object Import : org.jetbrains.skiko.wasi.wit.Lights` mirroring Haptics' Import block. Wrap each call in `freeAllComponentModelReallocAllocatedMemory()` + `withScopedMemoryAllocator` per the existing convention (memory: `feedback_wasi_realloc_allocator`).

### 4. Write `wart-host/src/lights_impl.rs` + wire `lib.rs`

- New file ~90 lines: `Host` trait impl + `binder_path` module.
- `binder_path::set` enumerates `getLights()`, filters by AIDL `LightType`, calls `setLightState` on each match. Returns `true` if any succeeded.
- `binder_path::supports` enumerates and returns `true` if any match.
- WIT↔AIDL enum translation in explicit `match` statements (safer than ordinal cast against future AIDL drift).
- `lib.rs:10`: add `mod lights_impl;` — `bindings::SkikoUi::add_to_linker` auto-picks up the new `Host` impl via the `bindgen!` macro.

### 5. Verify

- **Rust:** `cargo build --target aarch64-linux-android --release` — succeeds in 40s incremental, 5 benign warnings (all pre-existing dead code). `libwasm_android_host.so` ~50.84 MB.
- **Kotlin (verified 2026-05-16):**
  1. `cd ~/wart/skiko/skiko && ./gradlew publishWasmWasiPublicationToMavenLocal -Pskiko.wasmWasi.enabled=true -Dorg.gradle.configureondemand=false --console=plain --no-daemon` — republishes the wasmWasi klib with the new Lights bindings. **Note:** CLAUDE.md showed the task name as `:skiko:publishKotlinMultiplatformDecoratedPublicationToMavenLocal` but the actual task is `publishWasmWasiPublicationToMavenLocal` (no `:skiko:` prefix because we're already in the skiko project root; publication is named `wasmWasi`, not `KotlinMultiplatformDecorated`). 1m 37s.
  2. `cd ~/wart/wart-app && ./gradlew compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon` — compiles guest against new klib. **Note:** task name in CLAUDE.md (`wasmWasiProductionExecutable`) doesn't exist; correct task is `compileProductionExecutableKotlinWasmWasi`. 1m 53s, produces `build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm` (~11.3 MB).
  3. Additive-only skiko change → compose-multiplatform-core did NOT need recompilation. Confirmed: wart-app builds clean against new skiko + cached compose klibs.
- **Device (pending, requires Pixel 2 XL with `adb shell setenforce 0`):**
  - Build APK + AOT + deploy per `~/wart/wart-app/BUILD.md`.
  - In `Main.kt`: `Lights.Import.set(Lights.LightType.NOTIFICATIONS, Lights.LightState(colorArgb = 0xFF00FF00u, flashOnMs = 500u, flashOffMs = 500u, flashMode = Lights.FlashMode.TIMED))`
  - Expect: green LED blinking. Or `false` returned + logcat AVC denial if SELinux blocks — fine, code didn't crash.
  - `Lights.Import.supports(Lights.LightType.NOTIFICATIONS)` should return `true` on Pixel 2 XL (has a notification LED).

---

## Known issues / risks

1. **SELinux is the dominant production blocker.** `untrusted_app` typically cannot bind to `hal_light_default`. Even with `android.permission.LIGHTS` (privileged, signature-only), the binder transaction fails at the kernel binder driver. Production fix: roadmap §6.1 (init.rc service with appropriate `seclabel`). Dev workflow: `adb shell setenforce 0`.

2. **No manifest permission added.** `android.permission.LIGHTS` is `signature|privileged` — declaring it in an `untrusted_app` APK does nothing useful. Leaving it out keeps the manifest honest.

3. **Empty `getLights()` on some devices.** Some OEMs don't register their lights through the AIDL HAL at all — the `light-service` daemon may exist but expose zero logical lights. In that case `supports()` always returns false and `set()` always returns false. No crash; just no LED control.

4. **Component-model ABI for `set`.** Hand-edited Kotlin assumes the canonical ABI flattens the `(kind, light-state)` arg list into 5 i32 params (1 enum + 4 record fields). If wit-bindgen ever changes its flatten heuristics to pass via memory for records this size, the Kotlin call will silently break (the host would read garbage from stack/zero). **Verification: ensure first device test logs the expected `kind` ordinal in the host's `set()` impl.**

5. **`brightness-mode` not exposed.** Acceptable trade-off (see WIT design notes). Add `brightness-mode` field to the WIT record if a real use case appears.

---

## Out of scope

- Compose adapter for lights — no upstream Compose primitive maps to LED control. Exposing lights to Compose UI is a guest-API design problem, not a Boundary B issue.
- Per-physical-light addressing — guests get bulk `set(kind, state)` only.
- `IVibratorManager`-style multi-light-controller addressing — not needed; ILights is the only lighting HAL.
- Camera light (added to AIDL `LightType` in later AOSP releases) — would require bumping the vendored submodule to a newer android-NN tag and re-validating all other AIDLs. Defer until a use case demands it.
