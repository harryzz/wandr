// wart-sensormanager (task 93 + 94) — registers the frameworks-layer
// android.frameworks.sensorservice ISensorManager that system_server normally
// publishes (com_android_server_SystemServer.cpp: startSensorManager*Service), but
// which dies with ART. We publish BOTH transports system_server does:
//
//   • HIDL  @1.0::ISensorManager  (task 93) — the qcom CAMERA HAL's EIS (video
//     stabilization) reaches the gyro through this; without it the HAL SIGABRTs in
//     QCamera3HardwareInterface::startChannelLocked (device-confirmed). HIDL-only:
//     the vendor HAL is a hwbinder client and can't use the AIDL endpoint.
//
//   • AIDL  android.frameworks.sensorservice.ISensorManager/default (task 94) — the
//     wart Rust stack consumes THIS via rsbinder (the shared `wart-hal-sensors`
//     crate, used by the arbiter's sensor-driver + the host's guest-facing sensor
//     WIT). Registering it under --no-art lets the arbiter's existing task-77 sensor
//     path light up (auto-rotation / proximity / auto-brightness) and let us delete
//     the old `wart-sensors` direct-HAL daemon entirely.
//
// Both are THIN FAÇADES: each impl delegates to ::android::SensorManager::
// getInstanceForPackage() — the "sensorservice" binder. So this REQUIRES the
// standalone /system/bin/sensorservice running (the HAL owner + data source); it is
// not a replacement for it. SensorService multiplexes both endpoints + every client
// over the one single-client sensors HAL — exactly its job.
//
// JavaVM: we pass a minimal FAKE JavaVM (kFakeVm below), not nullptr — BOTH the HIDL
// and AIDL impls take a JavaVM* and AttachCurrentThread their createEventQueue poll
// thread (SensorManager.cpp / SensorManagerAidl.cpp getLooper, no null guard), so
// nullptr would SIGSEGV. The fake VM's Attach returns JNI_OK with a null JNIEnv the
// thread never dereferences (no JNI in this native path), so every ISensorManager
// path works with no real ART runtime and no platform-lib patch.
//
// Build: soong cc_binary on a-03 (see Android.bp).

#include <android/frameworks/sensorservice/1.0/ISensorManager.h>
#include <sensorservicehidl/SensorManager.h>
#include <hidl/HidlTransportSupport.h>
#include <jni.h>
#include <log/log.h>

// AIDL (task 94): the NDK SensorManagerAidl impl + the binder_ndk registration API.
#include <aidl/android/frameworks/sensorservice/ISensorManager.h>
#include <sensorserviceaidl/SensorManagerAidl.h>
#include <android/binder_manager.h>
#include <android/binder_process.h>

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
// AIDL (task 94) — alias to avoid the HIDL `ISensorManager` name clash above.
using AidlISensorManager = ::aidl::android::frameworks::sensorservice::ISensorManager;
using android::frameworks::sensorservice::implementation::SensorManagerAidl;

int main() {
    // ── HIDL endpoint (camera EIS, task 93) ──────────────────────────────────
    configureRpcThreadpool(4, true /*callerWillJoin*/);
    sp<ISensorManager> hidl = new SensorManager(&kFakeVm);
    status_t st = hidl->registerAsService();
    if (st != OK) {
        ALOGE("wart-sensormanager: registerAsService(HIDL ISensorManager) failed: %d", st);
        return 1;
    }
    ALOGI("wart-sensormanager: registered android.frameworks.sensorservice@1.0::ISensorManager");

    // ── AIDL endpoint (wart Rust stack via wart-hal-sensors, task 94) ─────────
    // Mirrors system_server's startSensorManagerAidlService
    // (com_android_server_SystemServer.cpp): SharedRefBase::make + addService under
    // "<descriptor>/default". Runs on its own libbinder_ndk thread pool; the HIDL
    // hwbinder pool and the AIDL binder pool are independent transports in one
    // process (as system_server does).
    ABinderProcess_setThreadPoolMaxThreadCount(4);
    std::shared_ptr<SensorManagerAidl> aidl =
            ndk::SharedRefBase::make<SensorManagerAidl>(&kFakeVm);
    const std::string instance = std::string() + AidlISensorManager::descriptor + "/default";
    binder_exception_t err =
            AServiceManager_addService(aidl->asBinder().get(), instance.c_str());
    if (err != EX_NONE) {
        ALOGE("wart-sensormanager: AServiceManager_addService(%s) failed: %d",
              instance.c_str(), err);
        return 1;
    }
    ALOGI("wart-sensormanager: registered AIDL %s", instance.c_str());
    ABinderProcess_startThreadPool();

    // Block on the HIDL pool (the AIDL pool runs on its own threads). Never returns.
    joinRpcThreadpool();
    return 0;
}
