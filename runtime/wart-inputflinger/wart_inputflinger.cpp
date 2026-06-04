// wart-inputflinger — Android's real InputManager (InputReader + InputDispatcher)
// run STANDALONE as the "inputflinger" binder service, for ART-less operation
// (no system_server). This is "path A": instead of every wart-host running its
// own evdev InputReader (the task-80 bootstrap, which fanned global keys to every
// host → the power-key flicker), ONE dispatcher reads input once and routes it:
//   * app keys/touches  → the FOCUSED window only (focus-based routing; the hosts
//     connect via their existing inputflinger InputChannel client path and stop
//     using WART_EVDEV_INPUT). No fan-out, no per-host region filter.
//   * system keys (POWER/VOLUME) → intercepted in the dispatcher policy and
//     forwarded to the wart-arbiter ONCE, then dropped from window dispatch
//     (Android's PhoneWindowManager role; in wart that policy is the arbiter).
//
// Window info is automatic: InputDispatcher's constructor self-registers as a
// SurfaceFlinger WindowInfosListener (InputDispatcher.cpp ~962), so the hosts'
// setInputWindowInfo() calls flow straight in — no bridge code needed here.
//
// Must run in system_server's security context (uid system + gid input +
// CAP_BLOCK_SUSPEND): launched via `wart-launch` under ART-off (bare root HANGS on
// SF's ACCESS_SURFACE_FLINGER check + EventHub aborts without CAP_BLOCK_SUSPEND).
//
// Build: soong cc_binary on a-03 (see Android.bp). Deploy: /data/local/tmp/wart-inputflinger.

#define LOG_TAG "wart_inputflinger"
#include <log/log.h>

#include <binder/IServiceManager.h>
#include <binder/ProcessState.h>
#include <binder/IPCThreadState.h>
#include <utils/StrongPointer.h>

#include "InputManager.h"
#include "InputReaderBase.h"
#include "InputDispatcherPolicyInterface.h"
#include "PointerChoreographerPolicyInterface.h"
#include "InputFilterPolicyInterface.h"

#include <input/DisplayViewport.h>
#include <input/Input.h>
#include <android/keycodes.h>
#include <ui/Rotation.h>
#include <ui/LogicalDisplayId.h>

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <cstring>
#include <cstdlib>
#include <string>

using namespace android;

static constexpr int32_t PANEL_W = 1440;
static constexpr int32_t PANEL_H = 2880;

// Display viewport for the InputReader → must put touch into the SAME coordinate
// space SF reports window touchableRegions in, or the dispatcher hit-tests miss
// ("no touchable window"). On the landscape-native taimen the wart layers' input
// regions come out 2880×1440, so the portrait 1440×2880 default misses. Made
// env-tunable so the right orientation/dims can be dialed in on-device without an
// a-03 rebuild each try. ROTATION from WART_VP_ORIENT (0|1|2|3 → 0/90/180/270).
static int32_t      g_vp_logical_w = PANEL_W;
static int32_t      g_vp_logical_h = PANEL_H;
static int32_t      g_vp_device_w  = PANEL_W;
static int32_t      g_vp_device_h  = PANEL_H;
static ui::Rotation g_vp_orient    = ui::ROTATION_0;

static int env_int(const char* name, int dflt) {
    const char* e = getenv(name);
    return (e && *e) ? atoi(e) : dflt;
}

// The arbiter's control socket — same one the host + arbiter use. Resolved (NOT
// hardcoded) from, in order: the `--arbiter-sock <path>` arg, the
// `WART_ARBITER_SOCK` env var, then the canonical default. This is the shared
// cross-process contract (the Rust host/arbiter resolve the same env var) so the
// path lives in one named place per process, overridable, never sprinkled.
static constexpr const char* ARBITER_SOCK_DEFAULT = "/data/local/tmp/wart-arbiter.sock";
static std::string g_arbiter_sock = ARBITER_SOCK_DEFAULT;

// Fire-and-forget one command line to the arbiter (connect + write + close). Used
// for system keys, which are rare + user-initiated, so a quick blocking local
// write is fine. Mirrors the host's forward_power_key/forward_volume_key.
static void arbiter_send(const char* line) {
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return;
    sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, g_arbiter_sock.c_str(), sizeof(addr.sun_path) - 1);
    if (connect(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0) {
        ssize_t n = write(fd, line, strlen(line));
        (void)n;
    }
    close(fd);
}

