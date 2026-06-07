// wart-framework-shim — the designed framework-shim for `--no-art` operation
// (task 96). Replaces the ad-hoc `wart-activityms` stub.
//
// Under `--no-art` the Java system_server (which hosts ActivityManager,
// PermissionController, SensorPrivacyManager, the package manager, …) is gone, but
// the native survivors (audioserver / cameraserver / sensorservice / the codec
// stack) still resolve a handful of its binder services during bring-up and on the
// stream/camera start paths. This shim registers exactly that minimal set and
// answers benignly, so nothing wedges. It is started and confirmed serving BEFORE
// audioserver / sensorservice / cameraserver are (re)started (run-hybrid-stack.sh
// native+shim layer) — the "shim-first" half of task 96.
//
// ── The service set is DERIVED from the daemon sources (not discovered by hang) ──
// (vendored under runtime/wart-host/vendor/; verified 2026-06-07.)
//
// Hard blockers — `waitForService` (blocks forever until registered; some FATAL):
//   • "activity"        audioserver AudioPolicyService::UidPolicy ActivityManager via
//                       BinderProxy::waitServiceOrDie → waitForService, with
//                       LOG_ALWAYS_FATAL_IF(null) (BinderProxy.h:59). MUST implement
//                       the IActivityManager interface (descriptor must match) → the
//                       precise ActivityStub below.
//   • "sensor_privacy"  SensorPrivacyManager::getService → waitForService
//                       (frameworks-native libs/sensorprivacy/SensorPrivacyManager.cpp).
//   • "package_native"  SensorService::enable (SensorService.cpp:144) and
//                       MediaCodec::connectFormatShaper (MediaCodec.cpp:2681).
//   • "processinfo"     media/utils/ProcessInfo.cpp:55/90 (codec/camera eviction
//                       priority). Replies are sized int[] arrays read as RAW blocks
//                       → the custom ProcessInfoStub below (the generic 0-reply can't
//                       serve it; the out-array length must echo the input pid count).
//
// Soft paths — `checkService`/`getService` (return null immediately, but a wrong/
// absent answer fail-closes or spins a retry-with-sleep loop):
//   • "scheduling_policy"  AAudio MMAP start → requestPriority spins on
//                          checkService("scheduling_policy") when system_server is gone.
//   • "permission"         MmapThread::start → checkAttributionSourcePackage →
//                          PermissionController::getService loops checkService 10s.
//   • "permission_checker" cameraserver AttributionAndPermissionUtils →
//                          PermissionChecker getService loop.
//   • "media.camera.proxy" CameraServiceProxyWrapper::isCameraDisabled fail-CLOSES
//                          (returns true = camera disabled) if the binder is null.
//
// Every uid is reported foreground (PROCESS_STATE_TOP) and every permission granted —
// correct for wart (single-user, privileged). NOT a real ActivityManager; later it
// can be backed by the arbiter's real foreground/UID state.
//
// C++/libbinder (NOT rsbinder): rsbinder's addService can't register with the real
// Android servicemanager (reply-parse BadParcelable); C++ libbinder addService is
// proven under --no-art. Built on a-03; launch via wart-launch (uid system) so it can
// register a system service name. Set WART_SHIM_TRACE=1 to log every transaction
// (off by default — the per-call log was task-94 dissection scaffolding that spams
// the log in steady state). See docs/artless-native-service-model.md, task 96.
//
// The transaction codes + parcel layout mirror frameworks/native
// libs/binder/IActivityManager.{h,cpp} (the native client audioserver uses): binder
// dispatches by integer code (not method name), so only the descriptor, the codes,
// and the per-method parcel format must match — they do.

#include <binder/Binder.h>  // class BBinder
#include <binder/IActivityManager.h>
#include <binder/IInterface.h>
#include <binder/IPCThreadState.h>
#include <binder/IServiceManager.h>
#include <binder/Parcel.h>
#include <binder/ProcessState.h>
#include <utils/Errors.h>
#include <utils/String16.h>
#include <utils/StrongPointer.h>
#include <log/log.h>
#include <utils/String8.h>

#include <cstdlib>  // getenv

using namespace android;

