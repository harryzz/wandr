// sf_surface — task 33 Step 1/2: the reusable libgui surface shim.
//
// sf_probe.cpp's proven SurfaceFlinger path, as an `extern "C"` shared
// library (libsf_surface.so). The wart-host standalone runtime dlopen()s it
// and calls sf_create_fullscreen_surface() to obtain a fullscreen
// ANativeWindow* with no NativeActivity, then drives EGL/Skia on it.
//
// Built IN-TREE as a soong cc_library_shared (see sf_surface.bp) — libgui's
// headers cannot be consumed out-of-tree. See memory
// project-boot-model-libgui-build and tasks/33-boot-model-bringup.md.

#include <gui/SurfaceComposerClient.h>
#include <gui/SurfaceControl.h>
#include <gui/Surface.h>
#include <gui/BLASTBufferQueue.h>
#include <gui/LayerState.h>
#include <gui/WindowInfo.h>
#include <binder/ProcessState.h>
#include <binder/IServiceManager.h>
#include <binder/Binder.h>
#include <android/os/IInputFlinger.h>
#include <android/gui/FocusRequest.h>
#include <input/Input.h>
#include <input/InputConsumer.h>
#include <input/InputTransport.h>
#include <ui/PixelFormat.h>
#include <ui/DisplayId.h>
#include <ui/LogicalDisplayId.h>
#include <ui/Rect.h>
#include <ui/Region.h>
#include <ui/Rotation.h>
#include <utils/String8.h>
#include <utils/Timers.h>

#include <android/native_window.h>
#include <android/input.h>
#include <android/log.h>

#include <cstdint>
#include <memory>
#include <vector>
#include <poll.h>

using namespace android;

// POD input event handed back across the C ABI by sf_input_poll(). Mirrored
// in sf_surface.h and in the Rust side (src/sf_surface.rs) — keep in sync.
struct SfInputEvent {
    int32_t kind;        // 0=down 1=up 2=move 3=scroll
    int32_t pointer_id;  // multi-touch pointer id (0..N)
    float   x;
    float   y;
    float   pressure;    // 0.0..1.0
    int32_t key_code;    // reserved — key events not emitted in this cut
};

namespace {
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO,  "sf_surface", __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, "sf_surface", __VA_ARGS__)

// Keep these alive for the process lifetime — dropping any of them
// invalidates the ANativeWindow* handed back to the caller.
sp<SurfaceComposerClient> g_client;
sp<SurfaceControl>        g_control;
sp<BLASTBufferQueue>      g_bbq;
sp<Surface>               g_surface;
sp<IBinder>               g_display;

// Input plumbing (task 33 Step 3) — an InputFlinger input channel registered
// for the wart layer so touches in our window are dispatched to us.
std::shared_ptr<InputChannel>  g_input_channel;
std::unique_ptr<InputConsumer> g_input_consumer;
sp<gui::WindowInfoHandle>      g_window_info;

// Register an InputFlinger input window for g_control so InputDispatcher
// routes touch events inside the panel to our input channel. Recipe from
// frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp. Non-fatal:
// on any failure input is simply disabled (g_input_consumer stays null).
void register_input_window(uint32_t PW, uint32_t PH) {
    sp<IBinder> binder =
        defaultServiceManager()->waitForService(String16("inputflinger"));
    sp<os::IInputFlinger> inputFlinger = interface_cast<os::IInputFlinger>(binder);
    if (inputFlinger == nullptr) {
        LOGE("inputflinger service unavailable — input disabled");
        return;
    }

    os::InputChannelCore channelCore;
    binder::Status st =
        inputFlinger->createInputChannel("wart input", &channelCore);
    if (!st.isOk()) {
        LOGE("createInputChannel failed — input disabled");
        return;
    }
    g_input_channel = std::shared_ptr<InputChannel>(
        InputChannel::create(std::move(channelCore)));
    g_input_consumer = std::make_unique<InputConsumer>(g_input_channel);

    g_window_info = sp<gui::WindowInfoHandle>::make();
    gui::WindowInfo* wi = g_window_info->editInfo();
    wi->token             = g_input_channel->getConnectionToken();
    wi->name              = "wart";
    wi->globalScaleFactor = 1.0f;
    wi->frame             = Rect(0, 0, static_cast<int32_t>(PW),
                                       static_cast<int32_t>(PH));
    wi->touchableRegion.orSelf(Rect(0, 0, static_cast<int32_t>(PW),
                                          static_cast<int32_t>(PH)));
    wi->displayId         = ui::LogicalDisplayId::DEFAULT;
    wi->applicationInfo.token = sp<BBinder>::make();
    wi->applicationInfo.name  = "wart";
    wi->applicationInfo.dispatchingTimeoutMillis = 5000;

    SurfaceComposerClient::Transaction t;
    t.setInputWindowInfo(g_control, g_window_info);
    gui::FocusRequest fr;
    fr.token      = wi->token;
    fr.windowName = wi->name;
    fr.timestamp  = systemTime(SYSTEM_TIME_MONOTONIC);
    fr.displayId  = ui::LogicalDisplayId::DEFAULT.val();
    t.setFocusedWindow(fr);
    t.apply(/*synchronous=*/true);
    LOGI("input window registered (channel fd %d)", g_input_channel->getFd());
}
}  // namespace

