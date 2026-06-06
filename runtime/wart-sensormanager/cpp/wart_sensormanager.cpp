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
// This is a THIN FAÇADE: the impl (libsensorservicehidl SensorManager) delegates
// everything to ::android::SensorManager::getInstanceForPackage() — i.e. the
// "sensorservice" binder (SensorManager.cpp:199). So it REQUIRES the standalone
// /system/bin/sensorservice running (the HAL owner + data source); it is not a
// replacement for it.
//
// JavaVM = nullptr RISK: mJavaVm is touched in exactly ONE path — createEventQueue
// -> getLooper() spawns a poll thread that does javaVm->AttachCurrentThread() with
// NO null guard (SensorManager.cpp ~166) -> SIGSEGV if vm is null. getSensorList /
// getDefaultSensor / createDirectChannel never touch it. The camera's EIS uses a
// DIRECT sensor channel, so null is safe for it (device-verified 28.8 fps). Any
// client that calls createEventQueue via this HIDL ISensorManager WOULD crash here.
// To harden: patch SensorManager.cpp getLooper to guard `if (mJavaVm != nullptr)`
// around Attach/DetachCurrentThread (safe — the poll thread never calls into Java in
// this native build) + rebuild libsensorservicehidl. wart-sensors (task 94) should
// use libsensor -> "sensorservice" directly, NOT this HIDL path, so it avoids it.
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
