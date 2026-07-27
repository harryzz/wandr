---
name: reference_dav1ddec_gstreamer_install
description: "How to build+install the dav1ddec (AV1 software) GStreamer plugin on Linux/WSL — it's NOT in Debian's plugins-bad"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-27T16:16:54.402Z
---

**AV1 software decode in GStreamer needs `dav1ddec`, which Debian/Ubuntu do NOT
ship in `gstreamer1.0-plugins-bad`.** It's the Rust `gst-plugin-dav1d` (lives in
`gst-plugins-rs`). Without it, `pick_decoder(Av1, false)` falls through to
`av1dec` (the aom *reference* decoder — very slow → AV1 playback is choppy/hiccups
on WSL). The project vendors the source at
`runtime/wandr-host/gst-plugins-rs`.

**Build + install (Linux/WSL):**
```bash
# 1. dav1d headers (runtime libdav1d7 is usually already present; -dev is not).
sudo apt install -y libdav1d-dev            # then `pkg-config --exists dav1d` == ok

# 2. Build the plugin from the in-repo gst-plugins-rs.
cd runtime/wandr-host/gst-plugins-rs/video/dav1d
cargo build --release                        # -> gst-plugins-rs/target/release/libgstdav1d.so (~19 MB)

# 3a. SYSTEM-WIDE (preferred — every run gets it, no env):
sudo cp ../../target/release/libgstdav1d.so /usr/lib/x86_64-linux-gnu/gstreamer-1.0/
# 3b. OR per-run, no sudo: point GST_PLUGIN_PATH at the build dir when launching the host:
#     export GST_PLUGIN_PATH="$PWD/../../target/release"

# 4. Verify.
gst-inspect-1.0 dav1ddec                      # Factory Details: Rank primary (256), "Dav1d AV1 Decoder"
```

Notes:
- `dav1ddec` is Rank **primary (256)**, so once registered it wins over `av1dec`
  automatically — `pick_decoder(Av1, false)` = `["dav1ddec","avdec_av1","av1dec"]`.
- Windows already had `dav1ddec` in its GStreamer install; this gap is
  Debian/WSL-specific.
- macOS: `brew install gstreamer` bundles it.
- Desktop-only (the Android backend uses MediaCodec, not GStreamer). See
  `[[reference_gstreamer_desktop_backend_spike]]`.
