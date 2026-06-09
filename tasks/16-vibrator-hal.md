# Task 16 — Vibrator HAL (rewire haptics_impl)

> **Status: ✅ device-verified on Pixel 2 XL 2026-05-17.** Phone buzzes when `Haptics.perform(CLICK)` is called from the WASM guest — full chain WIT → host → rsbinder → IVibrator HAL → hardware works end-to-end. Required two non-obvious workarounds that surfaced only during real-device testing — see "Workarounds" section below. **WIT interface, WIT mirror, Kotlin bindings, and Compose adapter all unchanged** from the originally-planned design; only the Rust host impl beneath the WIT trait changes.

## Goal

Make `Haptics.perform(feedback)` and `Haptics.vibrate_ms(ms)` reach a working vibrator on a stock Android 11+ device (with `setenforce 0` for the rooted-dev case). Until this task, the Rust host wrote to `/sys/class/timed_output/vibrator/enable` and `/sys/class/leds/vibrator/*`, both of which `EACCES` on stock Android — so `perform()` always returned `false` and the phone never buzzed even when Compose called the WIT interface.

Reference: `~/wandr/post-art-roadmap.md` §3 (Boundary B / Pattern 5) and Task 15 (pipeline).

---

## Architecture

```
WIT haptics.perform(feedback)
  └─ Rust Host::perform()
       ├─ binder_path::perform(feedback)          ← try this first
       │    ├─ map_feedback → (Effect, EffectStrength)
       │    ├─ rsbinder::hub::get_interface("android.hardware.vibrator.IVibrator/default")
       │    ├─ svc.perform(effect, strength, &cb)
       │    └─ if EX_UNSUPPORTED_OPERATION → vibrate_ms(duration_table[feedback])
       └─ try_vibrate_sysfs(duration)             ← fallback only if binder fails
```

The binder service handle + the no-op IVibratorCallback Strong are cached in `OnceLock`s — looked up exactly once per process, reused for every WIT call.

## Effect mapping

Mirrors the framework's `HapticFeedbackConstants → VibrationEffect` mapping in `frameworks/base/services/core/java/com/android/server/vibrator/`.

| WIT `Feedback` | AIDL `Effect` | `EffectStrength` |
|---|---|---|
| `Tap`         | `Effect::TICK`         | `LIGHT`  |
| `VirtualKey`  | `Effect::TICK`         | `MEDIUM` |
| `Click`       | `Effect::CLICK`        | `MEDIUM` |
| `LongPress`   | `Effect::HEAVY_CLICK`  | `STRONG` |
| `DoubleClick` | `Effect::DOUBLE_CLICK` | `MEDIUM` |

If the device's HAL returns `EX_UNSUPPORTED_OPERATION` for the chosen effect (not in `getSupportedEffects()`), we fall back to `IVibrator::on(ms)` with the legacy duration table (`Tap`/`VirtualKey`/`Click`=10ms, `LongPress`=40ms, `DoubleClick`=20ms).

---

## Workarounds discovered during device verification

Two pieces had to change vs. the plan after seeing actual Pixel 2 XL behavior:

### Workaround 1: rsbinder feature must be `android_11_plus`, not `android_11`

rsbinder's `hub::get_interface` panics with `default: Unsupported Android SDK version: 35` when run on Android 15 (SDK 35). rsbinder groups SDK 34 + 35 → its `android_14` feature; the runtime-dispatch table must have a `cfg(feature = "android_14")` arm enabled, otherwise unmatched-SDK → panic. `android_11_plus` is `["android_11", "android_12", "android_13", "android_14", "android_16"]` — covers SDK 30 through future SDK 36+. Documented in task 15.

### Workaround 2: `@nullable IVibratorCallback` via manual parcel transaction

AOSP `IVibrator.aidl` declares both `on()` and `perform()` with `@nullable IVibratorCallback`. The Pixel 2 XL's HAL returns `getCapabilities() = 196` — bits `CAP_ON_CALLBACK = 0` and `CAP_PERFORM_CALLBACK = 0` — meaning **the HAL refuses calls that pass a non-null callback** (returns `EX_UNSUPPORTED_OPERATION`). It will only accept null.

rsbinder-aidl 0.7.0 doesn't translate `@nullable` to `Option<&Strong>` — the generated Rust signature requires `&rsbinder::Strong<dyn IVibratorCallback>`, non-null only. So the generated proxy is unusable on any HAL with `CAP_*_CALLBACK = 0` (which is most older Pixels).

**Fix:** bypass the generated proxy and build the parcel by hand. rsbinder exposes:

