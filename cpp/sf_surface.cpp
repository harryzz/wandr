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
#include <binder/ProcessState.h>
#include <ui/PixelFormat.h>
#include <ui/DisplayId.h>
#include <ui/Rect.h>
#include <ui/Rotation.h>
#include <utils/String8.h>

#include <android/native_window.h>
#include <android/log.h>

#include <cstdint>
#include <vector>

using namespace android;

namespace {
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO,  "sf_surface", __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, "sf_surface", __VA_ARGS__)

// Keep these alive for the process lifetime — dropping any of them
// invalidates the ANativeWindow* handed back to the caller.
sp<SurfaceComposerClient> g_client;
sp<SurfaceControl>        g_control;
sp<Surface>               g_surface;
sp<IBinder>               g_display;
}  // namespace

extern "C" {

// Allocate a fullscreen, top-z-order SurfaceControl from SurfaceFlinger and
// return its ANativeWindow* (a libgui Surface; the caller drives EGL on it).
// Writes the surface dimensions to out_w/out_h if non-null. Returns nullptr
// on failure.
ANativeWindow* sf_create_fullscreen_surface(int32_t* out_w, int32_t* out_h) {
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

    // Pixel 2 XL ("taimen") panel native mode is 1440x2880 portrait.
    // TODO(task33): query the mode instead of hardcoding.
    const uint32_t W = 1440, H = 2880;

    // Step 1 — force the display into portrait FIRST, synchronously.
    //
    // A standalone runtime has no WindowManager driving display orientation,
    // so the display's logical space stays wherever it last was (often
    // landscape 2880x1440). The runtime owns orientation itself. This MUST
    // happen before createSurface: a surface created against a landscape
    // display inherits a 90-degree transform hint, and EGL then renders a
    // swapped 2880x1440 buffer. apply(true) blocks until SurfaceFlinger has
    // processed the projection change. Later this becomes sensor-driven
    // (portrait OR landscape via the accelerometer / sensors HAL) — at which
    // point it moves behind a settable rotation entry point.
    {
        SurfaceComposerClient::Transaction t;
        t.setDisplayProjection(g_display, ui::ROTATION_0,
                               Rect(W, H), Rect(W, H));
        t.apply(/*synchronous=*/true);
    }

    // Step 2 — create the surface against the now-portrait display.
    g_control = g_client->createSurface(
        String8("wart"), W, H, PIXEL_FORMAT_RGBA_8888, 0);
    if (g_control == nullptr || !g_control->isValid()) {
        LOGE("createSurface failed");
        return nullptr;
    }

    // Step 3 — show it, top z-order.
    {
        SurfaceComposerClient::Transaction t;
        t.setLayer(g_control, 0x7fffffff);
        t.show(g_control);
        t.apply();
    }

    g_surface = g_control->getSurface();
    if (g_surface == nullptr) {
        LOGE("getSurface returned null");
        return nullptr;
    }

    // Step 4 — pin the producer buffer geometry to portrait, so EGL renders
    // 1440x2880 regardless of any stale rotation transform hint on the
    // Surface. Belt-and-suspenders with the projection change above.
    ANativeWindow_setBuffersGeometry(
        g_surface.get(), static_cast<int32_t>(W), static_cast<int32_t>(H),
        WINDOW_FORMAT_RGBA_8888);

    if (out_w) *out_w = static_cast<int32_t>(W);
    if (out_h) *out_h = static_cast<int32_t>(H);
    LOGI("fullscreen portrait surface created: %ux%u", W, H);
    return g_surface.get();
}

// Release the surface, control and client.
void sf_destroy_surface() {
    g_surface.clear();
    g_control.clear();
    g_display.clear();
    g_client.clear();
}

}  // extern "C"
