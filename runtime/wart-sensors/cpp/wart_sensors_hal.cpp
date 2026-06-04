// wart_sensors_hal — task 85. C ABI shim over android.hardware.sensors@1.0::ISensors
// (HIDL), dlopen'd by the Rust wart-sensors daemon (HIDL uses hwbinder, unreachable
// from rsbinder, so this hop must be C++). Thin by design: open the HAL, enable a
// sensor by android type, poll raw events — all processing (orientation, etc.) is in
// Rust. Proven under ART-off (the framework SensorService is gone but the HAL + all
// its HAL-fused sensors, incl. DEVICE_ORIENTATION 27, survive). See wart_sensors_probe.

#define LOG_TAG "wart_sensors"
#include <android/hardware/sensors/1.0/ISensors.h>
#include <android/hardware/sensors/1.0/types.h>
#include <hidl/HidlSupport.h>
#include <log/log.h>

#include <cstdint>
#include <map>

using android::sp;
using android::hardware::hidl_vec;
using android::hardware::sensors::V1_0::Event;
using android::hardware::sensors::V1_0::ISensors;
using android::hardware::sensors::V1_0::Result;
using android::hardware::sensors::V1_0::SensorInfo;

namespace {
sp<ISensors> g_sensors;
std::map<int32_t, int32_t> g_type_handle; // android sensor type → first handle
}  // namespace

// Must match the Rust `WartSensorEvent` (#[repr(C)]). `x/y/z` are the event vector;
// for single-value sensors (proximity, device-orientation, light) the value is in
// `x` (the HIDL Event union overlaps scalar with vec3.x).
struct WartSensorEvent {
    int32_t type;
    int64_t timestamp_ns;
    float x, y, z;
};

extern "C" int wart_sensors_open(void) {
    g_sensors = ISensors::getService();
    if (g_sensors == nullptr) {
        ALOGE("ISensors@1.0::getService() returned null");
        return 1;
    }
    g_sensors->getSensorsList([](const hidl_vec<SensorInfo>& list) {
        for (const SensorInfo& s : list) {
            int32_t t = static_cast<int32_t>(s.type);
            g_type_handle.emplace(t, s.sensorHandle); // keep the first handle per type
        }
    });
    ALOGI("wart_sensors_open: %zu sensor types", g_type_handle.size());
    return 0;
}

extern "C" int wart_sensors_enable(int32_t type, int64_t period_ns, int enable) {
    if (g_sensors == nullptr) return -1;
    auto it = g_type_handle.find(type);
    if (it == g_type_handle.end()) return -2; // sensor type not present on this device
    int32_t handle = it->second;
    if (enable) {
        g_sensors->batch(handle, period_ns, 0);
        Result r = g_sensors->activate(handle, true);
        ALOGI("enable type=%d handle=%d → %d", type, handle, static_cast<int>(r));
        return r == Result::OK ? 0 : -3;
    }
    g_sensors->activate(handle, false);
    return 0;
}

extern "C" int wart_sensors_poll(WartSensorEvent* out, int max) {
    if (g_sensors == nullptr || out == nullptr || max <= 0) return -1;
    int n = 0;
    g_sensors->poll(max, [&](Result res, const hidl_vec<Event>& events,
                             const hidl_vec<SensorInfo>&) {
        if (res != Result::OK) return;
        for (const Event& e : events) {
            if (n >= max) break;
            out[n].type = static_cast<int32_t>(e.sensorType);
            out[n].timestamp_ns = e.timestamp;
            out[n].x = e.u.vec3.x; // == u.scalar for single-value sensors (union)
            out[n].y = e.u.vec3.y;
            out[n].z = e.u.vec3.z;
            n++;
        }
    });
    return n;
}
