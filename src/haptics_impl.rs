use crate::bindings::my::skiko_gfx::haptics::{Feedback, Host};
use std::fs;
use std::path::Path;

/// Try to vibrate for `ms` milliseconds. Returns true if a write succeeded.
///
/// Probes (in order):
///  1. legacy `/sys/class/timed_output/vibrator/enable` — write the duration
///  2. modern `/sys/class/leds/vibrator/{duration,activate}` — write duration
///     then "1" to activate
/// Both require the calling UID to have write access, which on stock Android
/// means the `system` uid (the Vibrator HAL runs as system). Without root or
/// a platform-signed app, both writes will fail with EACCES — we return false.
fn try_vibrate(ms: u32) -> bool {
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

/// Map semantic feedback to a duration in milliseconds, matching the rough
/// feel of Android's HapticFeedbackConstants.
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
        try_vibrate(feedback_duration(feedback))
    }

    fn vibrate_ms(&mut self, duration_ms: u32) -> bool {
        let clamped = duration_ms.clamp(1, 1000);
        try_vibrate(clamped)
    }
}
