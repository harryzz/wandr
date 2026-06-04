//! wart-activityms — a minimal stub `activity` (IActivityManager) binder service
//! for ART-off operation.
//!
//! Under `--no-art` the Java `system_server` (which hosts ActivityManager) is gone,
//! but native survivors still need it: `audioserver` (AudioPolicyService' UidPolicy)
//! and `cameraserver` call `defaultServiceManager()->waitForService("activity")` and
//! then `registerUidObserver` / `isUidActive` / `getUidProcessState` / `checkPermission`.
//! With no `activity` service those block forever, wedging audioserver in init so
//! `media.audio_flinger/policy/aaudio` never register → no audio (the lost startup
//! sound). This permissive stub registers `"activity"` and answers those calls so the
//! survivors finish init. It is NOT a real ActivityManager — every uid is reported
//! foreground and every permission granted (fine for wart: single-user, privileged).
//! Later it can be backed by the arbiter's real foreground/UID state.
//!
//! Launch via `wart-launch` (uid system) under `--no-art` so it has the context to
//! register a system service name (bare root can't, like wart-inputflinger).

include!(concat!(env!("OUT_DIR"), "/activitymanager.rs"));

use android::app::IActivityManager::{BnActivityManager, IActivityManager};
use rsbinder::service::{kernel, Registry as _};
use rsbinder::{Interface, Proxy as _, SIBinder};

/// `PROCESS_STATE_TOP` (ProcessStateEnum.aidl: UNKNOWN=-1, PERSISTENT=0,
/// PERSISTENT_UI=1, TOP=2). Reporting every uid as TOP/foreground means audio
/// focus / record-privacy never restrict a wart process under `--no-art`.
const PROCESS_STATE_TOP: i32 = 2;
/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;

struct ActivityStub;
impl Interface for ActivityStub {}

impl IActivityManager for ActivityStub {
    fn r#openContentUri(&self, _uri: &str) -> rsbinder::status::Result<i32> {
        Ok(0) // 0 = no fd (the native client treats nonzero as success+fd); unused by audio
    }
    fn r#registerUidObserver(
        &self,
        _observer: &SIBinder,
        _event: i32,
        _cutpoint: i32,
        _calling_package: &str,
    ) -> rsbinder::status::Result<()> {
        Ok(()) // no-op: we never push uid-state callbacks (everything is "foreground")
    }
    fn r#unregisterUidObserver(&self, _observer: &SIBinder) -> rsbinder::status::Result<()> {
        Ok(())
    }
    fn r#registerUidObserverForUids(
        &self,
        observer: &SIBinder,
        _event: i32,
        _cutpoint: i32,
        _calling_package: &str,
        _uids: &[i32],
    ) -> rsbinder::status::Result<SIBinder> {
        Ok(observer.clone()) // hand back a token; audioserver's UidPolicy uses the plain register
    }
    fn r#addUidToObserver(
        &self,
        _observer_token: &SIBinder,
        _calling_package: &str,
        _uid: i32,
    ) -> rsbinder::status::Result<()> {
        Ok(())
    }
    fn r#removeUidFromObserver(
        &self,
        _observer_token: &SIBinder,
        _calling_package: &str,
        _uid: i32,
    ) -> rsbinder::status::Result<()> {
        Ok(())
    }
    fn r#isUidActive(&self, _uid: i32, _calling_package: &str) -> rsbinder::status::Result<bool> {
        Ok(true)
    }
    fn r#getUidProcessState(
        &self,
        _uid: i32,
        _calling_package: &str,
    ) -> rsbinder::status::Result<i32> {
        Ok(PROCESS_STATE_TOP)
    }
    fn r#checkPermission(
        &self,
        _permission: &str,
        _pid: i32,
        _uid: i32,
    ) -> rsbinder::status::Result<i32> {
        Ok(PERMISSION_GRANTED)
    }
    fn r#logFgsApiBegin(&self, _api_type: i32, _uid: i32, _pid: i32) -> rsbinder::status::Result<()> {
        Ok(())
    }
    fn r#logFgsApiEnd(&self, _api_type: i32, _uid: i32, _pid: i32) -> rsbinder::status::Result<()> {
        Ok(())
    }
    fn r#logFgsApiStateChanged(
        &self,
        _api_type: i32,
        _state: i32,
        _uid: i32,
        _pid: i32,
    ) -> rsbinder::status::Result<()> {
        Ok(())
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let host = kernel::Host::new().expect("wart-activityms: ProcessState init (/dev/binder?)");
    // Start the binder thread pool BEFORE add_service: servicemanager pingBinder()s
    // the service during registration, and we must have a thread to answer it or the
    // add fails with FailedTransaction.
    rsbinder::ProcessState::start_thread_pool();
    let service = BnActivityManager::new_binder(ActivityStub);

    // Force the android_16 servicemanager backend instead of rsbinder's sdk-based
    // auto-dispatch (which maps sdk 15 → its android_14 variant, whose addService
    // reply parsing mis-handles THIS device's servicemanager → BadParcelable "Parcel
    // data not fully consumed, unread 36"). This LineageOS build's servicemanager may
    // match the v16 AIDL wire-format.
    let context = rsbinder::ProcessState::as_self()
        .context_object()
        .expect("wart-activityms: context_object (servicemanager handle)");
    let sm = rsbinder::hub::android_16::BpServiceManager::from_binder(context)
        .expect("wart-activityms: build android_16 BpServiceManager");
    match rsbinder::hub::android_16::add_service(&sm, "activity", service.as_binder()) {
        Ok(()) => log::info!("wart-activityms: registered 'activity' (android_16 SM) — serving"),
        Err(e) => log::error!(
            "wart-activityms: addService(activity, android_16) FAILED: debug={e:?} exception={:?}",
            e.exception_code(),
        ),
    }
    host.serve().expect("wart-activityms: join thread pool");
}
