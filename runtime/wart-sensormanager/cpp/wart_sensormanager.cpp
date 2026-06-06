// wart-sensormanager (task 93) — registers android.frameworks.sensorservice@1.0
// ::ISensorManager under --no-art, which system_server normally does
// (com_android_server_SystemServer.cpp: `new SensorManager(vm)`). Without it the
// qcom camera HAL's EIS (video stabilization) waits forever on the gyro via
// ISensorManager and SIGABRTs in QCamera3HardwareInterface::startChannelLocked
// (device-confirmed). The impl (libsensorservicehidl `SensorManager`) wraps the C++
// SensorService (the standalone /system/bin/sensorservice), so run BOTH: sensorservice
// (owns the sensors HAL + registers "sensorservice") and this (registers the HIDL
// ISensorManager on top).
//
// JavaVM = nullptr: it's used ONLY to attach createEventQueue's poll thread to the
// JVM (SensorManager.cpp getLooper). Native clients that use direct sensor channels
// (typical for camera EIS) never hit it. If a client calls createEventQueue, the
// poll thread would deref the null VM — handled by the matching SensorManager null
// guard patch if that path is exercised.
//
// Build: soong cc_binary on a-03 (see Android.bp).

#include <android/frameworks/sensorservice/1.0/ISensorManager.h>
#include <sensorservicehidl/SensorManager.h>
#include <hidl/HidlTransportSupport.h>
#include <log/log.h>

using android::OK;
using android::sp;
using android::status_t;
using android::frameworks::sensorservice::V1_0::ISensorManager;
using android::frameworks::sensorservice::V1_0::implementation::SensorManager;
using android::hardware::configureRpcThreadpool;
using android::hardware::joinRpcThreadpool;

int main() {
    configureRpcThreadpool(4, true /*callerWillJoin*/);

    sp<ISensorManager> manager = new SensorManager(nullptr /*JavaVM*/);
    status_t st = manager->registerAsService();
    if (st != OK) {
        ALOGE("wart-sensormanager: registerAsService(ISensorManager) failed: %d", st);
        return 1;
    }
    ALOGI("wart-sensormanager: registered android.frameworks.sensorservice@1.0::ISensorManager");

    joinRpcThreadpool();
    return 0;
}