- `Strong::as_binder() → SIBinder` (via the `Interface` supertrait of `IVibrator`)
- `SIBinder::as_proxy() → Option<&ProxyHandle>`
- `ProxyHandle::prepare_transact(write_header: bool) → Result<Parcel>`
- `ProxyHandle::submit_transact(code, &Parcel, flags) → Result<Option<Parcel>>`
- `rsbinder::FIRST_CALL_TRANSACTION: u32 = 1` (transaction codes from r48 AIDL: `on` = +2, `perform` = +3)
- Blanket `impl<T: SerializeOption> Serialize for Option<T>` — writing `&None::<rsbinder::Strong<dyn IVibratorCallback>>` produces a null binder reference in the parcel

So `transact_on(svc, ms)`:
```rust
let proxy = svc.as_binder().as_proxy().unwrap();
let mut data = proxy.prepare_transact(true)?;
data.write(&ms)?;
let null_cb: Option<rsbinder::Strong<dyn IVibratorCallback>> = None;
data.write(&null_cb)?;          // ← serializes as null binder
proxy.submit_transact(TXN_ON, &data, 0)
```

No `NopCallback`, no `TrivialRuntime`, no `BnVibratorCallback::new_async_binder`. The previously-planned approach (no-op callback wrapped in `new_async_binder` + a `BinderAsyncRuntime`) was attempted first; it caused `rsbinder::binder_object: flat_binder_object::acquire: unknown native id 2` errors with `TrivialRuntime` (rsbinder's local-binder bookkeeping requires a real runtime), then worked with a tokio current-thread runtime but the HAL still refused the call with `EX_UNSUPPORTED_OPERATION` because of the capability gap.

Net code: ~90 lines in `binder_path` module, no tokio dep needed, no async-trait emitted by our code (still needed transitively by rsbinder).

### Implications for the upstream

This workaround is generally useful — `@nullable IVibratorCallback` is just one instance of a broader gap. Any AOSP AIDL parameter annotated `@nullable` for a binder type can't currently be sent as null through rsbinder-aidl 0.7.0's generated proxies. Future cleanup: PR rsbinder-aidl to honor `@nullable` and emit `Option<&Strong<...>>` parameter types so the manual transact dance isn't needed per-call.

---

## Steps

### 1. Inspect generated IVibrator API

Confirm the trait signatures, enum constants, and module paths from the rsbinder-aidl codegen output at `$OUT_DIR/aosp_hal_bindings.rs`:

- `crate::binder_aidl::android::hardware::vibrator::IVibrator::IVibrator` (trait)
- `crate::binder_aidl::android::hardware::vibrator::Effect::Effect` (newtype `pub struct Effect(pub i32)` with associated consts `CLICK`, `TICK`, `HEAVY_CLICK`, `DOUBLE_CLICK`, ...)
- `crate::binder_aidl::android::hardware::vibrator::EffectStrength::EffectStrength` (newtype `pub struct EffectStrength(pub i8)` with `LIGHT`, `MEDIUM`, `STRONG`)
- `crate::binder_aidl::android::hardware::vibrator::IVibratorCallback::{IVibratorCallback, IVibratorCallbackAsyncService, BnVibratorCallback}`

### 2. Rewrite `wandr-host/src/haptics_impl.rs`

- Keep the existing `try_vibrate_sysfs` function (was `try_vibrate`) and `feedback_duration` table.
- Add `#[cfg(target_os = "android")] mod binder_path` with: `NopCallback`, `TrivialRuntime`, two `OnceLock`s (`VIB`, `CB`), `service()`, `callback()`, `vibrate_ms()`, `perform()`, `map_feedback()`.
- `Host::perform()` tries binder first, falls back to sysfs on `false`.
- `Host::vibrate_ms()` same pattern with `clamp(1, 1000)`.

### 3. Build verify

```bash
NDK=$HOME/android-ndk-r27d
TC=$NDK/toolchains/llvm/prebuilt/linux-x86_64
export ANDROID_NDK_HOME=$NDK PATH=$TC/bin:$PATH \
       CC_aarch64_linux_android=$TC/bin/aarch64-linux-android30-clang \
       CXX_aarch64_linux_android=$TC/bin/aarch64-linux-android30-clang++ \
       AR_aarch64_linux_android=$TC/bin/llvm-ar \
       CC_aarch64_linux_android_RANLIB=$TC/bin/llvm-ranlib
cd ~/wandr/wandr-host && cargo build --target aarch64-linux-android --release
```

Expected: succeeds in ~40s incremental, 5 benign warnings (all pre-existing dead code). `libwasm_android_host.so` ~50.7 MB.

### 4. Device verify (Pixel 2 XL, Android 15 / API 35 — verified 2026-05-17)

Setup:
```bash
adb root
adb shell setenforce 0          # SELinux permissive; needed for untrusted_app → hal_vibrator_default
adb shell service list | grep vibrator
# expect: android.hardware.vibrator.IVibrator/default: [android.hardware.vibrator.IVibrator]
```

Build + deploy + restart pipeline (CLAUDE.md "Build pipeline" §):
1. `cd ~/wandr/wandr-host && cargo apk build --release` — produces `target/release/apk/wasm_android_host.apk` (~32 MB)
2. `adb install -r <apk>`
3. `cd ~/wandr/wandr-app && ./gradlew compileProductionExecutableKotlinWasmWasi` — produces guest .wasm
4. `wasm-tools component embed --world my:skiko-gfx/skiko-ui ~/wandr/wit/skiko-gfx.wit <.wasm> -o /tmp/embedded.wasm`
5. `wasm-tools component new /tmp/embedded.wasm --adapt ~/wandr/skiko/wasi_snapshot_preview1.reactor.wasm -o /tmp/skiko-component.wasm`
6. `wasmtime compile --target aarch64-linux-android -W component-model -W gc -W function-references -W exceptions -o /tmp/skiko-component.cwasm /tmp/skiko-component.wasm` (~65 MB)
7. `adb shell am force-stop com.example.wasmruntime`
8. `adb push /tmp/skiko-component.cwasm /sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm`
9. `adb shell am start -n com.example.wasmruntime/android.app.NativeActivity`

Existing `android-haptics smoke` block in `wandr-app/src/wasmWasiMain/kotlin/Main.kt:109` already exercises the WIT — no temporary test code needed. Expected logcat lines (verified):

```
I wasm_android_host::ha..: haptics: IVibrator.perform(Effect(2),EffectStrength(0),null) → ok
I wasm_android_host::ha..: haptics: IVibrator.on(50ms, null) → ok
I wasm_android_host::ca..: [wasm] android-haptics smoke: perform(TAP)=true, vibrateMs(50)=true
```

Physical confirmation: phone buzzes twice on app start (TAP + 50ms).

Negative control: `adb shell setenforce 1` → AVC denials in dmesg → `binder_path::perform()` returns false → sysfs fallback also returns false (Pixel 2 XL has no `/sys/class/timed_output/vibrator/enable` or `/sys/class/leds/vibrator/*`) → no buzz, no crash. Behavior is graceful.

**Compose path not yet wired** — Compose's `DefaultHapticFeedback.skiko.kt` is still an empty stub, so `performHapticFeedback()` from Compose code never reaches our WIT. Task 18 (future) will write the override.

---

## Known issues / risks

1. **SELinux on stock devices** denies `untrusted_app → hal_vibrator_default` binder calls. Production fix waits for the boot-model work in roadmap §6.1 (running WAR as an init.rc service with a seclabel allowed to talk to vibrator HAL). For now: `setenforce 0` is the dev workflow on rooted devices; on a non-rooted consumer phone the binder path is permanently blocked (would need JNI-to-`android.os.Vibrator` as a tier-0 fallback to support that case).

2. **`@nullable` not translated** by rsbinder-aidl 0.7.0 — worked around by hand-built parcel transactions in `binder_path::transact_{on,perform}`. Upstream fix opportunity: PR rsbinder-aidl to honor `@nullable` so `Option<&Strong<...>>` parameters are emitted directly. Until then, every `@nullable` binder param needs the manual-transact dance.

3. **Effect support varies by device.** Pixel 2 XL's `getSupportedEffects()` returns `[TEXTURE_TICK, TICK, CLICK, HEAVY_CLICK, DOUBLE_CLICK]` — covers all 5 of our WIT mappings, no fallback needed in practice. On a stripped-down HAL that doesn't support a chosen effect, `perform()` returns `EX_UNSUPPORTED_OPERATION` and we fall through to `transact_on(duration_table[feedback])`.

4. **`getCapabilities()` not consulted at runtime.** The current impl assumes "always pass null callback" because that universally works (HALs with `CAP_*_CALLBACK = 0` require null; HALs with the bit set accept either). A future optimization could check capabilities once and pass a real callback when supported — but only useful if we actually want completion notifications, which the WIT doesn't expose.

5. **Compose still doesn't reach this code.** `DefaultHapticFeedback.skiko.kt` is empty. Task 16's verification happens via direct Kotlin-side `Haptics.Import.perform(...)` invocation in the existing `android-haptics smoke` block (Main.kt:109), not via Compose `performHapticFeedback()`. Task 18 will wire Compose through.

6. **`rsbinder` SDK-35 dispatch gap.** SDK 35 (Android 15) is only matched in rsbinder's runtime version table if the `android_14` feature (or `android_11_plus` umbrella) is enabled. Task 15 was updated from `["android_11"]` to `["android_11_plus"]` during this work.

---

## Out of scope

- Lights HAL — task 17.
- Compose `HapticFeedback` adapter — task 18.
- `IVibratorManager` (Android 12+, supports per-vibrator addressing for multi-actuator devices) — current single-vibrator API is sufficient for Pixel 2 XL.
- `compose()` / `alwaysOnEnable()` / `setAmplitude()` — advanced waveform APIs not exposed by our WIT.