namespace {

// Per-transaction tracing — OFF unless WART_SHIM_TRACE is set in the environment.
// (Task-94 used it to dissect exactly what the camera HAL asks each stub; in steady
// `--no-art` it is pure log spam.)
static const bool kTrace = (getenv("WART_SHIM_TRACE") != nullptr);

// ProcessStateEnum.aidl: UNKNOWN=-1, PERSISTENT=0, PERSISTENT_UI=1, TOP=2. Reporting
// TOP/foreground for every uid means audio focus / record-privacy never restrict a
// wart process under --no-art.
constexpr int32_t PROCESS_STATE_TOP = 2;
// PackageManager.PERMISSION_GRANTED.
constexpr int32_t PERMISSION_GRANTED = 0;

// The interface descriptor, hardcoded (a-03's libbinder doesn't export the
// IActivityManager::descriptor static — it doesn't compile IActivityManager.cpp).
// This is the exact string IMPLEMENT_META_INTERFACE(ActivityManager, …) uses, which
// is what the native client writes in writeInterfaceToken — the only thing that must
// match. The transaction-code enum (IActivityManager::*_TRANSACTION) is header-only
// (compile-time constants), so it needs no libbinder symbol.
static const String16 kDescriptor("android.app.IActivityManager");

class ActivityStub : public BBinder {
public:
    // Report the IActivityManager descriptor so a client interface_cast resolves.
    const String16& getInterfaceDescriptor() const override {
        return kDescriptor;
    }

    status_t onTransact(uint32_t code, const Parcel& data, Parcel* reply,
                        uint32_t flags) override {
        if (kTrace) {
            ALOGI("WART-SHIM-TRACE activity code=%u flags=%u callingPid=%d callingUid=%d",
                  code, flags, IPCThreadState::self()->getCallingPid(),
                  IPCThreadState::self()->getCallingUid());
        }
        // Every IActivityManager call writes the interface token first; consume +
        // verify it (the client wrote the same descriptor).
        if (!data.enforceInterface(kDescriptor)) {
            return BAD_TYPE;
        }
        switch (code) {
            // openContentUri → int. Client: readExceptionCode, then (if ok) readInt32
            // where nonzero = success+fd. Return 0 = no fd (audio doesn't use it).
            case IActivityManager::OPEN_CONTENT_URI_TRANSACTION:
                reply->writeNoException();
                reply->writeInt32(0);
                return NO_ERROR;

            // void methods → reply is just writeNoException().
            case IActivityManager::REGISTER_UID_OBSERVER_TRANSACTION:
            case IActivityManager::UNREGISTER_UID_OBSERVER_TRANSACTION:
            case IActivityManager::ADD_UID_TO_OBSERVER_TRANSACTION:
            case IActivityManager::REMOVE_UID_FROM_OBSERVER_TRANSACTION:
                reply->writeNoException();
                return NO_ERROR;

            // registerUidObserverForUids → IBinder. Client reads: exc, strongBinder,
            // exc. Hand back a null token (audioserver's UidPolicy uses the plain
            // registerUidObserver above, not this).
            case IActivityManager::REGISTER_UID_OBSERVER_FOR_UIDS_TRANSACTION:
                reply->writeNoException();
                reply->writeStrongBinder(nullptr);
                reply->writeNoException();
                return NO_ERROR;

            // isUidActive → boolean (writeInt32 1/0). Everything is active.
            case IActivityManager::IS_UID_ACTIVE_TRANSACTION:
                reply->writeNoException();
                reply->writeInt32(1);
                return NO_ERROR;

            // getUidProcessState → int.
            case IActivityManager::GET_UID_PROCESS_STATE_TRANSACTION:
                reply->writeNoException();
                reply->writeInt32(PROCESS_STATE_TOP);
                return NO_ERROR;

            // checkPermission → int (0 = granted).
            case IActivityManager::CHECK_PERMISSION_TRANSACTION:
                reply->writeNoException();
                reply->writeInt32(PERMISSION_GRANTED);
                return NO_ERROR;

            // FGS API logging is oneway — no reply is read, but be tidy if present.
            case IActivityManager::LOG_FGS_API_BEGIN_TRANSACTION:
            case IActivityManager::LOG_FGS_API_END_TRANSACTION:
            case IActivityManager::LOG_FGS_API_STATE_CHANGED_TRANSACTION:
                if (reply != nullptr) {
                    reply->writeNoException();
                }
                return NO_ERROR;

            default:
                return BBinder::onTransact(code, data, reply, flags);
        }
    }
};

// Generic permissive stub for the system_server services native survivors block on /
// query that only need to (a) exist so their waitForService()/checkService() returns,
// and (b) answer benignly: it ignores the request args and replies `noException` + a
// zero — which a client reads as false / 0 / null for bool/int/object returns (e.g.
// isSensorPrivacyEnabled → false = privacy OFF = mic allowed; isCameraDisabled →
// false = camera ENABLED), and which void clients ignore. Each instance carries its
// own descriptor so a client interface_cast resolves.
class GenericStub : public BBinder {
public:
    explicit GenericStub(const char* descriptor) : mDescriptor(descriptor) {}
    const String16& getInterfaceDescriptor() const override { return mDescriptor; }
    status_t onTransact(uint32_t code, const Parcel& /*data*/, Parcel* reply,
                        uint32_t flags) override {
        if (kTrace) {
            ALOGI("WART-SHIM-TRACE %s code=%u flags=%u callingPid=%d callingUid=%d",
                  String8(mDescriptor).c_str(), code, flags,
                  IPCThreadState::self()->getCallingPid(),
                  IPCThreadState::self()->getCallingUid());
        }
        if (reply != nullptr) {
            reply->writeNoException();
            reply->writeInt32(0);
        }
        return NO_ERROR;
    }
private:
    String16 mDescriptor;
};

// IProcessInfoService stub. ProcessInfo (media/utils, codec/camera eviction priority)
// queries `processinfo` for the oom-priority of clients; without system_server the
// query times out and the operation fails. The GenericStub can't serve this — the
// replies are sized int[] arrays the client reads as RAW blocks
// (frameworks/native/libs/binder/IProcessInfoService.cpp BpProcessInfoService):
// exception(0), then for each out-array `writeInt32(len)` + `len` int32s, then a
// trailing status int32 — and `len` MUST equal the input pid count or the client
// returns NOT_ENOUGH_DATA. We echo `length` back, reporting every pid as
// PROCESS_STATE_TOP with oom score 0 (single privileged client; the exact priority
// only matters when arbitrating eviction between camera clients).
class ProcessInfoStub : public BBinder {
public:
    // Codes from IProcessInfoService.h (header-only enum).
    enum {
        GET_PROCESS_STATES_FROM_PIDS = IBinder::FIRST_CALL_TRANSACTION,       // 1
        GET_PROCESS_STATES_AND_OOM_SCORES_FROM_PIDS,                          // 2
    };
    const String16& getInterfaceDescriptor() const override { return mDescriptor; }
    status_t onTransact(uint32_t code, const Parcel& data, Parcel* reply,
                        uint32_t flags) override {
        if (kTrace) {
            ALOGI("WART-SHIM-TRACE processinfo code=%u flags=%u callingPid=%d callingUid=%d",
                  code, flags, IPCThreadState::self()->getCallingPid(),
                  IPCThreadState::self()->getCallingUid());
        }
        switch (code) {
            case GET_PROCESS_STATES_FROM_PIDS:
            case GET_PROCESS_STATES_AND_OOM_SCORES_FROM_PIDS: {
                if (!data.enforceInterface(getInterfaceDescriptor())) {
                    return BAD_TYPE;
                }
                // writeInt32Array(length, pids) wrote the length first.
                int32_t length = data.readInt32();
                if (length < 0) length = 0;
                reply->writeNoException();
                // out int[] states
                reply->writeInt32(length);
                for (int32_t i = 0; i < length; i++) reply->writeInt32(PROCESS_STATE_TOP);
                if (code == GET_PROCESS_STATES_AND_OOM_SCORES_FROM_PIDS) {
                    // out int[] scores
                    reply->writeInt32(length);
                    for (int32_t i = 0; i < length; i++) reply->writeInt32(0);
                }
                reply->writeInt32(NO_ERROR);  // trailing status the client returns
                return NO_ERROR;
            }
            default:
                return BBinder::onTransact(code, data, reply, flags);
        }
    }
private:
    String16 mDescriptor{"android.os.IProcessInfoService"};
};

}  // namespace

