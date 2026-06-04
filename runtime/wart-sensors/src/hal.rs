//! `dlopen` wrapper for `libwart_sensors_hal.so` — the C++ HIDL shim over
//! `android.hardware.sensors@1.0::ISensors`. Mirrors how `wart-host` loads
//! `libsf_surface.so`: the only native piece (HIDL uses hwbinder, unreachable from
//! rsbinder), kept thin — it just opens the HAL, enables sensors, and hands raw
//! events back; all processing (orientation, fusion) stays in Rust.
//!
//! C ABI (must match `cpp/wart_sensors_hal.cpp`):
//! ```c
//! typedef struct { int32_t type; int64_t timestamp_ns; float x, y, z; } WartSensorEvent;
//! int wart_sensors_open(void);                                  // 0 on success
//! int wart_sensors_enable(int32_t type, int64_t period_ns, int enable); // 0 ok
//! int wart_sensors_poll(WartSensorEvent* out, int max);         // count (blocks)
//! ```

use std::ffi::{c_int, c_void, CString};
use std::os::raw::c_char;

/// One sensor sample. `type` is the android sensor type int (1=accel, 8=proximity,
/// …); `x/y/z` are the event vector (accel m/s², proximity in `x`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct WartSensorEvent {
    pub stype: i32,
    pub timestamp_ns: i64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const TYPE_ACCEL: i32 = 1;
/// Used by the upcoming proximity-under-ART-off wiring (task 78 follow-on).
#[allow(dead_code)]
pub const TYPE_PROXIMITY: i32 = 8;
/// Fused screen-rotation sensor (0/1/2/3 in event `x`). HAL-provided on Qualcomm
/// SSC devices (computed on the sensor core) — preferred over the accel fallback.
pub const TYPE_DEVICE_ORIENTATION: i32 = 27;

type OpenFn = unsafe extern "C" fn() -> c_int;
type EnableFn = unsafe extern "C" fn(i32, i64, c_int) -> c_int;
type PollFn = unsafe extern "C" fn(*mut WartSensorEvent, c_int) -> c_int;

pub struct SensorHal {
    _handle: *mut c_void,
    enable: EnableFn,
    poll: PollFn,
}

// The dlopen'd library lives for the process lifetime; the fn pointers are Send.
unsafe impl Send for SensorHal {}

impl SensorHal {
    /// dlopen the shim (default `/data/local/tmp/libwart_sensors_hal.so`, override
    /// with `WART_SENSORS_HAL_LIB`), resolve the symbols, and call
    /// `wart_sensors_open`. Returns an error string on any failure.
    pub fn load() -> Result<Self, String> {
        let path = std::env::var("WART_SENSORS_HAL_LIB")
            .unwrap_or_else(|_| "/data/local/tmp/libwart_sensors_hal.so".to_string());
        let c_path = CString::new(path.clone()).map_err(|e| e.to_string())?;
        let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW) };
        if handle.is_null() {
            return Err(format!("dlopen({path}) failed: {}", dlerror()));
        }
        let open: OpenFn = unsafe { sym(handle, "wart_sensors_open")? };
        let enable: EnableFn = unsafe { sym(handle, "wart_sensors_enable")? };
        let poll: PollFn = unsafe { sym(handle, "wart_sensors_poll")? };
        let rc = unsafe { open() };
        if rc != 0 {
            return Err(format!("wart_sensors_open() = {rc} (HAL unreachable?)"));
        }
        Ok(Self { _handle: handle, enable, poll })
    }

    /// Enable/disable a sensor by android type at `period_ns`. Returns Ok on 0.
    pub fn enable(&self, stype: i32, period_ns: i64, on: bool) -> Result<(), i32> {
        let rc = unsafe { (self.enable)(stype, period_ns, on as c_int) };
        if rc == 0 {
            Ok(())
        } else {
            Err(rc)
        }
    }

    /// Block for the next batch of events (up to `buf.len()`); returns the slice
    /// that was filled. An empty slice means the HAL returned no events this call.
    pub fn poll<'a>(&self, buf: &'a mut [WartSensorEvent]) -> &'a [WartSensorEvent] {
        let n = unsafe { (self.poll)(buf.as_mut_ptr(), buf.len() as c_int) };
        let n = if n < 0 { 0 } else { n as usize };
        &buf[..n.min(buf.len())]
    }
}

unsafe fn sym<T>(handle: *mut c_void, name: &str) -> Result<T, String> {
    let c_name = CString::new(name).map_err(|e| e.to_string())?;
    let p = libc::dlsym(handle, c_name.as_ptr());
    if p.is_null() {
        return Err(format!("dlsym({name}) failed: {}", dlerror()));
    }
    Ok(std::mem::transmute_copy::<*mut c_void, T>(&p))
}

fn dlerror() -> String {
    let p: *const c_char = unsafe { libc::dlerror() };
    if p.is_null() {
        return "unknown".to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}
