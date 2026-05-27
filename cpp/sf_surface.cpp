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
#include <ui/DisplayMode.h>
#include <ui/LogicalDisplayId.h>
#include <ui/Rect.h>
#include <ui/Region.h>
#include <ui/Rotation.h>
#include <ui/Size.h>
#include <utils/String8.h>
#include <utils/Timers.h>

#include <android/native_window.h>
#include <android/input.h>
#include <android/log.h>
#include <system/window.h>

#include <algorithm>
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <vector>
#include <poll.h>

using namespace android;

// POD input event handed back across the C ABI by sf_input_poll(). Mirrored
// in sf_surface.h and in the Rust side (src/sf_surface.rs) — keep in sync.
struct SfInputEvent {
    int32_t kind;        // 0=down 1=up 2=move 3=scroll  10=key-down 11=key-up
    int32_t pointer_id;  // multi-touch pointer id (0..N); 0 for key events
    float   x;
    float   y;
    float   pressure;    // 0.0..1.0; 0 for key events
    int32_t key_code;    // AKEYCODE_* for key events; 0 otherwise
    int32_t meta_state;  // AMETA_* shift/alt/ctrl bitmask for key events; 0 otherwise
};

namespace {
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO,  "sf_surface", __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, "sf_surface", __VA_ARGS__)

// Panel dimensions in PORTRAIT orientation (width <= height). Populated
// by `init_panel_dims()` from `SurfaceComposerClient::getActiveDisplayMode`
// after the display token is resolved. Defaults to taimen (Pixel 2 XL)
// so the legacy hardcoded behavior still works if the query somehow
// fails — but every fresh process path passes through init_panel_dims
// and overrides these. Task 48.
uint32_t PANEL_W = 1440;
uint32_t PANEL_H = 2880;

// Query the active display mode and update PANEL_W / PANEL_H. The
// resolution is normalized to portrait (`min(w,h)` → width,
// `max(w,h)` → height) so the rest of the shim's portrait-coord
// assumptions hold on any display the runtime targets. Falls back to
// the taimen defaults on a failed query with a warning logged.
void init_panel_dims(const sp<IBinder>& display) {
    ui::DisplayMode mode;
    status_t st = SurfaceComposerClient::getActiveDisplayMode(display, &mode);
    if (st != OK) {
        LOGE("init_panel_dims: getActiveDisplayMode failed (%d) — "
             "keeping defaults %ux%u", st, PANEL_W, PANEL_H);
        return;
    }
    int32_t w = mode.resolution.width;
    int32_t h = mode.resolution.height;
    if (w <= 0 || h <= 0 || w > 8192 || h > 8192) {
        LOGE("init_panel_dims: implausible resolution %dx%d — "
             "keeping defaults %ux%u", w, h, PANEL_W, PANEL_H);
        return;
    }
    // Normalize to portrait. The shim's setDisplayProjection(ROTATION_0,
    // Rect(PW, PH), ...) treats PW as the short side; landscape-native
    // displays (taimen, some tablets) report (long, short) here.
    uint32_t pw = static_cast<uint32_t>(std::min(w, h));
    uint32_t ph = static_cast<uint32_t>(std::max(w, h));
    PANEL_W = pw;
    PANEL_H = ph;
    LOGI("init_panel_dims: panel resolution %dx%d → portrait %ux%u",
         w, h, PANEL_W, PANEL_H);
}

// Keep these alive for the process lifetime — dropping any of them
// invalidates the ANativeWindow* handed back to the caller.
sp<SurfaceComposerClient> g_client;
sp<SurfaceControl>        g_control;
sp<BLASTBufferQueue>      g_bbq;
sp<Surface>               g_surface;
sp<IBinder>               g_display;

// Task 47 step 3c — overlay parent container. Mirrors AOSP's
// BlastInputSurface pattern (frameworks/native/libs/gui/tests/
// EndToEndNativeInputTest.cpp): the buffer-state surface (g_control)
// is parented to a container-only SurfaceControl, and positioning /
// layering / cropping are applied to the PARENT. Setting position
// directly on the BBQ-backed buffer-state child does not stick;
// the parent inherits geometry, the child draws the buffer.
sp<SurfaceControl>        g_overlay_parent;

// Input plumbing (task 33 Step 3) — an InputFlinger input channel registered
// for the wart layer so touches in our window are dispatched to us.
std::shared_ptr<InputChannel>  g_input_channel;
std::unique_ptr<InputConsumer> g_input_consumer;
sp<gui::WindowInfoHandle>      g_window_info;

