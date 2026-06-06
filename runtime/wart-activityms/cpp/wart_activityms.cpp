// wart-activityms — a minimal stub "activity" (IActivityManager) binder service for
// ART-off operation.
//
// Under --no-art the Java system_server (which hosts ActivityManager) is gone, but
// native survivors still need it: audioserver (AudioPolicyService::UidPolicy) and
// cameraserver call defaultServiceManager()->waitForService("activity") and then
// registerUidObserver / isUidActive / getUidProcessState / checkPermission. With no
// "activity" service they block forever, wedging audioserver in init so
// media.audio_flinger/policy/aaudio never register → no audio. This permissive stub
// registers "activity" and answers those calls so the survivors finish init. It is
// NOT a real ActivityManager — every uid is reported foreground (PROCESS_STATE_TOP)
// and every permission granted (fine for wart: single-user, privileged). Later it
// can be backed by the arbiter's real foreground/UID state.
//
// C++/libbinder (NOT rsbinder): rsbinder's addService can't register with the real
// Android servicemanager (reply-parse BadParcelable); C++ libbinder addService is
// proven under --no-art (wart-inputflinger registers wart.windowreg). Built on a-03
// like wart-inputflinger; launch via wart-launch (uid system) so it can register a
// system service name. See [[project-artless-audio]].
//
// The transaction codes + parcel layout mirror frameworks/native
// libs/binder/IActivityManager.{h,cpp} (the native client audioserver uses): binder
// dispatches by integer code (not method name), so only the descriptor
// ("android.app.IActivityManager"), the codes, and the per-method parcel format must
// match — they do.

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

using namespace android;

namespace {

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

// Generic permissive stub for the OTHER system_server services native survivors
// (audioserver/cameraserver) block on under --no-art (sensor_privacy, …). It only
// needs to (a) exist so their waitForService() returns, and (b) answer benignly: it
// ignores the request args and replies `noException` + a zero — which a client reads
// as false / 0 / null for bool/int/object returns (e.g. isSensorPrivacyEnabled →
// false = privacy OFF = mic allowed), and which void clients ignore. Each instance
// carries its own descriptor so a client interface_cast resolves.
class GenericStub : public BBinder {
public:
    explicit GenericStub(const char* descriptor) : mDescriptor(descriptor) {}
    const String16& getInterfaceDescriptor() const override { return mDescriptor; }
    status_t onTransact(uint32_t /*code*/, const Parcel& /*data*/, Parcel* reply,
                        uint32_t /*flags*/) override {
        if (reply != nullptr) {
            reply->writeNoException();
            reply->writeInt32(0);
        }
        return NO_ERROR;
    }
private:
    String16 mDescriptor;
};

// IProcessInfoService stub (task 93 — camera open). cameraserver's
// handleEvictionsLocked queries `processinfo` for the oom-priority of camera
// clients; without system_server the query times out (-110) and openCamera fails.
// The GenericStub can't serve this — the replies are sized int[] arrays the client
// reads as RAW blocks (frameworks/native/libs/binder/IProcessInfoService.cpp
// BpProcessInfoService): exception(0), then for each out-array `writeInt32(len)` +
// `len` int32s, then a trailing status int32 — and `len` MUST equal the input pid
// count or the client returns NOT_ENOUGH_DATA. We echo `length` back, reporting
// every pid as PROCESS_STATE_TOP with oom score 0 (single privileged client; the
// exact priority only matters when arbitrating eviction between camera clients).
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

    // The precise IActivityManager stub.
    status_t st = sm->addService(String16("activity"), sp<ActivityStub>::make());
    if (st != OK) {
        ALOGE("wart-activityms: addService(activity) failed: %d", st);
        return 1;
    }
    ALOGI("wart-activityms: registered 'activity' (IActivityManager)");