// ── Reader policy (from the task-80 input spike) ─────────────────────────────
class WartReaderPolicy : public InputReaderPolicyInterface {
public:
    void getReaderConfiguration(InputReaderConfiguration* outConfig) override {
        DisplayViewport v;
        v.displayId = ui::LogicalDisplayId::DEFAULT;
        v.orientation = g_vp_orient;
        v.logicalRight = g_vp_logical_w;  v.logicalBottom = g_vp_logical_h;
        v.physicalRight = g_vp_logical_w; v.physicalBottom = g_vp_logical_h;
        v.deviceWidth = g_vp_device_w;    v.deviceHeight = g_vp_device_h;
        v.isActive = true;
        v.uniqueId = "local:0";
        v.type = ViewportType::INTERNAL;
        outConfig->setDisplayViewports({v});
    }
    void notifyInputDevicesChanged(const std::vector<InputDeviceInfo>&) override {}
    void notifyTouchpadHardwareState(const SelfContainedHardwareState&, int32_t) override {}
    void notifyTouchpadGestureInfo(GestureType, int32_t) override {}
    void notifyTouchpadThreeFingerTap() override {}
    std::shared_ptr<KeyCharacterMap> getKeyboardLayoutOverlay(
            const InputDeviceIdentifier&, const std::optional<KeyboardLayoutInfo>) override {
        return nullptr;
    }
    std::string getDeviceAlias(const InputDeviceIdentifier&) override { return ""; }
    TouchAffineTransformation getTouchAffineTransformation(
            const std::string&, ui::Rotation) override { return TouchAffineTransformation(); }
    void notifyStylusGestureStarted(int32_t, nsecs_t) override {}
    bool isInputMethodConnectionActive() override { return false; }
    std::optional<DisplayViewport> getPointerViewportForAssociatedDisplay(
            ui::LogicalDisplayId) override { return std::nullopt; }
};

// ── Dispatcher policy — the wart "PhoneWindowManager": intercept system keys ──
class WartDispatcherPolicy : public InputDispatcherPolicyInterface {
public:
    void notifyNoFocusedWindowAnr(const std::shared_ptr<InputApplicationHandle>&) override {}
    void notifyWindowUnresponsive(const sp<IBinder>&, std::optional<gui::Pid>,
                                  const std::string&) override {}
    void notifyWindowResponsive(const sp<IBinder>&, std::optional<gui::Pid>) override {}
    void notifyInputChannelBroken(const sp<IBinder>&) override {}
    void notifyFocusChanged(const sp<IBinder>&, const sp<IBinder>&) override {}
    void notifySensorEvent(int32_t, InputDeviceSensorType, InputDeviceSensorAccuracy, nsecs_t,
                           const std::vector<float>&) override {}
    void notifySensorAccuracy(int32_t, InputDeviceSensorType, InputDeviceSensorAccuracy) override {}
    void notifyVibratorState(int32_t, bool) override {}
    void notifyFocusedDisplayChanged(ui::LogicalDisplayId) override {}
    bool filterInputEvent(const InputEvent&, uint32_t) override { return true; }

    // Called for EVERY key before it is queued. This is the single choke point
    // (one dispatcher, not N hosts), so forwarding here happens exactly once per
    // physical press — fixing the ART-off power-key fan-out/flicker. For system
    // keys we forward to the arbiter and do NOT set POLICY_FLAG_PASS_TO_USER, so
    // the dispatcher drops them (InputDispatcher.cpp: DropReason::POLICY) instead
    // of routing to a window. Forward on DOWN only (ignore the UP) for a single
    // event per press.
    void interceptKeyBeforeQueueing(const KeyEvent& keyEvent, uint32_t& policyFlags) override {
        const int32_t code = keyEvent.getKeyCode();
        const bool down = keyEvent.getAction() == AKEY_EVENT_ACTION_DOWN;
        switch (code) {
            case AKEYCODE_POWER:
                if (down) arbiter_send("power-key\n");
                return; // not PASS_TO_USER → dropped from window dispatch
            case AKEYCODE_VOLUME_UP:
                if (down) arbiter_send("volume up\n");
                return;
            case AKEYCODE_VOLUME_DOWN:
                if (down) arbiter_send("volume down\n");
                return;
            default:
                policyFlags |= POLICY_FLAG_PASS_TO_USER; // app key → focused window
                return;
        }
    }
    void interceptMotionBeforeQueueing(ui::LogicalDisplayId, uint32_t, int32_t, nsecs_t,
                                       uint32_t& policyFlags) override {
        policyFlags |= POLICY_FLAG_PASS_TO_USER;
    }
    nsecs_t interceptKeyBeforeDispatching(const sp<IBinder>&, const KeyEvent&, uint32_t) override {
        return 0;
    }
    std::optional<KeyEvent> dispatchUnhandledKey(const sp<IBinder>&, const KeyEvent&,
                                                 uint32_t) override {
        return std::nullopt;
    }
    void notifySwitch(nsecs_t, uint32_t, uint32_t, uint32_t) override {}
    void pokeUserActivity(nsecs_t, int32_t, ui::LogicalDisplayId, int32_t) override {}
    void onPointerDownOutsideFocus(const sp<IBinder>&) override {}
    void setPointerCapture(const PointerCaptureRequest&) override {}
    void notifyDropWindow(const sp<IBinder>&, float, float) override {}
    void notifyDeviceInteraction(DeviceId, nsecs_t, const std::set<gui::Uid>&) override {}
};