// Task 47 step 3c — overlay surfaces are positioned at (0, PANEL_H - H)
// on the panel, so InputDispatcher delivers display-coord events in
// the range [Y, PANEL_H] in Y. The guest expects surface-local coords
// (range [0, H]), so sf_input_poll subtracts this offset from motion
// events. 0 = fullscreen surface (no subtraction).
int32_t g_overlay_y_offset = 0;

// Register an InputFlinger input window for g_control so InputDispatcher
// routes touch events inside `rect` (in display coords) to our input
// channel. Recipe from
// frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp. Non-fatal:
// on any failure input is simply disabled (g_input_consumer stays null).
//
// `rect` is in DISPLAY coordinates. The fullscreen path passes
// Rect(0,0,PW,PH); the overlay path (task 47 step 3c) passes
// Rect(0,Y,PW,PH) for a bottom strip — that rect determines which
// taps InputDispatcher routes to us; events arrive in display coords,
// and sf_input_poll subtracts g_overlay_y_offset to give the guest
// surface-local coords.
void register_input_window_at(const Rect& rect, const char* name) {
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
    wi->name              = name;
    wi->globalScaleFactor = 1.0f;
    wi->frame             = rect;
    wi->touchableRegion.orSelf(rect);
    wi->displayId         = ui::LogicalDisplayId::DEFAULT;
    wi->applicationInfo.token = sp<BBinder>::make();
    wi->applicationInfo.name  = name;
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
    LOGI("input window '%s' registered at (%d,%d)-(%d,%d) (channel fd %d)",
         name, rect.left, rect.top, rect.right, rect.bottom,
         g_input_channel->getFd());
}

// Back-compat wrapper for the fullscreen path. Same behavior as the
// original `register_input_window(PW, PH)` — registers a Rect(0,0,PW,PH)
// input window named "wart". Kept so sf_create_fullscreen_surface's
// body is unchanged.
void register_input_window(uint32_t PW, uint32_t PH) {
    register_input_window_at(
        Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(PH)),
        "wart");
}