    // Generic stubs for the other system_server services audioserver/cameraserver
    // wait on under --no-art. Add names here as more surface in the logs.
    struct { const char* name; const char* descriptor; } generics[] = {
        {"sensor_privacy", "android.hardware.ISensorPrivacyManager"},
        // AAudio MMAP start path: AAudioServiceStreamBase::registerAudioThread ->
        // android::requestPriority(isForApp=true) -> infinite loop on
        // checkService("scheduling_policy") when system_server is gone, so the
        // stream never starts (no PCM, no route) -> silence. GenericStub's
        // writeNoException()+writeInt32(0) is exactly what BpSchedulingPolicyService
        // ::requestPriority reads (NO_ERROR). See [[project-artless-audio]].
        {"scheduling_policy", "android.os.ISchedulingPolicyService"},
        // AAudio MMAP start path (4th stub): MmapThread::start ->
        // afutils::checkAttributionSourcePackage -> PermissionController::
        // getPackagesForUid -> getService() loops `checkService("permission");
        // sleep(1)` for 10s then "giving up" (frameworks-native/libs/binder/
        // PermissionController.cpp:30) when system_server's IPermissionController
        // is gone. This 10s block runs ON the audioserver command thread inside
        // START_CLIENT, so the host's startStream (3s timeout) gives up -> no PCM
        // -> silence (device-confirmed: "Waiting for permission service" x N).
        // Registering any binder makes checkService return instantly; GenericStub's
        // writeNoException()+writeInt32(0) decodes as getPackagesForUid's empty
        // Vector<String16>, which checkAttributionSourcePackage handles fine.
        {"permission", "android.os.IPermissionController"},
        // Camera/codec path (task 93): cameraserver's AttributionAndPermissionUtils
        // -> PermissionChecker (frameworks/native/libs/permission) blocks on
        // `getService("permission_checker")` when system_server's
        // PermissionCheckerService is gone (device-confirmed: cameraserver logs
        // "PermissionChecker: Waiting for permission checker service"). The
        // IPermissionChecker methods we hit return an int: checkPermission/checkOp
        // -> PERMISSION_GRANTED (0); finishDataDelivery is void. GenericStub's
        // writeNoException()+writeInt32(0) is exactly PERMISSION_GRANTED, so a single
        // generic stub unblocks the camera (and the codec attribution path).
        {"permission_checker", "android.permission.IPermissionChecker"},
        // Camera open (task 93): CameraService::connectDeviceImpl ->
        // CameraServiceProxyWrapper::isCameraDisabled does
        // `checkService("media.camera.proxy")` and FAIL-CLOSES — `if (proxyBinder
        // == nullptr) return true` (camera DISABLED) — when system_server's
        // ICameraServiceProxy is gone (device-confirmed: "Camera disabled by device
        // policy", openCamera -> ACAMERA_ERROR_PERMISSION_DENIED -10012). Registering
        // a stub makes the proxy non-null; `boolean isCameraDisabled(int)` then
        // reads GenericStub's writeInt32(0) = false → camera ENABLED. Its other
        // methods are oneway void (reply ignored).
        {"media.camera.proxy", "android.hardware.ICameraServiceProxy"},
        // Codec configure (task 93): MediaCodec::connectFormatShaper does
        // `waitForService("package_native")` (BLOCKS forever until registered) and
        // then `IPackageManagerNative::hasSystemFeature(name, ver, out bool)` only to
        // guess if the device is "handheld" for format-shaping — the answer isn't
        // load-bearing for configure (device-confirmed: AMediaCodec_configure hangs,
        // probe pid retrying `package_native` 14×, service not found). A GenericStub
        // unblocks the wait; its writeNoException()+writeInt32(0) reads as Status OK +
        // hasFeature=false (→ shaper treats us as not-handheld, harmless), so configure
        // proceeds. (MediaCodec.cpp:2681; descriptor = the stable-AIDL package.name.)
        {"package_native", "android.content.pm.IPackageManagerNative"},
    };
    for (const auto& g : generics) {
        status_t s = sm->addService(String16(g.name), sp<GenericStub>::make(g.descriptor));
        ALOGI("wart-activityms: addService(%s) = %d", g.name, s);
    }

    // IProcessInfoService needs the custom array-marshalling stub (task 93, camera
    // eviction priority query) — the generic zero-reply can't serve its out int[].
    status_t pis = sm->addService(String16("processinfo"), sp<ProcessInfoStub>::make());
    ALOGI("wart-activityms: addService(processinfo) = %d", pis);

    ALOGI("wart-activityms: serving");
    IPCThreadState::self()->joinThreadPool();
    return 0;
}
