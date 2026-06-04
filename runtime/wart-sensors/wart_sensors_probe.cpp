// wart_sensors_probe — task 85 keystone de-risk. Read the sensor HAL DIRECTLY
// (android.hardware.sensors@1.0::ISensors, HIDL) under ART-off, bypassing the
// dead framework SensorService/ISensorManager. Proves a non-SensorService process
// can open ISensors, enumerate sensors, and poll accelerometer events when the
// Java framework is stopped — the foundation for the wart sensor HAL shim.
//
// Build: soong cc_binary on a-03 (see Android.bp). Run under --no-art via
// wart-launch (uid system) + setenforce 0.

#define LOG_TAG "wart_sensors"
#include <android/hardware/sensors/1.0/ISensors.h>
#include <android/hardware/sensors/1.0/types.h>
#include <hidl/HidlSupport.h>
#include <log/log.h>

#include <cstdio>
#include <cstdint>

using android::sp;
using android::hardware::hidl_vec;
using android::hardware::sensors::V1_0::Event;
using android::hardware::sensors::V1_0::ISensors;
using android::hardware::sensors::V1_0::Result;
using android::hardware::sensors::V1_0::SensorInfo;
using android::hardware::sensors::V1_0::SensorType;

static constexpr int32_t TYPE_ACCEL = 1;   // SensorType::ACCELEROMETER
static constexpr int32_t TYPE_PROX = 8;    // SensorType::PROXIMITY
static constexpr int32_t TYPE_ORIENT = 27; // SensorType::DEVICE_ORIENTATION (fused)

int main() {
    sp<ISensors> sensors = ISensors::getService();
    if (sensors == nullptr) {
        ALOGE("ISensors@1.0::getService() returned null");
        printf("FAIL: no ISensors@1.0 (hwservicemanager / HAL unreachable)\n");
        return 1;
    }
    ALOGI("got ISensors@1.0");
    printf("OK: ISensors@1.0 acquired\n");

    int32_t accel = -1, prox = -1, orient = -1;
    sensors->getSensorsList([&](const hidl_vec<SensorInfo>& list) {
        printf("sensor count = %zu\n", list.size());
        for (const SensorInfo& s : list) {
            printf("  handle=%d type=%d name=%s\n", s.sensorHandle,
                   static_cast<int>(s.type), s.name.c_str());
            int32_t t = static_cast<int32_t>(s.type);
            if (t == TYPE_ACCEL && accel < 0) accel = s.sensorHandle;
            if (t == TYPE_PROX && prox < 0) prox = s.sensorHandle;
            if (t == TYPE_ORIENT && orient < 0) orient = s.sensorHandle;
        }
    });
    printf("resolved: accel=%d prox=%d device_orientation=%d\n", accel, prox, orient);
    if (accel < 0) {
        printf("FAIL: no accelerometer in HAL list\n");
        return 1;
    }

    // Enable accel at ~10 Hz and poll a handful of events.
    Result br = sensors->batch(accel, 100000000 /*100 ms*/, 0);
    Result ar = sensors->activate(accel, true);
    printf("accel batch=%d activate=%d — polling (tilt the device)…\n",
           static_cast<int>(br), static_cast<int>(ar));

    int got = 0;
    for (int i = 0; i < 30 && got < 20; i++) {
        sensors->poll(16, [&](Result res, const hidl_vec<Event>& events,
                              const hidl_vec<SensorInfo>&) {
            if (res != Result::OK) {
                printf("poll result=%d\n", static_cast<int>(res));
                return;
            }
            for (const Event& e : events) {
                if (static_cast<int32_t>(e.sensorType) == TYPE_ACCEL) {
                    printf("accel x=%.2f y=%.2f z=%.2f\n", e.u.vec3.x, e.u.vec3.y,
                           e.u.vec3.z);
                    got++;
                }
            }
        });
    }
    sensors->activate(accel, false);
    printf("DONE: read %d accel samples directly from the HAL under ART-off\n", got);
    return got > 0 ? 0 : 2;
}