// Task 47 step 3c — update an EXISTING input window's bounds to
// `rect` (display coords). Used by sf_resize_overlay after the
// SurfaceControl is resized + repositioned. No-op if input wasn't
// registered (e.g. inputflinger was unavailable at create time).
void update_input_window_bounds(const Rect& rect) {
    if (g_window_info == nullptr) {
        return;
    }
    gui::WindowInfo* wi = g_window_info->editInfo();
    wi->frame = rect;
    wi->touchableRegion.clear();
    wi->touchableRegion.orSelf(rect);
    SurfaceComposerClient::Transaction t;
    t.setInputWindowInfo(g_control, g_window_info);
    t.apply(/*synchronous=*/true);
    LOGI("input window bounds updated to (%d,%d)-(%d,%d)",
         rect.left, rect.top, rect.right, rect.bottom);
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

    // Task 48 — populate PANEL_W/PANEL_H from the active display mode.
    // Pre-task 48 this was hardcoded to taimen's 1440x2880 portrait;
    // now any display the runtime targets works.
    init_panel_dims(g_display);
    const uint32_t PW = PANEL_W, PH = PANEL_H;  // local aliases for the rest of this fn

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

    // Step 3 — show it, top z-order, marked opaque; transform hint handling.
    //
    // eLayerOpaque tells SurfaceFlinger the layer fully covers its bounds, so
    // it does NOT blend whatever is behind it (the launcher) through pixels
    // the guest left transparent. It must be set via the transaction's
    // setFlags (a layer_state_t flag) — the createSurface `flags` parameter
    // uses a different, unrelated enum. The host pairs this by clearing the
    // surface to opaque black each frame (SkiaRenderer::begin_frame).
    //
    // Transform hint (task 33 orientation fix). The taimen panel is
    // physically landscape-native, so SurfaceFlinger hands this layer a
    // ROT_90 transform hint and EGL PRE-ROTATES — the producer's buffer is
    // transposed from the requested size. Rather than fight that, the host
    // now reads the real hint back via sf_query_transform_hint() and renders
    // pre-rotated to match it (the Android pre-rotation model). So by default
    // we do NOT pin the hint — SurfaceFlinger's natural hint flows through to
    // the producer and is queryable. WART_SF_HINT=<0..7>, if set, pins the
    // layer + client-cache hint to that value for on-device iteration:
    //   - setFixedTransformHint: SurfaceFlinger composites + reports it fixed.
    //   - g_control->setTransformHint: the client-side cache the BLASTBuffer
    //     queue forwards to the EGL producer (set ONCE at createSurface and
    //     never auto-updated by setFixedTransformHint), so it must be poked
    //     before the BBQ is constructed below.
    const char* pin_env = getenv("WART_SF_HINT");
    int pinned_hint = -1;
    if (pin_env != nullptr && pin_env[0] != '\0') {
        pinned_hint = atoi(pin_env);
    }
    {
        SurfaceComposerClient::Transaction t;
        t.setLayer(g_control, 0x7fffffff);
        if (pinned_hint >= 0) {
            t.setFixedTransformHint(g_control, pinned_hint);
        }
        t.setFlags(g_control, layer_state_t::eLayerOpaque,
                   layer_state_t::eLayerOpaque);
        t.show(g_control);
        t.apply(/*synchronous=*/true);
    }
    if (pinned_hint >= 0) {
        g_control->setTransformHint(pinned_hint);
        LOGI("transform hint pinned to %d (WART_SF_HINT)", pinned_hint);
    } else {
        LOGI("transform hint NOT pinned — SurfaceFlinger natural hint in use");
    }

    // Step 3b — attach a BLASTBufferQueue DIRECTLY to g_control.
    //
    // SurfaceControl::getSurface() would instead create an internal
    // "[BBQ] wart" CHILD SurfaceControl and put the buffer there. That child
    // is clipped to g_control's bounds — a parent/child clip we avoid by
    // owning the BBQ ourselves (the same call getSurface() makes internally,
    // minus the child). One layer, no parent/child clip — the BBQ buffer
    // composites full-screen; the host pre-rotates content per the queried
    // transform hint so the guest UI lands upright.
    g_bbq = sp<BLASTBufferQueue>::make(
        "wart", g_control, PW, PH, PIXEL_FORMAT_RGBA_8888);
    g_surface = g_bbq->getSurface(/*includeSurfaceControlHandle=*/true);
    if (g_surface == nullptr) {
        LOGE("BLASTBufferQueue getSurface returned null");
        return nullptr;
    }

    // Step 4 — register an InputFlinger input window (task 33 Step 3).
    register_input_window(PW, PH);

    // Report the portrait logical size. out_transform stays 0 here — the
    // real producer transform hint is only valid post-EGL-connect and is
    // read separately via sf_query_transform_hint().
    if (out_w) *out_w = static_cast<int32_t>(PW);
    if (out_h) *out_h = static_cast<int32_t>(PH);
    if (out_transform) *out_transform = 0;
    LOGI("surface created: portrait %ux%u logical (host reads the transform "
         "hint post-connect via sf_query_transform_hint)", PW, PH);
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
                // InputDispatcher delivers MotionEvent coordinates in
                // WINDOW-LOCAL space — the layer's display→window
                // transform (TRANSLATE 0,-Y for overlay layers) has
                // already been applied. So we pass m->getY through
                // verbatim; no offset subtraction needed.
                out[n].y          = m->getY(idx);
                out[n].pressure   = m->getPressure(idx);
                out[n].key_code   = 0;
                out[n].meta_state = 0;
            }
        } else if (ev->getType() == InputEventType::KEY) {
            KeyEvent* k = static_cast<KeyEvent*>(ev);
            switch (k->getAction()) {
                case AKEY_EVENT_ACTION_DOWN: out[n].kind = 10; emitted = true; break;
                case AKEY_EVENT_ACTION_UP:   out[n].kind = 11; emitted = true; break;
                // AKEY_EVENT_ACTION_MULTIPLE is deprecated; ignore.
                default: break;
            }
            if (emitted) {
                out[n].pointer_id = 0;
                out[n].x          = 0.0f;
                out[n].y          = 0.0f;
                out[n].pressure   = 0.0f;
                out[n].key_code   = k->getKeyCode();
                out[n].meta_state = k->getMetaState();
            }
        }
        g_input_consumer->sendFinishedSignal(seq, /*handled=*/true);
        if (emitted) {
            n++;
        }
    }
    return n;
}