extern "C" {

// Allocate a fullscreen, top-z-order SurfaceControl from SurfaceFlinger and
// return its ANativeWindow* (a libgui Surface; the caller drives EGL on it).
// Writes the portrait logical dimensions to out_w/out_h and the SurfaceFlinger
// display rotation (ui::Rotation, 0..3) to out_transform, all if non-null.
// Returns nullptr on failure.
ANativeWindow* sf_create_fullscreen_surface(int32_t* out_w, int32_t* out_h,
                                            uint32_t* out_transform) {
    ProcessState::self()->startThreadPool();

    g_client = new SurfaceComposerClient();
    status_t err = g_client->initCheck();
    if (err != NO_ERROR) {
        LOGE("SurfaceComposerClient initCheck failed: %d", err);
        return nullptr;
    }

    std::vector<PhysicalDisplayId> ids =
        SurfaceComposerClient::getPhysicalDisplayIds();
    if (ids.empty()) {
        LOGE("no physical displays");
        return nullptr;
    }
    g_display = SurfaceComposerClient::getPhysicalDisplayToken(ids[0]);
    if (g_display == nullptr) {
        LOGE("getPhysicalDisplayToken returned null");
        return nullptr;
    }

    // Pixel 2 XL ("taimen") panel: 1440x2880 portrait physical display.
    // TODO(task33): query the mode instead of hardcoding.
    const uint32_t PW = 1440, PH = 2880;  // physical panel (portrait)

    // Step 1 — pin the display projection to portrait identity.
    //
    // ROTATION_0 with a portrait layer stack == the panel: this rotates
    // nothing, it just resets the projection in case a prior run left it
    // skewed (setDisplayProjection state persists across process exit). It
    // must NOT carry a rotation — that is a global display change that would
    // rotate the launcher / SystemUI too.
    {
        SurfaceComposerClient::Transaction t;
        t.setDisplayProjection(g_display, ui::ROTATION_0,
                               Rect(PW, PH), Rect(PW, PH));
        t.apply(/*synchronous=*/true);
    }

    // Step 2 — create the surface, PORTRAIT PWxPH (1440x2880).
    //
    // With the transform hint pinned to ROT_0 (Step 3, setFixedTransformHint)
    // there is no auto-prerotation, so the layer, the BLASTBufferQueue and the
    // EGL buffer are all the same portrait 1440x2880 — matching the portrait
    // panel and composition space 1:1, guest renders with an identity matrix.
    g_control = g_client->createSurface(
        String8("wart"), PW, PH, PIXEL_FORMAT_RGBA_8888, 0);
    if (g_control == nullptr || !g_control->isValid()) {
        LOGE("createSurface failed");
        return nullptr;
    }

    // Step 3 — show it, top z-order, marked opaque, transform hint pinned.
    //
    // eLayerOpaque tells SurfaceFlinger the layer fully covers its bounds, so
    // it does NOT blend whatever is behind it (the launcher) through pixels
    // the guest left transparent. It must be set via the transaction's
    // setFlags (a layer_state_t flag) — the createSurface `flags` parameter
    // uses a different, unrelated enum. The host pairs this by clearing the
    // surface to opaque black each frame (SkiaRenderer::begin_frame).
    //
    // Transform hint, ROT_0 — the keystone. The taimen panel is physically
    // landscape-native, so SurfaceFlinger would otherwise hand this layer a
    // ROT_90 transform hint; EGL reacts to that hint and PRE-ROTATES — it
    // hands the producer a buffer transposed from the requested size, the
    // guest UI ends up 90deg off. Two calls pin it to ROT_0:
    //   - setFixedTransformHint(ROT_0): SurfaceFlinger composites the layer
    //     with a fixed ROT_0 hint and reports ROT_0 in frame stats.
    //   - g_control->setTransformHint(0): the client-side SurfaceControl hint
    //     cache is set ONCE at createSurface (to the natural ROT_90) and never
    //     auto-updates — setFixedTransformHint does NOT refresh it. The
    //     BLASTBufferQueue below reads exactly this cache
    //     (mSurfaceControl->getTransformHint()) and forwards it to the
    //     producer, so EGL sees whatever is here. It must be 0 before the BBQ
    //     is constructed, hence this explicit poke.
    // Net: EGL gets hint 0, no prerotation, buffer == requested portrait
    // 1440x2880, host renders identity. SurfaceFlinger rotates for the
    // landscape panel itself (one composite step — fine for one fullscreen
    // layer).
    {
        SurfaceComposerClient::Transaction t;
        t.setLayer(g_control, 0x7fffffff);
        t.setFixedTransformHint(g_control, 0 /* ui::ROTATION_0 */);
        t.setFlags(g_control, layer_state_t::eLayerOpaque,
                   layer_state_t::eLayerOpaque);
        t.show(g_control);
        t.apply(/*synchronous=*/true);
    }
    g_control->setTransformHint(0);

    // Step 3b — attach a BLASTBufferQueue DIRECTLY to g_control.
    //
    // SurfaceControl::getSurface() would instead create an internal
    // "[BBQ] wart" CHILD SurfaceControl and put the buffer there. That child
    // is clipped to g_control's bounds — a parent/child clip we avoid by
    // owning the BBQ ourselves (the same call getSurface() makes internally,
    // minus the child). One layer, no parent/child clip; with the ROT_0 hint
    // the BBQ's portrait 1440x2880 buffer fills the panel upright.
    g_bbq = sp<BLASTBufferQueue>::make(
        "wart", g_control, PW, PH, PIXEL_FORMAT_RGBA_8888);
    g_surface = g_bbq->getSurface(/*includeSurfaceControlHandle=*/true);
    if (g_surface == nullptr) {
        LOGE("BLASTBufferQueue getSurface returned null");
        return nullptr;
    }

    // Step 4 — register an InputFlinger input window (task 33 Step 3).
    register_input_window(PW, PH);

    // The caller authors a portrait UI 1:1 with the portrait panel: report the
    // portrait size and transform 0 (no rotation — the buffer already matches).
    if (out_w) *out_w = static_cast<int32_t>(PW);
    if (out_h) *out_h = static_cast<int32_t>(PH);
    if (out_transform) *out_transform = 0;
    LOGI("surface created: portrait %ux%u layer+buffer (transform hint pinned "
         "ROT_0, no prerotation) matching the panel", PW, PH);
    return g_surface.get();
}

// Drain pending input events into `out` (capacity `max`); returns the count
// written. Non-blocking — call once per frame from the render loop. Each
// consumed InputFlinger event is decoded to the action pointer and finished.
// Returns 0 if input was never set up.
int32_t sf_input_poll(SfInputEvent* out, int32_t max) {
    if (g_input_consumer == nullptr || out == nullptr || max <= 0) {
        return 0;
    }
    static PreallocatedInputEventFactory factory;
    int32_t n = 0;
    while (n < max) {
        InputEvent* ev = nullptr;
        uint32_t seq = 0;
        status_t st = g_input_consumer->consume(
            &factory, /*consumeBatches=*/true, /*frameTime=*/-1, &seq, &ev);
        if (st != OK || ev == nullptr) {
            break;  // WOULD_BLOCK — nothing more pending
        }
        bool emitted = false;
        if (ev->getType() == InputEventType::MOTION) {
            MotionEvent* m = static_cast<MotionEvent*>(ev);
            size_t idx = 0;
            switch (m->getActionMasked()) {
                case AMOTION_EVENT_ACTION_DOWN:
                case AMOTION_EVENT_ACTION_POINTER_DOWN:
                    out[n].kind = 0; idx = m->getActionIndex(); emitted = true;
                    break;
                case AMOTION_EVENT_ACTION_UP:
                case AMOTION_EVENT_ACTION_POINTER_UP:
                case AMOTION_EVENT_ACTION_CANCEL:
                    out[n].kind = 1; idx = m->getActionIndex(); emitted = true;
                    break;
                case AMOTION_EVENT_ACTION_MOVE:
                    out[n].kind = 2; idx = 0; emitted = true;
                    break;
                case AMOTION_EVENT_ACTION_SCROLL:
                    out[n].kind = 3; idx = 0; emitted = true;
                    break;
                default:
                    break;
            }
            if (emitted) {
                out[n].pointer_id = m->getPointerId(idx);
                out[n].x          = m->getX(idx);
                out[n].y          = m->getY(idx);
                out[n].pressure   = m->getPressure(idx);
                out[n].key_code   = 0;
            }
        }
        g_input_consumer->sendFinishedSignal(seq, /*handled=*/true);
        if (emitted) {
            n++;
        }
    }
    return n;
}

// Release the surface, control, client and input plumbing.
void sf_destroy_surface() {
    g_input_consumer.reset();
    g_input_channel.reset();
    g_window_info.clear();
    g_surface.clear();
    g_bbq.clear();
    g_control.clear();
    g_display.clear();
    g_client.clear();
}

}  // extern "C"
