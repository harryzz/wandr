// wandr_inputflinger_spike — task 81/architecture spike.
//
// Run Android's real InputManager (= BnInputFlinger: InputReader + InputDispatcher
// + …) STANDALONE, register it as the "inputflinger" binder service, and start it —
// proving the proven input architecture works with no system_server. If this comes
// up + registers + serves, our hosts' EXISTING inputflinger client path
// (createInputChannel/setInputWindowInfo/InputConsumer) connects to it unchanged.
//
// Stage 1 (this file): construct + register + start (validates standalone runtime +
// SELinux service registration). Window-info routing (SF WindowInfosListener →
// dispatcher) is stage 2. Build as a soong cc_binary on a-03.

#define LOG_TAG "wandr_inputflinger_spike"
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
#include <ui/Rotation.h>
#include <ui/LogicalDisplayId.h>

#include <chrono>
#include <thread>

using namespace android;

static constexpr int32_t PANEL_W = 1440;
static constexpr int32_t PANEL_H = 2880;

// ── Reader policy (from the task-80 input spike) ─────────────────────────────
class SpikeReaderPolicy : public InputReaderPolicyInterface {
public:
    void getReaderConfiguration(InputReaderConfiguration* outConfig) override {
        DisplayViewport v;
        v.displayId = ui::LogicalDisplayId::DEFAULT;
        v.orientation = ui::ROTATION_0;
        v.logicalRight = PANEL_W;  v.logicalBottom = PANEL_H;
        v.physicalRight = PANEL_W; v.physicalBottom = PANEL_H;
        v.deviceWidth = PANEL_W;   v.deviceHeight = PANEL_H;
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

// ── Dispatcher policy (23 methods — stubs; keys pass through to apps) ─────────
class SpikeDispatcherPolicy : public InputDispatcherPolicyInterface {
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
    void interceptKeyBeforeQueueing(const KeyEvent&, uint32_t& policyFlags) override {
        policyFlags |= POLICY_FLAG_PASS_TO_USER;
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
class SpikeChoreographerPolicy : public PointerChoreographerPolicyInterface {
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
class SpikeFilterPolicy : public InputFilterPolicyInterface {
public:
    void notifyStickyModifierStateChanged(uint32_t, uint32_t) override {}
};

int main() {
    ALOGI("spike: starting standalone InputManager (= inputflinger, no system_server)");
    android::ProcessState::self()->startThreadPool();

    sp<SpikeReaderPolicy> readerPolicy = sp<SpikeReaderPolicy>::make();
    static SpikeDispatcherPolicy dispatcherPolicy;
    static SpikeChoreographerPolicy choreographerPolicy;
    static SpikeFilterPolicy filterPolicy;

    sp<InputManager> im = sp<InputManager>::make(readerPolicy, dispatcherPolicy,
                                                 choreographerPolicy, filterPolicy);

    status_t reg = defaultServiceManager()->addService(String16("inputflinger"), im);
    ALOGI("spike: addService(inputflinger) → %d", reg);

    status_t st = im->start();
    ALOGI("spike: InputManager::start() → %d", st);

    ALOGI("spike: up — inputflinger registered + running standalone");
    android::IPCThreadState::self()->joinThreadPool();
    return 0;
}