// Re-request input focus for the wart window. The standalone runtime has
// no Activity, so any activity-backed window (com.android.launcher3,
// Messaging, etc) that AMS resumes will steal focus from InputDispatcher's
// point of view, even though wart owns the z-top SurfaceFlinger layer.
// Call this periodically from the host render loop to keep key events
// flowing to wart. Returns 0 on success, -1 on failure.
int32_t sf_request_focus() {
    if (g_window_info == nullptr) {
        return -1;
    }
    gui::WindowInfo* wi = g_window_info->editInfo();
    gui::FocusRequest fr;
    fr.token      = wi->token;
    fr.windowName = wi->name;
    fr.timestamp  = systemTime(SYSTEM_TIME_MONOTONIC);
    fr.displayId  = ui::LogicalDisplayId::DEFAULT.val();
    SurfaceComposerClient::Transaction t;
    t.setFocusedWindow(fr);
    t.apply(/*synchronous=*/false);
    return 0;
}

// Query the live Android producer transform hint
// (NATIVE_WINDOW_TRANSFORM_HINT, a 0..7 bitmask: FLIP_H=1, FLIP_V=2,
// ROT_90=4). Must be called AFTER the host's EGL producer connects — the
// hint is not populated before then. Returns 0 if the surface is not up or
// the query fails. The host (canvas_impl.rs) maps its base transform from
// this value.
uint32_t sf_query_transform_hint() {
    if (g_surface == nullptr) {
        return 0;
    }
    int v = 0;
    status_t st = g_surface->query(NATIVE_WINDOW_TRANSFORM_HINT, &v);
    if (st != OK) {
        LOGE("query(NATIVE_WINDOW_TRANSFORM_HINT) failed: %d", st);
        return 0;
    }
    LOGI("transform hint queried: %d", v);
    return static_cast<uint32_t>(v);
}

// Reposition the wart layer on the SurfaceFlinger z-axis (task 46 step 4/5).
// Higher z is drawn on top; default at creation is INT32_MAX. The arbiter
// pushes backgrounded apps to z=0 and pulls foreground to z=INT32_MAX —
// approximates AOSP's stacking with no shim source surface re-creation.
//
// Task 47 step 3c — for overlay surfaces, the geometry (position +
// layer + crop) lives on the PARENT container surface (BlastInputSurface
// pattern). Route setLayer to the parent so z-changes apply.
int32_t sf_set_layer(int32_t z) {
    sp<SurfaceControl> sc = g_overlay_parent != nullptr ? g_overlay_parent : g_control;
    if (sc == nullptr) {
        return -1;
    }
    SurfaceComposerClient::Transaction t;
    t.setLayer(sc, z);
    t.apply(/*synchronous=*/false);
    return 0;
}

// Toggle wart-layer visibility (task 46 step 4/5). Cheaper than re-creating
// the surface: the layer stays allocated, its BBQ keeps the last frame, and
// re-showing is one Transaction round-trip. The arbiter prefers this over
// killing/relaunching the app for background transitions.
//
// Task 47 step 3c — for overlay surfaces, route show/hide to the
// PARENT container (which carries geometry). The buffer-state child
// remains shown — toggling visibility of the parent hides/shows the
// whole subtree.
int32_t sf_set_visible(int32_t visible) {
    sp<SurfaceControl> sc = g_overlay_parent != nullptr ? g_overlay_parent : g_control;
    if (sc == nullptr) {
        return -1;
    }
    SurfaceComposerClient::Transaction t;
    if (visible) {
        t.show(sc);
    } else {
        t.hide(sc);
    }
    t.apply(/*synchronous=*/false);
    return 0;
}

