// wart_input_spike — task 80 Step 0 (the go/no-go gate).
//
// Proves Android's C++ InputReader runs STANDALONE (no system_server): a minimal
// InputReaderPolicyInterface + an InputListenerInterface that just logs, driven by
// createInputReader() (which builds its own EventHub reading /dev/input/event*).
// If touching the screen logs MOTION coordinates, the ART-less input path (Option
// B) is validated: ABI match, EventHub evdev read, statsd best-effort, policy
// sufficiency — all at once. Build as a soong cc_binary on the AOSP host (a-03).

#define LOG_TAG "wart_input_spike"
#include <log/log.h>

#include <utils/RefBase.h>
#include <utils/StrongPointer.h>

#include "InputReaderBase.h"
#include "InputReaderFactory.h"
#include "InputListener.h"

#include <input/DisplayViewport.h>
#include <ui/Rotation.h>
#include <ui/LogicalDisplayId.h>

#include <chrono>
#include <thread>

using namespace android;

// Panel facts (Pixel 2 XL). The touchscreen reports 0..1439 / 0..2879 — 1:1.
static constexpr int32_t PANEL_W = 1440;
static constexpr int32_t PANEL_H = 2880;

class SpikeListener : public InputListenerInterface {
public:
    void notifyInputDevicesChanged(const NotifyInputDevicesChangedArgs& a) override {
        ALOGI("spike: devices changed (%zu)", a.inputDeviceInfos.size());
    }
    void notifyKey(const NotifyKeyArgs& a) override {
        ALOGI("spike: KEY action=%d code=%d", a.action, a.keyCode);
    }
    void notifyMotion(const NotifyMotionArgs& a) override {
        float x = a.pointerCoords.empty() ? -1.f : a.pointerCoords[0].getX();
        float y = a.pointerCoords.empty() ? -1.f : a.pointerCoords[0].getY();
        ALOGI("spike: MOTION action=0x%x pointers=%zu x=%.1f y=%.1f",
              a.action, a.pointerCoords.size(), x, y);
    }
    void notifySwitch(const NotifySwitchArgs&) override {}
    void notifySensor(const NotifySensorArgs&) override {}
    void notifyVibratorState(const NotifyVibratorStateArgs&) override {}
    void notifyDeviceReset(const NotifyDeviceResetArgs& a) override {
        ALOGI("spike: device reset id=%d", a.deviceId);
    }
    void notifyPointerCaptureChanged(const NotifyPointerCaptureChangedArgs&) override {}
};

class SpikePolicy : public InputReaderPolicyInterface {
public:
    void getReaderConfiguration(InputReaderConfiguration* outConfig) override {
        // Associate the touchscreen with one internal display so InputReader
        // configures it + maps coordinates. Minimal viewport: 1:1, ROT_0.
        DisplayViewport v;
        v.displayId = ui::LogicalDisplayId::DEFAULT;
        v.orientation = ui::ROTATION_0;
        v.logicalLeft = 0;
        v.logicalTop = 0;
        v.logicalRight = PANEL_W;
        v.logicalBottom = PANEL_H;
        v.physicalLeft = 0;
        v.physicalTop = 0;
        v.physicalRight = PANEL_W;
        v.physicalBottom = PANEL_H;
        v.deviceWidth = PANEL_W;
        v.deviceHeight = PANEL_H;
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
            const InputDeviceIdentifier&,
            const std::optional<KeyboardLayoutInfo>) override {
        return nullptr;
    }
    std::string getDeviceAlias(const InputDeviceIdentifier&) override { return ""; }
    TouchAffineTransformation getTouchAffineTransformation(
            const std::string&, ui::Rotation) override {
        return TouchAffineTransformation();
    }
    void notifyStylusGestureStarted(int32_t, nsecs_t) override {}
    bool isInputMethodConnectionActive() override { return false; }
    std::optional<DisplayViewport> getPointerViewportForAssociatedDisplay(
            ui::LogicalDisplayId) override {
        return std::nullopt;
    }
};

int main() {
    ALOGI("spike: starting standalone InputReader (no system_server)");
    sp<SpikePolicy> policy = sp<SpikePolicy>::make();
    static SpikeListener listener;

    std::unique_ptr<InputReaderInterface> reader = createInputReader(policy, listener);
    if (reader == nullptr) {
        ALOGE("spike: createInputReader returned null");
        return 1;
    }
    if (reader->start() != OK) {
        ALOGE("spike: reader->start() failed");
        return 2;
    }
    ALOGI("spike: reader started — TOUCH THE SCREEN (Ctrl-C / kill to stop)");
    for (;;) {
        std::this_thread::sleep_for(std::chrono::seconds(3600));
    }
    return 0;
}
