// sf_surface — C ABI of the task-33 libgui surface shim (libsf_surface.so).
// wart-host dlopen()s the .so and dlsym()s these symbols; this header is
// the contract (and documentation for the Rust mirror in src/sf_surface.rs).
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct ANativeWindow;

// Allocate a fullscreen top-z-order SurfaceControl from SurfaceFlinger and
// return its ANativeWindow* (drive EGL on it). Writes the portrait logical
// dimensions to out_w/out_h and the SurfaceFlinger display rotation
// (ui::Rotation, 0..3) to out_transform, all if non-null. Returns NULL on
// failure.
struct ANativeWindow* sf_create_fullscreen_surface(int32_t* out_w,
                                                   int32_t* out_h,
                                                   uint32_t* out_transform);

// POD input event drained by sf_input_poll(). Mirrored in sf_surface.cpp and
// in the Rust side (src/sf_surface.rs) — keep all three in sync.
struct SfInputEvent {
    int32_t kind;        // 0=down 1=up 2=move 3=scroll  10=key-down 11=key-up
    int32_t pointer_id;  // multi-touch pointer id (0..N); 0 for key events
    float   x;
    float   y;
    float   pressure;    // 0.0..1.0; 0 for key events
    int32_t key_code;    // AKEYCODE_* for key events; 0 otherwise
    int32_t meta_state;  // shift/alt/ctrl bits (AMETA_*) for key events; 0 otherwise
};

// Drain pending InputFlinger events into `out` (capacity `max`); returns the
// count written. Non-blocking — call once per frame. Returns 0 if the input
// channel was never set up (e.g. inputflinger unavailable).
int32_t sf_input_poll(struct SfInputEvent* out, int32_t max);

// Re-request input focus for the wart window. The standalone runtime has no
// Activity, so any activity-backed window AMS resumes (com.android.launcher3,
// Messaging, …) steals InputDispatcher focus even though wart owns the z-top
// SurfaceFlinger layer. Call periodically from the host loop to keep key
// events flowing. Returns 0 on success, -1 on failure.
int32_t sf_request_focus(void);

// Query the live Android producer transform hint (NATIVE_WINDOW_TRANSFORM_HINT,
// a 0..7 bitmask: FLIP_H=1, FLIP_V=2, ROT_90=4). Call only AFTER the host's
// EGL producer has connected — the hint is unpopulated before then. Returns 0
// if the surface is down or the query fails.
uint32_t sf_query_transform_hint(void);

// Release the surface/control/client and input plumbing.
void sf_destroy_surface(void);

#ifdef __cplusplus
}
#endif