// Task 47 step 3c — allocate a BOTTOM-STRIP overlay SurfaceControl of
// height `height_px` pixels (panel-width × height_px), positioned at
// `(0, PANEL_H - height_px)`. The input window is registered for that
// same bottom rect; the rest of the panel keeps dispatching to whoever
// owns those z-layers (typically wart-app's fullscreen surface).
//
// Starts INVISIBLE (`t.hide`) — the arbiter promotes to fg + flips
// visible explicitly via `sf_set_visible(1)` only when an editor
// focuses (cmd_overlay or auto-tied from cmd_attach_editor).
//
// Returns nullptr on bad height or any libgui error. `out_w` is set
// to PANEL_W; `out_h` to height_px; `out_transform` to 0 (host queries
// the live transform-hint via sf_query_transform_hint post-EGL-connect).
ANativeWindow* sf_create_overlay_surface(int32_t height_px,
                                          int32_t* out_w, int32_t* out_h,
                                          uint32_t* out_transform) {
    if (height_px <= 0 || height_px > static_cast<int32_t>(PANEL_H)) {
        LOGE("[overlay] sf_create_overlay_surface: bad height_px=%d "
             "(panel H=%u)", height_px, PANEL_H);
        return nullptr;
    }

    ProcessState::self()->startThreadPool();

    g_client = new SurfaceComposerClient();
    status_t err = g_client->initCheck();
    if (err != NO_ERROR) {
        LOGE("[overlay] SurfaceComposerClient initCheck failed: %d", err);
        return nullptr;
    }

    std::vector<PhysicalDisplayId> ids =
        SurfaceComposerClient::getPhysicalDisplayIds();
    if (ids.empty()) {
        LOGE("[overlay] no physical displays");
        return nullptr;
    }
    g_display = SurfaceComposerClient::getPhysicalDisplayToken(ids[0]);
    if (g_display == nullptr) {
        LOGE("[overlay] getPhysicalDisplayToken returned null");
        return nullptr;
    }

    // Task 48 — query active display mode (idempotent; safe to call
    // even if sf_create_fullscreen_surface ran first).
    init_panel_dims(g_display);
    const uint32_t PW = PANEL_W;
    const uint32_t PH = PANEL_H;
    const uint32_t H  = static_cast<uint32_t>(height_px);
    const int32_t  Y  = static_cast<int32_t>(PH - H);
    g_overlay_y_offset = Y;

    // Pin display projection to portrait identity — same reasoning as
    // sf_create_fullscreen_surface (clears any prior skew).
    {
        SurfaceComposerClient::Transaction t;
        t.setDisplayProjection(g_display, ui::ROTATION_0,
                               Rect(PW, PH), Rect(PW, PH));
        t.apply(/*synchronous=*/true);
    }

    // Parent container — AOSP BlastInputSurface pattern (test ref:
    // frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp).
    // Position / layer / crop go on this PARENT; the BBQ-backed
    // buffer-state child inherits geometry. Setting position on the
    // buffer-state child directly was empirically a no-op on this
    // device.
    g_overlay_parent = g_client->createSurface(
        String8("wart-ime-overlay-parent"),
        /*w=*/0, /*h=*/0, PIXEL_FORMAT_RGBA_8888,
        ISurfaceComposerClient::eFXSurfaceContainer);
    if (g_overlay_parent == nullptr || !g_overlay_parent->isValid()) {
        LOGE("[overlay] createSurface(parent container) failed");
        return nullptr;
    }

    // Buffer-state child, parented to the container. PW × H buffer;
    // the parent supplies position + crop.
    g_control = g_client->createSurface(
        String8("wart-ime-overlay"), PW, H, PIXEL_FORMAT_RGBA_8888,
        ISurfaceComposerClient::eFXSurfaceBufferState,
        g_overlay_parent->getHandle());
    if (g_control == nullptr || !g_control->isValid()) {
        LOGE("[overlay] createSurface(buffer child) failed");
        return nullptr;
    }

    // Same WART_SF_HINT env-var handling as fullscreen. Default: SF's
    // natural transform hint flows through to the producer.
    const char* pin_env = getenv("WART_SF_HINT");
    int pinned_hint = -1;
    if (pin_env != nullptr && pin_env[0] != '\0') {
        pinned_hint = atoi(pin_env);
    }
    {
        SurfaceComposerClient::Transaction t;
        // Parent: position + crop + layer (the geometry parts).
        // Initial z: just below i32::MAX. The arbiter's promote-to-
        // overlay calls sf_set_layer(i32::MAX) which we route to the
        // parent now (see sf_set_layer below).
        t.setLayer(g_overlay_parent, 0x7fffffff - 1);
        t.setPosition(g_overlay_parent, 0.0f, static_cast<float>(Y));
        t.setCrop(g_overlay_parent,
                  Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)));
        // Start HIDDEN — arbiter shows on attach-editor.
        t.hide(g_overlay_parent);

        // Child: flags + crop + show (the buffer parts).
        if (pinned_hint >= 0) {
            t.setFixedTransformHint(g_control, pinned_hint);
        }
        t.setFlags(g_control, layer_state_t::eLayerOpaque,
                   layer_state_t::eLayerOpaque);
        t.setCrop(g_control,
                  Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)));
        t.show(g_control);
        t.apply(/*synchronous=*/true);
    }
    if (pinned_hint >= 0) {
        g_control->setTransformHint(pinned_hint);
        LOGI("[overlay] transform hint pinned to %d (WART_SF_HINT)", pinned_hint);
    }

    g_bbq = sp<BLASTBufferQueue>::make(
        "wart-ime-overlay", g_control, PW, H, PIXEL_FORMAT_RGBA_8888);
    g_surface = g_bbq->getSurface(/*includeSurfaceControlHandle=*/true);
    if (g_surface == nullptr) {
        LOGE("[overlay] BLASTBufferQueue getSurface returned null");
        return nullptr;
    }

    // Input window for the bottom strip only. Registered against the
    // child (BBQ buffer). The touchableRegion is in LAYER-LOCAL
    // coords: SurfaceFlinger adds the layer's position to convert to
    // display coords. With layer at (0, Y), passing Rect(0, 0, PW, H)
    // yields display-coord touchable region (0, Y) → (PW, Y + H) =
    // exactly the visible overlay strip.
    //
    // Bug found in initial smoke (task 49 step 2): passing
    // Rect(0, Y, PW, PH) yielded display coords (0, 2Y) → (PW, Y+PH)
    // = off-screen, so taps in the overlay area fell through to
    // wart-app's fullscreen input window. The IME never saw any
    // touches; wart-app's in-canvas keyboard got them.
    register_input_window_at(
        Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)),
        "wart-ime");

    if (out_w) *out_w = static_cast<int32_t>(PW);
    if (out_h) *out_h = static_cast<int32_t>(H);
    if (out_transform) *out_transform = 0;
    LOGI("[overlay] surface created: %ux%u logical at (0,%d), panel %ux%u",
         PW, H, Y, PW, PH);
    return g_surface.get();
}

