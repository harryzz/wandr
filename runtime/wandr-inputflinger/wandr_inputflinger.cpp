// wandr-inputflinger — Android's real InputManager (InputReader + InputDispatcher)
// run STANDALONE as the "inputflinger" binder service, for ART-less operation
// (no system_server). This is "path A": instead of every wandr-host running its
// own evdev InputReader (the task-80 bootstrap, which fanned global keys to every
// host → the power-key flicker), ONE dispatcher reads input once and routes it:
//   * app keys/touches  → the FOCUSED window only (focus-based routing; the hosts
//     connect via their existing inputflinger InputChannel client path and stop
//     using WANDR_EVDEV_INPUT). No fan-out, no per-host region filter.
//   * system keys (POWER/VOLUME) → intercepted in the dispatcher policy and
//     forwarded to the wandr-arbiter ONCE, then dropped from window dispatch
//     (Android's PhoneWindowManager role; in wandr that policy is the arbiter).
//
// Window info is automatic: InputDispatcher's constructor self-registers as a
// SurfaceFlinger WindowInfosListener (InputDispatcher.cpp ~962), so the hosts'
// setInputWindowInfo() calls flow straight in — no bridge code needed here.
//
// Must run in system_server's security context (uid system + gid input +
// CAP_BLOCK_SUSPEND): launched via `wandr-launch` under ART-off (bare root HANGS on
// SF's ACCESS_SURFACE_FLINGER check + EventHub aborts without CAP_BLOCK_SUSPEND).
//
// Build: soong cc_binary on a-03 (see Android.bp). Deploy: /data/local/tmp/wandr-inputflinger.

#define LOG_TAG "wandr_inputflinger"
#include <log/log.h>

#include <binder/IServiceManager.h>
#include <binder/ProcessState.h>
#include <binder/IPCThreadState.h>
#include <binder/Binder.h>
#include <binder/Parcel.h>
#include <utils/StrongPointer.h>
#include <utils/Timers.h>

#include "InputManager.h"
#include "InputReaderBase.h"
#include "InputDispatcherPolicyInterface.h"
#include "PointerChoreographerPolicyInterface.h"
#include "InputFilterPolicyInterface.h"

#include <input/DisplayViewport.h>
#include <input/Input.h>
#include <gui/WindowInfo.h>
#include <gui/WindowInfosUpdate.h>
#include <android/keycodes.h>
#include <ui/Rect.h>
#include <ui/Region.h>
#include <ui/Rotation.h>
#include <ui/LogicalDisplayId.h>

#include <sys/socket.h>
#include <sys/un.h>
#include <sys/stat.h>
#include <unistd.h>
#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <cerrno>
#include <cstddef>
#include <string>
#include <vector>
#include <map>
#include <mutex>
#include <thread>

using namespace android;

static constexpr int32_t PANEL_W = 1440;
static constexpr int32_t PANEL_H = 2880;

// Display viewport for the InputReader → must put touch into the SAME coordinate
// space SF reports window touchableRegions in, or the dispatcher hit-tests miss
// ("no touchable window"). On the landscape-native taimen the wandr layers' input
// regions come out 2880×1440, so the portrait 1440×2880 default misses. Made
// env-tunable so the right orientation/dims can be dialed in on-device without an
// a-03 rebuild each try. ROTATION from WANDR_VP_ORIENT (0|1|2|3 → 0/90/180/270).
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
// `WANDR_ARBITER_SOCK` env var, then the canonical default. This is the shared
// cross-process contract (the Rust host/arbiter resolve the same env var) so the
// path lives in one named place per process, overridable, never sprinkled.
static constexpr const char* ARBITER_SOCK_DEFAULT = "/data/local/tmp/wandr-arbiter.sock";
static std::string g_arbiter_sock = ARBITER_SOCK_DEFAULT;

