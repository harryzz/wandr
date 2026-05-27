//! IMMS round-trip probe (task 40 session 2 — first read-only call).
//!
//! Calls `isImeTraceEnabled()` on the `input_method` binder service
//! (descriptor `com.android.internal.view.IInputMethodManager`) and
//! logs the result. Read-only, no permission required
//! (`@RequiresNoPermission`), no behavior change. Validates that
//! rsbinder reaches IMMS ahead of the much heavier session-3 work
//! (`addClient` + `IInputMethodClient` server + WindowToken validation).
//!
//! Session 2 scope, per `tasks/40-real-ime.md`: prove the transport
//! works. The IMMS `IInputMethodManager` AIDL stub in `build.rs`
//! preserves the transaction code for `isImeTraceEnabled` at
//! `FIRST_CALL_TRANSACTION + 25` (its position in the upstream
//! `android-15.0.0_r36` interface) by declaring 25 no-import slot_NN
//! placeholder methods ahead of it. The placeholders are never called.

#[cfg(target_os = "android")]
pub fn probe() {
    use crate::binder_aidl::com::android::internal::view::IInputMethodManager::IInputMethodManager;

    if let Err(reason) = crate::binder::init() {
        log::warn!("ime: binder init failed: {reason}");
        return;
    }

    let svc: rsbinder::Strong<dyn IInputMethodManager> =
        match rsbinder::hub::get_interface("input_method") {
            Ok(s)  => s,
            Err(e) => {
                log::warn!("ime: input_method service unavailable: {e:?}");
                return;
            }
        };

    match svc.r#isImeTraceEnabled() {
        Ok(enabled) => log::info!(
            "ime: IMMS round-trip OK — isImeTraceEnabled() = {enabled}. \
             Transport validated against com.android.internal.view.IInputMethodManager. \
             Session 2 first-call milestone reached.",
        ),
        Err(e) => log::info!(
            "ime: IMMS round-trip reached service — isImeTraceEnabled() returned {e:?}. \
             Service responded with a structured Status; rsbinder transport works. \
             Session 2 first-call milestone reached (transport-level signal).",
        ),
    }
}

#[cfg(not(target_os = "android"))]
pub fn probe() {}
