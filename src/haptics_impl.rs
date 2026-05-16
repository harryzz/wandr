use crate::bindings::my::skiko_gfx::haptics::{Feedback, Host};
use std::fs;
use std::path::Path;

// ── Binder path (Android, stable AIDL HAL) ───────────────────────────────────
//
// Reaches /vendor/bin/hw/android.hardware.vibrator-service via libbinder_ndk.
// The service is registered as "android.hardware.vibrator.IVibrator/default"
// in servicemanager on Android 11+. SELinux on stock devices may block this
// from an untrusted_app domain; `setenforce 0` is required during dev.

#[cfg(target_os = "android")]
mod binder_path {
    use crate::binder_aidl::android::hardware::vibrator::{
        Effect::Effect, EffectStrength::EffectStrength,
        IVibrator::IVibrator,
        IVibratorCallback::{IVibratorCallback, IVibratorCallbackAsyncService, BnVibratorCallback},
    };
    use std::sync::OnceLock;

    // The vibrator HAL methods take a `@nullable` IVibratorCallback in AIDL,
    // but rsbinder-aidl 0.7.0 doesn't translate that to Option<&Strong>. So
    // we pass a no-op callback. The callback gets called once when the
    // vibration completes; we don't care, return Ok(()).
    struct NopCallback;
    impl rsbinder::Interface for NopCallback {}
    #[async_trait::async_trait]
    impl IVibratorCallbackAsyncService for NopCallback {
        async fn r#onComplete(&self) -> rsbinder::status::Result<()> { Ok(()) }
    }

    // BinderAsyncRuntime is required by new_async_binder. NopCallback::onComplete
    // returns Poll::Ready immediately on the first poll, so a trivial executor
    // suffices — no tokio or other runtime needed. Polling once and panicking
    // on Pending is correct because any other future on this runtime would be
    // a bug.
    struct TrivialRuntime;
    impl rsbinder::BinderAsyncRuntime for TrivialRuntime {
        fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
            use std::task::{Context, Poll, Waker, RawWaker, RawWakerVTable};
            fn raw() -> RawWaker {
                fn no_op(_: *const ()) {}
                fn clone(_: *const ()) -> RawWaker { raw() }
                const VT: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
                RawWaker::new(std::ptr::null(), &VT)
            }
            let waker = unsafe { Waker::from_raw(raw()) };
            let mut cx = Context::from_waker(&waker);
            let mut fut = std::pin::pin!(f);
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => v,
                Poll::Pending  => panic!("TrivialRuntime: future must be Ready on first poll"),
            }
        }
    }

    static VIB: OnceLock<Option<rsbinder::Strong<dyn IVibrator>>> = OnceLock::new();
    static CB:  OnceLock<rsbinder::Strong<dyn IVibratorCallback>> = OnceLock::new();

    fn service() -> Option<&'static rsbinder::Strong<dyn IVibrator>> {
        VIB.get_or_init(|| {
            rsbinder::hub::get_interface::<dyn IVibrator>(
                "android.hardware.vibrator.IVibrator/default"
            ).ok()
        }).as_ref()
    }

    fn callback() -> &'static rsbinder::Strong<dyn IVibratorCallback> {
        CB.get_or_init(|| BnVibratorCallback::new_async_binder(NopCallback, TrivialRuntime))
    }

    pub fn vibrate_ms(ms: u32) -> bool {
        let Some(svc) = service() else { return false };
        svc.r#on(ms as i32, callback()).is_ok()
    }

    pub fn perform(f: super::Feedback) -> bool {
        let Some(svc) = service() else { return false };
        let (effect, strength) = map_feedback(f);
        // Returns the actual duration the HAL chose, or EX_UNSUPPORTED if the
        // device's vibrator doesn't implement this effect. Either way we
        // treat "no error" as success; the caller didn't ask for a duration.
        match svc.r#perform(effect, strength, callback()) {
            Ok(_) => true,
            Err(_) => {
                // Effect unsupported on this device — fall back to a raw
                // timed vibration of approximately the right duration.
                vibrate_ms(super::feedback_duration(f))
            }
        }
    }

    fn map_feedback(f: super::Feedback) -> (Effect, EffectStrength) {
        // Mirrors the framework's HapticFeedbackConstants → VibrationEffect
        // mapping in services/core/java/com/android/server/vibrator/.
        match f {
            super::Feedback::Tap         => (Effect::TICK,         EffectStrength::LIGHT),
            super::Feedback::VirtualKey  => (Effect::TICK,         EffectStrength::MEDIUM),
            super::Feedback::Click       => (Effect::CLICK,        EffectStrength::MEDIUM),
            super::Feedback::LongPress   => (Effect::HEAVY_CLICK,  EffectStrength::STRONG),
            super::Feedback::DoubleClick => (Effect::DOUBLE_CLICK, EffectStrength::MEDIUM),
        }
    }
}

// ── Sysfs fallback path ──────────────────────────────────────────────────────
//
// On Android the binder path covers the common case. Sysfs is kept for two
// niche scenarios: (1) custom rooted ROMs that have sysfs vibrator nodes but
// no AIDL HAL registered, (2) non-Android Linux devices where we cross-build
// the host. Both writes require write access to the nodes; on most devices
// EACCES, in which case we return false and the caller sees no buzz.

fn try_vibrate_sysfs(ms: u32) -> bool {
    let ms_str = ms.to_string();

    let legacy = Path::new("/sys/class/timed_output/vibrator/enable");
    if legacy.exists() {
        if fs::write(legacy, &ms_str).is_ok() {
            return true;
        }
    }

    let leds_dir = Path::new("/sys/class/leds/vibrator");
    if leds_dir.exists() {
        let dur = leds_dir.join("duration");
        let act = leds_dir.join("activate");
        if fs::write(&dur, &ms_str).is_ok() && fs::write(&act, "1").is_ok() {
            return true;
        }
    }

    false
}

fn feedback_duration(f: Feedback) -> u32 {
    match f {
        Feedback::Tap         => 10,
        Feedback::VirtualKey  => 10,
        Feedback::Click       => 10,
        Feedback::LongPress   => 40,
        Feedback::DoubleClick => 20,
    }
}

impl Host for crate::HostState {
    fn perform(&mut self, feedback: Feedback) -> bool {
        #[cfg(target_os = "android")]
        if binder_path::perform(feedback) { return true; }
        try_vibrate_sysfs(feedback_duration(feedback))
    }

    fn vibrate_ms(&mut self, duration_ms: u32) -> bool {
        let clamped = duration_ms.clamp(1, 1000);
        #[cfg(target_os = "android")]
        if binder_path::vibrate_ms(clamped) { return true; }
        try_vibrate_sysfs(clamped)
    }
}