// The arbiter→inputflinger window-feed socket (task 84). WE listen here; the
// arbiter connects + pushes the ordered window list. Resolved (NOT hardcoded)
// from `--inputflinger-sock <path>` > `WANDR_INPUTFLINGER_SOCK` env > default —
// symmetric with the Rust arbiter's resolution of the same env var.
//
// ABSTRACT namespace by default (leading `@`): wandr-inputflinger runs as uid
// system (via wandr-launch) and CANNOT create a file under `/data/local/tmp`
// (0771 shell:shell — connect-yes, bind-no), so a filesystem socket bind would
// EACCES. The abstract namespace has no filesystem entry + no path perms; the
// root arbiter connects to it by name. A `@name` here ⇒ abstract `\0name`.
static constexpr const char* INPUTFLINGER_SOCK_DEFAULT = "@wandr-inputflinger";
static std::string g_inputflinger_sock = INPUTFLINGER_SOCK_DEFAULT;

// Fill a sockaddr_un for either a filesystem path or (if it starts with '@') an
// abstract-namespace name; returns the address length to pass to bind/connect.
static socklen_t fill_un_addr(sockaddr_un& addr, const std::string& path) {
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    if (!path.empty() && path[0] == '@') {
        // Abstract: sun_path[0] = '\0', then the name (NOT NUL-terminated).
        const std::string name = path.substr(1);
        memcpy(addr.sun_path + 1, name.c_str(), name.size());
        return static_cast<socklen_t>(offsetof(sockaddr_un, sun_path) + 1 + name.size());
    }
    strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);
    return static_cast<socklen_t>(sizeof(addr));
}

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
class WandrReaderPolicy : public InputReaderPolicyInterface {
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

// ── Dispatcher policy — the wandr "PhoneWindowManager": intercept system keys ──
class WandrDispatcherPolicy : public InputDispatcherPolicyInterface {
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
                // Task 110 — forward DOWN + UP so the arbiter can time the press
                // (single decider): short = panel toggle, hold ≥1 s = power menu.
                arbiter_send(down ? "power-down\n" : "power-up\n");
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
    // AOSP: the InputDispatcher calls this for every event passed to a window
    // (InputDispatcher.cpp pokeUserActivityLocked → mPolicy.pokeUserActivity), and
    // the real policy forwards it to PowerManagerService.userActivity() to reset the
    // screen-off timer. In wandr the arbiter IS PowerManagerService, so we forward a
    // `user-activity` poke. The dispatcher already rate-limits these, but throttle
    // here too (~1/s) so a swipe can't flood the arbiter socket. `eventTime` is the
    // event's CLOCK_MONOTONIC ns.
    void pokeUserActivity(nsecs_t eventTime, int32_t, ui::LogicalDisplayId, int32_t) override {
        static nsecs_t s_lastSent = 0;
        if (eventTime - s_lastSent < 1'000'000'000LL) return; // ≥ 1 s apart
        s_lastSent = eventTime;
        arbiter_send("user-activity\n");
    }
    void onPointerDownOutsideFocus(const sp<IBinder>&) override {}
    void setPointerCapture(const PointerCaptureRequest&) override {}
    void notifyDropWindow(const sp<IBinder>&, float, float) override {}
    void notifyDeviceInteraction(DeviceId, nsecs_t, const std::set<gui::Uid>&) override {}
};

// ── Choreographer policy (touch-only → no pointer controller) ────────────────
class WandrChoreographerPolicy : public PointerChoreographerPolicyInterface {
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
class WandrFilterPolicy : public InputFilterPolicyInterface {
public:
    void notifyStickyModifierStateChanged(uint32_t, uint32_t) override {}
};

// ── Task 84: feed app windows to the dispatcher (ART-off) ────────────────────
//
// With the Java framework stopped there is no system_server WindowManager, so
// SurfaceFlinger never pushes WindowInfos to our InputDispatcher (it binds the
// inputflinger service once at its own init, cleared mInputFlinger when
// system_server died, and updateInputFlinger then early-returns). The dispatcher
// thus has ZERO windows and drops every touch ("no touchable window"). The
// wandr-arbiter — which IS the window manager — instead authors the ordered window
// list and pushes it to us over a UNIX socket; we feed it straight into the
// dispatcher via InputDispatcher::onWindowInfosChanged, the same entry the AOSP
// dispatcher unit tests use to simulate SF's WindowInfosListener callback.
//
// Reaching that concrete method without dragging in the heavy private
// InputDispatcher.h (its 42 internal headers + perfetto + aconfig + the Rust
// bridge — none of which we link): InputDispatcher is *single*-inheritance from
// InputDispatcherInterface (InputDispatcher.h: `class InputDispatcher : public
// android::InputDispatcherInterface`), so the `InputDispatcherInterface&` that
// InputManager::getDispatcher() hands us IS the InputDispatcher object at offset
// 0. We declare just the one method here, in the dispatcher's real namespace
// (android::inputdispatcher), so the call resolves against libinputflinger.so's
// exported symbol `android::inputdispatcher::InputDispatcher::onWindowInfosChanged`
// (verified present via llvm-nm -D). (Invariant: if a future AOSP bump inserts a
// base before InputDispatcherInterface, this offset-0 cast must be revisited.)
namespace android {
namespace inputdispatcher {
class InputDispatcher {
public:
    void onWindowInfosChanged(const gui::WindowInfosUpdate&);
};
}  // namespace inputdispatcher
}  // namespace android

// pid → its input-channel connection token (minted by our own dispatcher at
// createInputChannel, then registered back by the host over binder so the token's
// kernel identity is preserved end-to-end) + a stable per-pid application token
// for focus/ANR ownership. The arbiter refers to windows by pid (which it already
// keys everything on); we join pid → token here. The token cannot ride the
// arbiter's socket (it's a kernel binder object), so this one hop is binder —
// host↔inputflinger, never through the (Rust) arbiter.
struct WinTokens {
    sp<IBinder> channel;
    sp<IBinder> application;
};
static std::mutex g_win_mtx;
static std::map<int32_t, WinTokens> g_win_tokens;

// Last complete window block the arbiter pushed, cached so a token that registers
// AFTER the block referencing its pid (the common case at launch — the arbiter
// authors the window before the forked host has created its input channel) can be
// re-applied. Without this re-feed the freshly launched app's window stays skipped
// (feed_window_block drops pids with no token) until the next arbiter push, i.e.
// the user has to background+foreground the app to get input. `g_block_mtx` guards
// the cache; `g_feed_mtx` serializes the dispatch itself (the listener thread and
// the binder re-feed below both call feed_window_block).
static std::mutex g_block_mtx;
static std::vector<std::string> g_last_block;
static InputManagerInterface* g_last_im = nullptr;
static std::mutex g_feed_mtx;
static void refeed_last_block();  // defined below feed_window_block

static void wandr_register_token(int32_t pid, const sp<IBinder>& token) {
    std::lock_guard<std::mutex> lk(g_win_mtx);
    WinTokens& e = g_win_tokens[pid];
    e.channel = token;
    if (e.application == nullptr) e.application = sp<BBinder>::make();
    ALOGI("wandr-inputflinger: registered window token pid=%d", pid);
}
static void wandr_unregister_token(int32_t pid) {
    std::lock_guard<std::mutex> lk(g_win_mtx);
    g_win_tokens.erase(pid);
    ALOGI("wandr-inputflinger: unregistered window token pid=%d", pid);
}

// Hand-rolled binder service the hosts call to register their (pid, token). A
// plain BBinder (no AIDL codegen) with two transaction codes; the payload is read
// directly — no interface-token handshake for this private, same-tree contract.
// Registered as "wandr.windowreg"; the host resolves it after createInputChannel.
class WandrWindowReg : public BBinder {
public:
    enum { TX_REGISTER = IBinder::FIRST_CALL_TRANSACTION, TX_UNREGISTER };
    status_t onTransact(uint32_t code, const Parcel& data, Parcel* reply,
                        uint32_t flags) override {
        switch (code) {
            case TX_REGISTER: {
                int32_t pid = data.readInt32();
                sp<IBinder> token = data.readStrongBinder();
                if (token != nullptr) {
                    wandr_register_token(pid, token);
                    // The arbiter likely authored this pid's window before we had
                    // its token (it was skipped). Re-apply the last block now so
                    // the app gets input immediately, with no foreground round-trip.
                    refeed_last_block();
                }
                return OK;
            }
            case TX_UNREGISTER:
                wandr_unregister_token(data.readInt32());
                return OK;
            default:
                return BBinder::onTransact(code, data, reply, flags);
        }
    }
};

// Parse one arbiter window-list block (terminated by `win-commit`) and feed the
// dispatcher. Lines:
//   win-begin <orient> <w> <h>                 (informational — rects are already
//                                               in the reader's display space)
//   win <pid> <l> <t> <r> <b> <focusable:0|1>  (TOP→BOTTOM z-order)
//   win-focus <pid|-1>
//   win-commit
// Each window's token is looked up by pid; a window whose host hasn't registered
// its token yet is skipped — but the block is cached and TX_REGISTER re-applies it
// via refeed_last_block() the moment that token arrives, so the window appears
// without waiting for the next arbiter push (a background+foreground round-trip).
static void feed_window_block(InputManagerInterface* im,
                              const std::vector<std::string>& lines,
                              bool cache = true) {
    // Serialize dispatch: the window-feed listener thread and the binder re-feed
    // (refeed_last_block, from a token registration) can both land here.
    std::lock_guard<std::mutex> feed_lk(g_feed_mtx);
    // Cache this block so a late token registration can re-apply it. The re-feed
    // itself passes cache=false so it can't revert the cache to a stale block if a
    // newer arbiter push raced in between its copy and this dispatch.
    if (cache) {
        std::lock_guard<std::mutex> lk(g_block_mtx);
        g_last_im = im;
        g_last_block = lines;
    }
    std::vector<gui::WindowInfo> wins;
    std::map<int32_t, bool> seen;  // dedup pids — the dispatcher rejects dup window ids
    int32_t focus_pid = -1;
    for (const std::string& ln : lines) {
        if (ln.rfind("win ", 0) == 0) {
            int pid = 0, l = 0, t = 0, r = 0, b = 0, foc = 0;
            if (sscanf(ln.c_str(), "win %d %d %d %d %d %d", &pid, &l, &t, &r, &b, &foc) != 6) {
                continue;
            }
            if (seen.count(pid)) continue;
            seen[pid] = true;
            sp<IBinder> chan, app;
            {
                std::lock_guard<std::mutex> lk(g_win_mtx);
                auto it = g_win_tokens.find(pid);
                if (it == g_win_tokens.end()) continue;  // token not registered yet
                chan = it->second.channel;
                app = it->second.application;
            }
            gui::WindowInfo wi;
            wi.token = chan;
            // The dispatcher rejects a WindowInfosUpdate with duplicate window ids
            // (default -1) — one per host surface, so key it on the (unique) pid.
            wi.id = pid;
            wi.name = "wandr-" + std::to_string(pid);
            wi.globalScaleFactor = 1.0f;
            wi.frame = Rect(l, t, r, b);
            // touchableRegion is display-space (the dispatcher hit-tests it against
            // the display-transformed touch point — InputDispatcher.cpp:599).
            wi.touchableRegion.orSelf(Rect(l, t, r, b));
            // transform maps DISPLAY → WINDOW-LOCAL: the dispatcher delivers
            // `transform.transform(rawX, rawY)` to the channel (InputDispatcher.cpp
            // :2135), and the host passes that through to the guest verbatim. Under
            // normal ART, SurfaceFlinger authored this; bypassing SF we must set it
            // ourselves or an offset window (the IME, a bottom strip) hands the
            // guest raw display coords — far outside its surface — so its keys never
            // register. Pure translate by -topLeft (identity for fullscreen windows).
            wi.transform.set(static_cast<float>(-l), static_cast<float>(-t));
            wi.displayId = ui::LogicalDisplayId::DEFAULT;
            wi.applicationInfo.token = app;
            wi.applicationInfo.name = wi.name;
            wi.applicationInfo.dispatchingTimeoutMillis = 5000;
            // Default inputConfig = touchable + focusable (matches the proven host
            // WindowInfo). Chrome/IME are touch-only → mark NOT_FOCUSABLE so keys
            // never target a bar/keyboard strip.
            if (foc == 0) {
                wi.inputConfig |= gui::WindowInfo::InputConfig::NOT_FOCUSABLE;
            }
            wins.push_back(std::move(wi));
        } else if (ln.rfind("win-focus ", 0) == 0) {
            sscanf(ln.c_str(), "win-focus %d", &focus_pid);
        }
    }

    static int64_t vsync = 0;
    gui::WindowInfosUpdate update(std::move(wins), {}, ++vsync,
                                  systemTime(SYSTEM_TIME_MONOTONIC));
    reinterpret_cast<android::inputdispatcher::InputDispatcher*>(&im->getDispatcher())
            ->onWindowInfosChanged(update);

    // Key focus (touch routes by hit-test regardless). setFocusedWindow is on the
    // public InputDispatcherInterface, so no shim needed. The window must be a
    // focusable one we just pushed — which holds (the arbiter picks a focusable
    // pid: lockscreen or the visible app).
    if (focus_pid >= 0) {
        sp<IBinder> chan;
        {
            std::lock_guard<std::mutex> lk(g_win_mtx);
            auto it = g_win_tokens.find(focus_pid);
            if (it != g_win_tokens.end()) chan = it->second.channel;
        }
        if (chan != nullptr) {
            gui::FocusRequest fr;
            fr.token = chan;
            fr.windowName = "wandr-" + std::to_string(focus_pid);
            fr.timestamp = systemTime(SYSTEM_TIME_MONOTONIC);
            fr.displayId = ui::LogicalDisplayId::DEFAULT.val();
            im->getDispatcher().setFocusedWindow(fr);
        }
    }
}

// Re-apply the last authored window block — called when a host registers its
// window token, which may have arrived after the arbiter pushed the block that
// referenced that pid (so the window was skipped). Re-running the feed now picks
// the pid up via the (now-present) token. Copies the cache out before dispatch so
// it doesn't hold g_block_mtx across feed_window_block.
static void refeed_last_block() {
    InputManagerInterface* im;
    std::vector<std::string> block;
    {
        std::lock_guard<std::mutex> lk(g_block_mtx);
        im = g_last_im;
        block = g_last_block;
    }
    if (im != nullptr && !block.empty()) feed_window_block(im, block, /*cache=*/false);
}

// Listen on the arbiter→inputflinger window-feed socket. The arbiter pushes one
// block per connection (connect → write → shutdown(Write) → close), so we read to
// EOF, split into lines, and feed iff the block is complete (`win-commit` seen).
static void window_feed_listener(InputManagerInterface* im, std::string sock_path) {
    const bool is_abstract = !sock_path.empty() && sock_path[0] == '@';
    if (!is_abstract) unlink(sock_path.c_str());
    int srv = socket(AF_UNIX, SOCK_STREAM, 0);
    if (srv < 0) {
        ALOGE("wandr-inputflinger: window-feed socket() failed: %s", strerror(errno));
        return;
    }
    sockaddr_un addr;
    socklen_t len = fill_un_addr(addr, sock_path);
    if (bind(srv, reinterpret_cast<sockaddr*>(&addr), len) != 0) {
        ALOGE("wandr-inputflinger: window-feed bind(%s) failed: %s", sock_path.c_str(),
              strerror(errno));
        close(srv);
        return;
    }
    if (!is_abstract) chmod(sock_path.c_str(), 0666);
    listen(srv, 8);
    ALOGI("wandr-inputflinger: window-feed listening on %s", sock_path.c_str());
    for (;;) {
        int c = accept(srv, nullptr, nullptr);
        if (c < 0) continue;
        std::string buf;
        char tmp[2048];
        ssize_t n;
        while ((n = read(c, tmp, sizeof(tmp))) > 0) buf.append(tmp, static_cast<size_t>(n));
        close(c);
        std::vector<std::string> lines;
        bool commit = false;
        size_t start = 0, nl;
        while ((nl = buf.find('\n', start)) != std::string::npos) {
            std::string line = buf.substr(start, nl - start);
            if (line == "win-commit") commit = true;
            lines.push_back(std::move(line));
            start = nl + 1;
        }
        if (commit) feed_window_block(im, lines);
    }
}

int main(int argc, char** argv) {
    // Resolve the arbiter socket: --arbiter-sock <path> > WANDR_ARBITER_SOCK env >
    // default. (No hardcoded literal in the hot path; one source, overridable.)
    for (int i = 1; i + 1 < argc; ++i) {
        if (strcmp(argv[i], "--arbiter-sock") == 0) g_arbiter_sock = argv[i + 1];
        else if (strcmp(argv[i], "--inputflinger-sock") == 0) g_inputflinger_sock = argv[i + 1];
    }
    if (g_arbiter_sock == ARBITER_SOCK_DEFAULT) {
        if (const char* e = getenv("WANDR_ARBITER_SOCK"); e && *e) g_arbiter_sock = e;
    }
    if (g_inputflinger_sock == INPUTFLINGER_SOCK_DEFAULT) {
        if (const char* e = getenv("WANDR_INPUTFLINGER_SOCK"); e && *e) g_inputflinger_sock = e;
    }
    // Viewport tuning (default = portrait panel; override to align with SF's window
    // coordinate space on rotated/landscape-native panels).
    g_vp_logical_w = env_int("WANDR_VP_LOGICAL_W", PANEL_W);
    g_vp_logical_h = env_int("WANDR_VP_LOGICAL_H", PANEL_H);
    g_vp_device_w  = env_int("WANDR_VP_DEVICE_W", g_vp_logical_w);
    g_vp_device_h  = env_int("WANDR_VP_DEVICE_H", g_vp_logical_h);
    switch (env_int("WANDR_VP_ORIENT", 0)) {
        case 1: g_vp_orient = ui::ROTATION_90; break;
        case 2: g_vp_orient = ui::ROTATION_180; break;
        case 3: g_vp_orient = ui::ROTATION_270; break;
        default: g_vp_orient = ui::ROTATION_0; break;
    }
    ALOGI("wandr-inputflinger: starting standalone InputManager (= inputflinger, no system_server); "
          "arbiter-sock=%s viewport=logical %dx%d device %dx%d orient=%d",
          g_arbiter_sock.c_str(), g_vp_logical_w, g_vp_logical_h,
          g_vp_device_w, g_vp_device_h, static_cast<int>(g_vp_orient));
    android::ProcessState::self()->startThreadPool();

    sp<WandrReaderPolicy> readerPolicy = sp<WandrReaderPolicy>::make();
    static WandrDispatcherPolicy dispatcherPolicy;
    static WandrChoreographerPolicy choreographerPolicy;
    static WandrFilterPolicy filterPolicy;

    sp<InputManager> im = sp<InputManager>::make(readerPolicy, dispatcherPolicy,
                                                 choreographerPolicy, filterPolicy);

    status_t reg = defaultServiceManager()->addService(String16("inputflinger"), im);
    ALOGI("wandr-inputflinger: addService(inputflinger) → %d", reg);

    status_t st = im->start();
    ALOGI("wandr-inputflinger: InputManager::start() → %d", st);

    // The InputDispatcher boots with dispatch DISABLED ("Dropped event because
    // input dispatch is disabled") — system_server's InputManagerService normally
    // enables it after boot. We are that owner now, so enable it. (System-key
    // interception in interceptKeyBeforeQueueing runs at enqueue time and works
    // regardless, which is why POWER/VOLUME worked but touch was dropped.)
    im->getDispatcher().setInputDispatchMode(/*enabled=*/true, /*frozen=*/false);
    // We own the (single) display; system_server's IMS normally sets this. Needed
    // for key events to reach the focused window (touch hit-tests by displayId
    // regardless). The arbiter authors per-window focus via the feed below.
    im->getDispatcher().setFocusedDisplay(ui::LogicalDisplayId::DEFAULT);
    ALOGI("wandr-inputflinger: input dispatch ENABLED");

    // Task 84 — app TOUCH/key routing under ART-off. SurfaceFlinger can't push
    // WindowInfos to us (it bound the inputflinger service once at its own init,
    // cleared mInputFlinger when system_server died, and updateInputFlinger then
    // early-returns), so instead the wandr-arbiter (the window manager) authors the
    // window list and pushes it to our socket; we feed it to the dispatcher via
    // onWindowInfosChanged. Two pieces:
    //   1. a binder service the hosts register their (pid, channel-token) with, and
    status_t wreg = defaultServiceManager()->addService(String16("wandr.windowreg"),
                                                        sp<WandrWindowReg>::make());
    ALOGI("wandr-inputflinger: addService(wandr.windowreg) → %d", wreg);
    //   2. a socket listener that turns each arbiter push into onWindowInfosChanged.
    std::thread(window_feed_listener, im.get(), g_inputflinger_sock).detach();
    ALOGI("wandr-inputflinger: up — system keys → arbiter (dedup); app touch/keys → "
          "arbiter window-feed (%s)", g_inputflinger_sock.c_str());
    android::IPCThreadState::self()->joinThreadPool();
    return 0;
}