// ── Choreographer policy (touch-only → no pointer controller) ────────────────
class WartChoreographerPolicy : public PointerChoreographerPolicyInterface {
public:
    std::shared_ptr<PointerControllerInterface> createPointerController(
            PointerControllerInterface::ControllerType) override {
        return nullptr;
    }
    void notifyPointerDisplayIdChanged(ui::LogicalDisplayId, const vec2&) override {}
    bool isInputMethodConnectionActive() override { return false; }
    void notifyMouseCursorFadedOnTyping() override {}
};

// ── Input filter policy (stub) ───────────────────────────────────────────────
class WartFilterPolicy : public InputFilterPolicyInterface {
public:
    void notifyStickyModifierStateChanged(uint32_t, uint32_t) override {}
};

int main(int argc, char** argv) {
    // Resolve the arbiter socket: --arbiter-sock <path> > WART_ARBITER_SOCK env >
    // default. (No hardcoded literal in the hot path; one source, overridable.)
    for (int i = 1; i + 1 < argc; ++i) {
        if (strcmp(argv[i], "--arbiter-sock") == 0) { g_arbiter_sock = argv[i + 1]; break; }
    }
    if (g_arbiter_sock == ARBITER_SOCK_DEFAULT) {
        if (const char* e = getenv("WART_ARBITER_SOCK"); e && *e) g_arbiter_sock = e;
    }
    // Viewport tuning (default = portrait panel; override to align with SF's window
    // coordinate space on rotated/landscape-native panels).
    g_vp_logical_w = env_int("WART_VP_LOGICAL_W", PANEL_W);
    g_vp_logical_h = env_int("WART_VP_LOGICAL_H", PANEL_H);
    g_vp_device_w  = env_int("WART_VP_DEVICE_W", g_vp_logical_w);
    g_vp_device_h  = env_int("WART_VP_DEVICE_H", g_vp_logical_h);
    switch (env_int("WART_VP_ORIENT", 0)) {
        case 1: g_vp_orient = ui::ROTATION_90; break;
        case 2: g_vp_orient = ui::ROTATION_180; break;
        case 3: g_vp_orient = ui::ROTATION_270; break;
        default: g_vp_orient = ui::ROTATION_0; break;
    }
    ALOGI("wart-inputflinger: starting standalone InputManager (= inputflinger, no system_server); "
          "arbiter-sock=%s viewport=logical %dx%d device %dx%d orient=%d",
          g_arbiter_sock.c_str(), g_vp_logical_w, g_vp_logical_h,
          g_vp_device_w, g_vp_device_h, static_cast<int>(g_vp_orient));
    android::ProcessState::self()->startThreadPool();

    sp<WartReaderPolicy> readerPolicy = sp<WartReaderPolicy>::make();
    static WartDispatcherPolicy dispatcherPolicy;
    static WartChoreographerPolicy choreographerPolicy;
    static WartFilterPolicy filterPolicy;

    sp<InputManager> im = sp<InputManager>::make(readerPolicy, dispatcherPolicy,
                                                 choreographerPolicy, filterPolicy);

    status_t reg = defaultServiceManager()->addService(String16("inputflinger"), im);
    ALOGI("wart-inputflinger: addService(inputflinger) → %d", reg);

    status_t st = im->start();
    ALOGI("wart-inputflinger: InputManager::start() → %d", st);

    // The InputDispatcher boots with dispatch DISABLED ("Dropped event because
    // input dispatch is disabled") — system_server's InputManagerService normally
    // enables it after boot. We are that owner now, so enable it. (System-key
    // interception in interceptKeyBeforeQueueing runs at enqueue time and works
    // regardless, which is why POWER/VOLUME worked but touch was dropped.)
    im->getDispatcher().setInputDispatchMode(/*enabled=*/true, /*frozen=*/false);
    ALOGI("wart-inputflinger: input dispatch ENABLED");

    // NOTE (path A, open item): under ART-off, SurfaceFlinger does NOT push
    // WindowInfos to our dispatcher because SF binds the inputflinger service once
    // at its own init (mInputFlinger) and cleared it when system_server died, so
    // SF::updateInputFlinger early-returns. System-key interception (above) works
    // because it runs at enqueue; app TOUCH does not route (the dispatcher has no
    // windows). Cracking that needs SF to re-bind mInputFlinger to us — a fragile
    // SF-restart dance — or a different window source. See memory
    // project_pathA_inputflinger + the wart_wininfo_probe diagnostic.
    ALOGI("wart-inputflinger: up — system keys → arbiter (dedup); app touch routing "
          "pending SF mInputFlinger fix under ART-off");
    android::IPCThreadState::self()->joinThreadPool();
    return 0;
}