int main() {
    sp<ProcessState> ps = ProcessState::self();
    ps->startThreadPool();

    sp<IServiceManager> sm = defaultServiceManager();

    // The precise IActivityManager stub (the audioserver wedge — FATAL if absent).
    status_t st = sm->addService(String16("activity"), sp<ActivityStub>::make());
    if (st != OK) {
        ALOGE("wart-framework-shim: addService(activity) failed: %d", st);
        return 1;
    }
    ALOGI("wart-framework-shim: registered 'activity' (IActivityManager)");

    // The remaining services native survivors block on / query under --no-art.
    // (Derived from the daemon sources — see the header comment. Add names here only
    // with a source citation, never by "discover-a-missing-stub-by-hang".)
    struct { const char* name; const char* descriptor; } generics[] = {
        {"sensor_privacy", "android.hardware.ISensorPrivacyManager"},
        {"scheduling_policy", "android.os.ISchedulingPolicyService"},
        {"permission", "android.os.IPermissionController"},
        {"permission_checker", "android.permission.IPermissionChecker"},
        {"media.camera.proxy", "android.hardware.ICameraServiceProxy"},
        {"package_native", "android.content.pm.IPackageManagerNative"},
    };
    for (const auto& g : generics) {
        status_t s = sm->addService(String16(g.name), sp<GenericStub>::make(g.descriptor));
        ALOGI("wart-framework-shim: addService(%s) = %d", g.name, s);
    }

    // IProcessInfoService needs the custom array-marshalling stub (camera/codec
    // eviction priority query) — the generic zero-reply can't serve its out int[].
    status_t pis = sm->addService(String16("processinfo"), sp<ProcessInfoStub>::make());
    ALOGI("wart-framework-shim: addService(processinfo) = %d", pis);

    ALOGI("wart-framework-shim: serving");
    IPCThreadState::self()->joinThreadPool();
    return 0;
}
