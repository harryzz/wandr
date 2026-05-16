// rsbinder ProcessState init — one-shot for the process lifetime.
//
// The crate's own `init_default()` already uses an internal OnceLock,
// but it panics on failure. We wrap it so callers get a Result and the
// host degrades gracefully (sysfs fallback for haptics, return-false
// for lights) when /dev/binder isn't reachable.

#[cfg(target_os = "android")]
pub fn init() -> Result<(), &'static str> {
    use std::sync::OnceLock;
    static RESULT: OnceLock<Result<(), &'static str>> = OnceLock::new();
    *RESULT.get_or_init(|| {
        if !std::path::Path::new("/dev/binder").exists() {
            return Err("/dev/binder not present");
        }
        match std::panic::catch_unwind(|| {
            rsbinder::ProcessState::init_default();
            rsbinder::ProcessState::start_thread_pool();
        }) {
            Ok(_)  => Ok(()),
            Err(_) => Err("rsbinder init panicked"),
        }
    })
}

#[cfg(not(target_os = "android"))]
pub fn init() -> Result<(), &'static str> { Ok(()) }
