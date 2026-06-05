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
    };
    for (const auto& g : generics) {
        status_t s = sm->addService(String16(g.name), sp<GenericStub>::make(g.descriptor));
        ALOGI("wart-activityms: addService(%s) = %d", g.name, s);
    }

    ALOGI("wart-activityms: serving");
    IPCThreadState::self()->joinThreadPool();
    return 0;
}