// Task 47 step 3c — resize the overlay SurfaceControl to `new_height_px`
// pixels tall (panel-width unchanged). Re-positions to
// `(0, PANEL_H - new_height_px)`, updates the BLASTBufferQueue's buffer
// dimensions, re-registers the input window at the new rect, and
// updates g_overlay_y_offset so subsequent sf_input_poll calls
// translate motion-event Y values correctly. The Rust side calls
// `ANativeWindow_setBuffersGeometry` after this returns to flush
// EGL/Skia's view of the new dimensions.
//
// Returns 0 on success, -1 if the surface isn't an overlay (or
// not yet created), -2 if `new_height_px` is out of range.
int32_t sf_resize_overlay(int32_t new_height_px) {
    if (g_control == nullptr || g_bbq == nullptr) {
        return -1;
    }
    if (new_height_px <= 0 || new_height_px > static_cast<int32_t>(PANEL_H)) {
        LOGE("[overlay] sf_resize_overlay: bad new_height_px=%d (panel H=%u)",
             new_height_px, PANEL_H);
        return -2;
    }
    const uint32_t PW = PANEL_W;
    const uint32_t PH = PANEL_H;
    const uint32_t H  = static_cast<uint32_t>(new_height_px);
    const int32_t  Y  = static_cast<int32_t>(PH - H);
    g_overlay_y_offset = Y;

    {
        SurfaceComposerClient::Transaction t;
        // Position + crop go on the parent container. Crop is the
        // sub-rect of the parent that's visible; we want the full
        // PW × H bottom strip.
        if (g_overlay_parent != nullptr) {
            t.setPosition(g_overlay_parent, 0.0f, static_cast<float>(Y));
            t.setCrop(g_overlay_parent,
                      Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)));
        }
        // Child crop also updates so the buffer-state surface knows
        // its bounded region matches the new H.
        t.setCrop(g_control,
                  Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)));
        t.apply(/*synchronous=*/true);
    }
    // Refresh the BBQ's notion of buffer dimensions. Without this it
    // keeps producing PW×OldH buffers that SF clips/distorts.
    g_bbq->update(g_control, PW, H, PIXEL_FORMAT_RGBA_8888);

    // Re-register the input window at the new bounds. Layer-local
    // coords — SF adds the layer's position (which is now Y) to
    // convert to display coords. See the create path's comment for
    // the layer-local-vs-display bug history.
    update_input_window_bounds(
        Rect(0, 0, static_cast<int32_t>(PW), static_cast<int32_t>(H)));

    LOGI("[overlay] resized to %ux%u at (0,%d)", PW, H, Y);
    return 0;
}

// Release the surface, control, client and input plumbing.
void sf_destroy_surface() {
    g_input_consumer.reset();
    g_input_channel.reset();
    g_window_info.clear();
    g_surface.clear();
    g_bbq.clear();
    g_control.clear();
    g_overlay_parent.clear();
    g_display.clear();
    g_client.clear();
    g_overlay_y_offset = 0;
}

}  // extern "C"
