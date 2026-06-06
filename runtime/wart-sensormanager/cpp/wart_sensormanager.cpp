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
// JavaVM: we pass a minimal FAKE JavaVM (kFakeVm above), not nullptr. The VM is only
// used by createEventQueue's poll thread to AttachCurrentThread (SensorManager.cpp
// getLooper, no null guard) — passing nullptr would SIGSEGV there. The fake VM's
// Attach returns JNI_OK with a null JNIEnv the thread never dereferences (no JNI in
// this native path), so EVERY ISensorManager path (getSensorList / createDirectChannel
// / createEventQueue) works with no real ART runtime and no platform-lib patch. The
// camera's EIS uses a direct channel (device-verified 28.8 fps); the fake VM also
// covers any future createEventQueue client.
//
// Build: soong cc_binary on a-03 (see Android.bp).

#include <android/frameworks/sensorservice/1.0/ISensorManager.h>
#include <sensorservicehidl/SensorManager.h>
#include <hidl/HidlTransportSupport.h>
#include <jni.h>
#include <log/log.h>

namespace {
// Minimal fake JavaVM. libsensorservicehidl SensorManager only uses the VM to
// AttachCurrentThread its createEventQueue poll thread "so pollAll doesn't crash if
// it calls into Java" (SensorManager.cpp getLooper). This native service never calls
// into Java, so the thread never dereferences the JNIEnv — Attach just has to return
// JNI_OK. Emulating that one slice of the JNI Invocation API (a few C function
// pointers) is better than passing nullptr (which crashes on createEventQueue) AND
// than patching the platform lib: it keeps ALL ISensorManager paths working with no
// real ART runtime.
jint fakeAttach(JavaVM*, JNIEnv** penv, void*) { if (penv) *penv = nullptr; return JNI_OK; }
jint fakeDetach(JavaVM*) { return JNI_OK; }
jint fakeGetEnv(JavaVM*, void** penv, jint) { if (penv) *penv = nullptr; return JNI_EDETACHED; }
jint fakeDestroy(JavaVM*) { return JNI_OK; }
const struct JNIInvokeInterface kFakeInvoke = {
    nullptr, nullptr, nullptr,
    fakeDestroy,  // DestroyJavaVM
    fakeAttach,   // AttachCurrentThread
    fakeDetach,   // DetachCurrentThread
    fakeGetEnv,   // GetEnv
    fakeAttach,   // AttachCurrentThreadAsDaemon
};
JavaVM kFakeVm = { &kFakeInvoke };
}  // namespace

using android::OK;
using android::sp;
using android::status_t;
using android::frameworks::sensorservice::V1_0::ISensorManager;
using android::frameworks::sensorservice::V1_0::implementation::SensorManager;
using android::hardware::configureRpcThreadpool;
using android::hardware::joinRpcThreadpool;

int main() {
    configureRpcThreadpool(4, true /*callerWillJoin*/);

    sp<ISensorManager> manager = new SensorManager(&kFakeVm);
    status_t st = manager->registerAsService();
    if (st != OK) {
        ALOGE("wart-sensormanager: registerAsService(ISensorManager) failed: %d", st);
        return 1;
    }
    ALOGI("wart-sensormanager: registered android.frameworks.sensorservice@1.0::ISensorManager");

    joinRpcThreadpool();
    return 0;
}
