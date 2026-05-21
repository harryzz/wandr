// sf_surface — C ABI of the task-33 libgui surface shim (libsf_surface.so).
// wart-host dlopen()s the .so and dlsym()s these two symbols; this header is
// the contract (and documentation for the Rust mirror in src/sf_surface.rs).
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct ANativeWindow;

// Allocate a fullscreen top-z-order SurfaceControl from SurfaceFlinger and
// return its ANativeWindow* (drive EGL on it). Writes dimensions to
// out_w/out_h if non-null. Returns NULL on failure.
struct ANativeWindow* sf_create_fullscreen_surface(int32_t* out_w,
                                                   int32_t* out_h);

// Release the surface/control/client.
void sf_destroy_surface(void);

#ifdef __cplusplus
}
#endif
