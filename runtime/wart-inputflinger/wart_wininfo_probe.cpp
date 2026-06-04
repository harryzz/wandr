// wart_wininfo_probe — minimal diagnostic: register a SurfaceFlinger
// WindowInfosListener and log every push. Does NOT addService or touch input, so
// it's safe to run under normal ART (won't disturb the live system). Purpose:
// determine whether SF delivers WindowInfos to an ordinary process at all (ART up,
// mInputFlinger valid). If it does here but not under ART-off, the ART-off touch
// failure is conclusively SF's mInputFlinger going null — not a listener/permission
// problem. Run: wart_wininfo_probe (as root or via wart-launch).

#define LOG_TAG "wart_wininfo_probe"
#include <log/log.h>
#include <binder/ProcessState.h>
#include <binder/IPCThreadState.h>
#include <utils/StrongPointer.h>
#include <gui/WindowInfosListener.h>
#include <gui/WindowInfosUpdate.h>
#include <gui/SurfaceComposerClient.h>

using namespace android;

class Probe : public gui::WindowInfosListener {
public:
    void onWindowInfosChanged(const gui::WindowInfosUpdate& u) override {
        ALOGE("WART-PROBE onWindowInfosChanged: %zu windows, %zu displays",
              u.windowInfos.size(), u.displayInfos.size());
        for (const auto& w : u.windowInfos) {
            ALOGE("WART-PROBE   win '%s' frame=[%d,%d,%d,%d] cfg=0x%x",
                  w.name.c_str(), w.frame.left, w.frame.top, w.frame.right, w.frame.bottom,
                  w.inputConfig.get());
        }
    }
};

int main() {
    ProcessState::self()->startThreadPool();
    sp<Probe> probe = sp<Probe>::make();
    std::pair<std::vector<gui::WindowInfo>, std::vector<gui::DisplayInfo>> initial;
    status_t st = SurfaceComposerClient::getDefault()->addWindowInfosListener(probe, &initial);
    ALOGE("WART-PROBE addWindowInfosListener -> %d, initial windows=%zu displays=%zu",
          st, initial.first.size(), initial.second.size());
    IPCThreadState::self()->joinThreadPool();
    return 0;
}
