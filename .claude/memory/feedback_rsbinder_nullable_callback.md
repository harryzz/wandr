---
name: feedback-rsbinder-nullable-callback
description: "rsbinder-aidl 0.7.0 doesn't translate AIDL @nullable to Option<&Strong>. For HAL calls where the device requires null binder (CAP_*_CALLBACK=0 returned by getCapabilities), bypass the generated proxy and write the parcel by hand with Option::<Strong>::None as a null binder ref."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 65c8020a-f9c3-4d47-8372-153531fef6d2
---

When calling a stable AIDL HAL method that declares an `@nullable IFooCallback` parameter and the device's HAL doesn't support callbacks for that method, the generated `rsbinder-aidl` proxy will fail with `Status::EX_UNSUPPORTED_OPERATION` because rsbinder-aidl 0.7.0 doesn't translate `@nullable` to `Option<&Strong<...>>`. **Fix: bypass the generated proxy and build the parcel manually with `Option::<Strong<dyn IFooCallback>>::None`** — rsbinder's blanket `impl<T: SerializeOption> Serialize for Option<T>` writes `None` as a null binder reference in the parcel.

**Why:** Real-device discovery on Pixel 2 XL (Android 15) verifying task 16. `IVibrator.aidl` declares `void on(in int timeoutMs, in @nullable IVibratorCallback callback)`. The Pixel 2 XL HAL returns `getCapabilities() = 196` — bits `CAP_ON_CALLBACK=0` and `CAP_PERFORM_CALLBACK=0`, meaning *the HAL refuses non-null callback parameters*. The previously-planned `NopCallback + BinderAsyncRuntime` workaround was attempted in two flavors and failed in both: (a) hand-rolled single-poll `TrivialRuntime` caused `rsbinder::binder_object: flat_binder_object::acquire: unknown native id 2` errors because rsbinder's local-binder bookkeeping requires a real async runtime; (b) tokio current-thread runtime fixed the bookkeeping but the HAL still rejected the call because it has CAP=0. Manual-parcel + null-binder is the only path that works on these older HALs, and it works universally (HALs with CAP=1 accept null too — they just don't fire the completion notification we never wanted).

**How to apply:** Whenever you wire a new rsbinder HAL call where the AIDL has `@nullable` on a binder parameter:

1. Don't use the generated proxy method (e.g. `svc.r#on(ms, cb)`) — its `&Strong<...>` parameter rules out null.
2. Get the underlying transaction handle: `let binder = svc.as_binder(); let proxy = binder.as_proxy()?;`
3. Build the parcel by hand: `let mut data = proxy.prepare_transact(true)?; data.write(&arg1)?; data.write(&Option::<Strong<dyn IFooCallback>>::None)?;`
4. Submit with the transaction code constant from the generated bindings' `pub(crate) mod transactions` (e.g. `FIRST_CALL_TRANSACTION + 2` for IVibrator.on, `+ 3` for IVibrator.perform). The codes are visible in `$OUT_DIR/aosp_hal_bindings.rs`.
5. `proxy.submit_transact(TXN_CODE, &data, 0)` — `flags = 0` for normal sync calls; `FLAG_ONEWAY | FLAG_CLEAR_BUF` for oneway.
6. Don't read the reply parcel unless you need the return value — `submit_transact` returns `Result<Option<Parcel>>` and you can drop the inner `Parcel`.

Concrete worked example in [[project-wasm-runtime]] at `wandr-host/src/haptics_impl.rs` — module `binder_path`, functions `transact_on` and `transact_perform`. ~90 lines total replace the entire previously-planned NopCallback / TrivialRuntime / new_async_binder path.

**Related: rsbinder Android SDK version dispatch.** Same task 16 device verification surfaced that rsbinder 0.7.0's `hub::get_interface` panics with `default: Unsupported Android SDK version: 35` if the corresponding `android_NN` feature isn't compiled in. rsbinder groups SDK 34 + 35 under `android_14`. Use `features = ["android_11_plus"]` (which includes `android_11..android_14, android_16`) rather than a single-version feature to cover the runtime SDK dispatch table. See [[project-wasm-runtime]] §current-status.

**Upstream cleanup opportunity:** PR rsbinder-aidl to honor `@nullable` and emit `Option<&Strong<...>>` parameters in the generated proxies. Until that lands, this manual-parcel dance is needed for every `@nullable` binder param across HAL bindings.
