# Task 16 — Vibrator HAL (rewire haptics_impl)

> **Status: ✅ implementation complete (built 2026-05-16), device-verification pending.** Rewrites `wart-host/src/haptics_impl.rs` to call `android.hardware.vibrator.IVibrator` over binder via rsbinder. Sysfs fallback retained for non-Android Linux + custom ROMs without an AIDL HAL. **WIT interface, WIT mirror, Kotlin bindings, and Compose adapter are all unchanged** — only the Rust host impl beneath the WIT trait changes.

## Goal

Make `Haptics.perform(feedback)` and `Haptics.vibrate_ms(ms)` reach a working vibrator on a stock Android 11+ device (with `setenforce 0` for the rooted-dev case). Until this task, the Rust host wrote to `/sys/class/timed_output/vibrator/enable` and `/sys/class/leds/vibrator/*`, both of which `EACCES` on stock Android — so `perform()` always returned `false` and the phone never buzzed even when Compose called the WIT interface.

Reference: `~/wart/post-art-roadmap.md` §3 (Boundary B / Pattern 5) and Task 15 (pipeline).

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

## The `@nullable IVibratorCallback` workaround

The AOSP AIDL declares both `on()` and `perform()` with `@nullable IVibratorCallback`. rsbinder-aidl 0.7.0 doesn't translate `@nullable` to `Option<&Strong>` — the generated Rust signature requires `&rsbinder::Strong<dyn IVibratorCallback>`. So we must pass a real callback, even though we don't care about completion.

The smallest workable callback:

1. **Struct `NopCallback`** — `impl Interface` + `impl IVibratorCallbackAsyncService` (the async-flavored trait) with `async fn r#onComplete -> Ok(())`. Returns Ready on the first poll, never pends.
2. **Struct `TrivialRuntime`** — `impl rsbinder::BinderAsyncRuntime` with a hand-rolled `block_on` using a dummy `Waker` (no tokio dep). Polls the future once; panics on `Pending` because any other future on this runtime would be a bug.
3. **Cached `Strong<dyn IVibratorCallback>`** — built once via `BnVibratorCallback::new_async_binder(NopCallback, TrivialRuntime)` and stored in a `OnceLock`.

This is ~30 lines and adds no runtime dependencies. Cost: one `BnVibratorCallback` binder server gets registered with the kernel binder driver per process. The vibrator HAL daemon will call back into our process once per `on()`/`perform()`, our `onComplete` returns Ok, end of story.

---

## Steps

### 1. Inspect generated IVibrator API

Confirm the trait signatures, enum constants, and module paths from the rsbinder-aidl codegen output at `$OUT_DIR/aosp_hal_bindings.rs`:

- `crate::binder_aidl::android::hardware::vibrator::IVibrator::IVibrator` (trait)
- `crate::binder_aidl::android::hardware::vibrator::Effect::Effect` (newtype `pub struct Effect(pub i32)` with associated consts `CLICK`, `TICK`, `HEAVY_CLICK`, `DOUBLE_CLICK`, ...)
- `crate::binder_aidl::android::hardware::vibrator::EffectStrength::EffectStrength` (newtype `pub struct EffectStrength(pub i8)` with `LIGHT`, `MEDIUM`, `STRONG`)
- `crate::binder_aidl::android::hardware::vibrator::IVibratorCallback::{IVibratorCallback, IVibratorCallbackAsyncService, BnVibratorCallback}`

### 2. Rewrite `wart-host/src/haptics_impl.rs`

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
cd ~/wart/wart-host && cargo build --target aarch64-linux-android --release
```

Expected: succeeds in ~40s incremental, 5 benign warnings (all pre-existing dead code). `libwasm_android_host.so` ~50.7 MB.

### 4. Device verify (pending, requires Pixel 2 XL)

Build APK and deploy:

```bash
cargo apk build --release
~/wart/scripts/deploy.sh
```

Then exercise the WIT:

- **Positive test** (rooted, `adb shell setenforce 0`):
  Add a one-line `Haptics.Import.perform(Haptics.Feedback.CLICK)` at startup in `wart-app/src/wasmWasiMain/kotlin/Main.kt`. Expect: phone buzzes once on app start; `adb logcat | grep -i vibrator` shows the binder transaction.
- **Negative control** (`adb shell setenforce 1`):
  Same test. Expect: AVC denial in dmesg, `binder_path::perform()` returns false, sysfs fallback also returns false (paths don't exist on this device), no buzz, no crash.
- **Compose path**: still won't trigger this code today — Compose's `DefaultHapticFeedback.skiko.kt` is an empty stub. See task 18.

Remove the test invocation before committing.

---

## Known issues / risks

1. **SELinux on stock devices** denies `untrusted_app → hal_vibrator_default` binder calls. Production fix waits for the boot-model work in roadmap §6.1 (running WAR as an init.rc service with a seclabel allowed to talk to vibrator HAL). For now: `setenforce 0` is the dev workflow.

2. **`@nullable` not translated** by rsbinder-aidl 0.7.0 — see workaround section above. Upstream fix opportunity: PR rsbinder-aidl to honor `@nullable` and emit `Option<&Strong<...>>`. Until then, the NopCallback boilerplate stands.

3. **Effect support varies by device.** Pixel 2 XL implements only `CLICK` and a couple others; `HEAVY_CLICK` returns `EX_UNSUPPORTED_OPERATION` → falls back to `on(40)`. Tested mapping is documented above.

4. **TrivialRuntime panics on `Pending`.** If a future rsbinder version changes `IVibratorCallbackAsyncService::r#onComplete` to actually await something, this will crash. Add a runtime-detection check (or switch to tokio current-thread runtime) if upgrading rsbinder.

5. **Compose still doesn't reach this code.** `DefaultHapticFeedback.skiko.kt` is empty. Task 16's verification therefore happens via direct Kotlin-side `Haptics.Import.perform(...)` invocation, not via Compose `performHapticFeedback()`. Task 18 will wire Compose through.

---

## Out of scope

- Lights HAL — task 17.
- Compose `HapticFeedback` adapter — task 18.
- `IVibratorManager` (Android 12+, supports per-vibrator addressing for multi-actuator devices) — current single-vibrator API is sufficient for Pixel 2 XL.
- `compose()` / `alwaysOnEnable()` / `setAmplitude()` — advanced waveform APIs not exposed by our WIT.
