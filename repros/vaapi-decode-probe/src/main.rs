//! VA-API hardware H.264 decode probe — cros-libva-DIRECT surfaces (task 117 M2).
//!
//! WHY NOT cros-codecs' own frame types: its only `VideoFrame` impls are GBM/DMA
//! backed, and GBM allocation fails on BOTH available machines for unrelated
//! reasons (fedora Ivybridge i915 rejects `GBM_BO_USE_HW_VIDEO_DECODER`
//! contiguous NV12; WSL's DRM node is *vgem*, a dummy device whose real GPU
//! memory lives behind /dev/dxg). VA-API itself works on both. So we implement
//! `VideoFrame` over a **VA-allocated `Surface<()>`** — `vaCreateSurfaces`, no
//! GBM anywhere — and keep cros-codecs only for its H.264 parser + DPB/reference
//! management (the genuinely hard part).
//!
//! OUTPUT TIERS (probed and logged; we currently CONSUME tier 3):
//!   1. zero-copy  — `Surface::export_prime()` -> DMA-buf fd handed to the
//!                   compositor/GL. No CPU copy. Needs driver `DRM_PRIME_2`.
//!   2. derive     — `Image::derive_from` (vaDeriveImage): direct map of the
//!                   surface, usually cheap.
//!   3. copy       — `Image::create_from` (vaGetImage): explicit copy. Always
//!                   available; what we read back today.
//! Tier 1's CPU saving only materializes once the host consumes a texture
//! directly (the zero-copy `present(at-ns)` path); today the `I420Ref` contract
//! forces a readback regardless, so tier 3 is wired end to end and 1/2 are
//! probed so we can see what each machine supports.
//!
//! Input: raw H.264 **Annex-B** elementary stream. Run:
//!   LIBVA_DRIVER_NAME=d3d12 MESA_LOADER_DRIVER_OVERRIDE=vgem GALLIUM_DRIVER=d3d12 \
//!   WANDR_DRM_DEVICE=/dev/dri/card0 vaapi-decode-probe bbb.h264 100

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cros_codecs::backend::vaapi::decoder::VaapiBackend;
use cros_codecs::bitstream_utils::NalIterator;
use cros_codecs::codec::h264::parser::Nalu as H264Nalu;
use cros_codecs::decoder::stateless::h264::H264;
use cros_codecs::decoder::stateless::{DecodeError, StatelessDecoder, StatelessVideoDecoder};
use cros_codecs::decoder::{BlockingMode, DecodedHandle, DecoderEvent};
use cros_codecs::video_frame::{ReadMapping, VideoFrame, WriteMapping};
use cros_codecs::{DecodedFormat, Fourcc, Resolution};
use libva::{Display, Surface, UsageHint};

// ── a VideoFrame backed by a VA-allocated surface (no GBM) ───────────────────

/// Placeholder frame. The real pixels live in the VA surface that
/// `to_native_handle` allocates; we read them back off the decoded handle's
/// surface (see `readback`), so `map()` is never used.
#[derive(Debug)]
struct VaSurfaceFrame {
    resolution: Resolution,
}

impl VideoFrame for VaSurfaceFrame {
    type MemDescriptor = (); // () = "VA allocates the memory itself"
    type NativeHandle = Surface<()>;

    fn fourcc(&self) -> Fourcc {
        Fourcc::from(b"NV12")
    }
    fn resolution(&self) -> Resolution {
        self.resolution
    }
    fn get_plane_size(&self) -> Vec<usize> {
        let (w, h) = (self.resolution.width as usize, self.resolution.height as usize);
        vec![w * h, w * h / 2]
    }
    fn get_plane_pitch(&self) -> Vec<usize> {
        let w = self.resolution.width as usize;
        vec![w, w]
    }
    fn map<'a>(&'a self) -> Result<Box<dyn ReadMapping<'a> + 'a>, String> {
        Err("VaSurfaceFrame: read back via the decoded handle's surface".into())
    }
    fn map_mut<'a>(&'a mut self) -> Result<Box<dyn WriteMapping<'a> + 'a>, String> {
        Err("VaSurfaceFrame is decode-output only".into())
    }
    /// Let VA allocate the decode target — this is the whole point: no GBM.
    fn to_native_handle(&self, display: &Rc<Display>) -> Result<Self::NativeHandle, String> {
        display
            .create_surfaces(
                libva::VA_RT_FORMAT_YUV420,
                Some(u32::from(self.fourcc())),
                self.resolution.width,
                self.resolution.height,
                Some(UsageHint::USAGE_HINT_DECODER),
                vec![()],
            )
            .map_err(|e| format!("vaCreateSurfaces failed: {e:?}"))?
            .pop()
            .ok_or_else(|| "vaCreateSurfaces returned no surface".to_string())
    }
}

// ── capability probing: report what each tier can do on this machine ─────────

fn probe_tiers(display: &Rc<Display>) {
    let mut config = match display.create_config(
        vec![libva::VAConfigAttrib {
            type_: libva::VAConfigAttribType::VAConfigAttribRTFormat,
            value: libva::VA_RT_FORMAT_YUV420,
        }],
        libva::VAProfile::VAProfileH264Main,
        libva::VAEntrypoint::VAEntrypointVLD,
    ) {
        Ok(c) => c,
        Err(e) => {
            println!("tier-probe: cannot create H264 config: {e:?}");
            return;
        }
    };

    let ints = |cfg: &mut libva::Config, t| -> Vec<i32> {
        cfg.query_surface_attributes_by_type(t)
            .map(|v| {
                v.into_iter()
                    .filter_map(|g| match g {
                        libva::GenericValue::Integer(i) => Some(i),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let min_w = ints(&mut config, libva::VASurfaceAttribType::VASurfaceAttribMinWidth);
    let min_h = ints(&mut config, libva::VASurfaceAttribType::VASurfaceAttribMinHeight);
    let max_w = ints(&mut config, libva::VASurfaceAttribType::VASurfaceAttribMaxWidth);
    let max_h = ints(&mut config, libva::VASurfaceAttribType::VASurfaceAttribMaxHeight);
    println!("driver resolution limits: min={min_w:?}x{min_h:?} max={max_w:?}x{max_h:?}");

    let mem = ints(&mut config, libva::VASurfaceAttribType::VASurfaceAttribMemoryType);
    let bits = mem.first().copied().unwrap_or(0) as u32;
    let named = [
        ("VA", libva::VA_SURFACE_ATTRIB_MEM_TYPE_VA),
        ("USER_PTR", libva::VA_SURFACE_ATTRIB_MEM_TYPE_USER_PTR),
        ("KERNEL_DRM", libva::VA_SURFACE_ATTRIB_MEM_TYPE_KERNEL_DRM),
        ("DRM_PRIME", libva::VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME),
        ("DRM_PRIME_2", libva::VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2),
    ];
    let supported: Vec<&str> =
        named.iter().filter(|(_, b)| bits & b != 0).map(|(n, _)| *n).collect();
    println!("driver memory types: {supported:?} (raw 0x{bits:08x})");

    // Allocate one real surface and try each readback tier against it.
    let probe_res = (640u32, 480u32);
    let surface = match display.create_surfaces::<()>(
        libva::VA_RT_FORMAT_YUV420,
        Some(u32::from(Fourcc::from(b"NV12"))),
        probe_res.0,
        probe_res.1,
        Some(UsageHint::USAGE_HINT_DECODER),
        vec![()],
    ) {
        Ok(mut v) => match v.pop() {
            Some(s) => s,
            None => {
                println!("tier-probe: no surface returned");
                return;
            }
        },
        Err(e) => {
            println!("tier-probe: vaCreateSurfaces failed: {e:?}");
            return;
        }
    };

    let t1 = surface.export_prime();
    println!(
        "  tier 1 (zero-copy export_prime -> DMA-buf): {}",
        match &t1 {
            Ok(d) => format!("AVAILABLE ({} dma-buf objects)", d.objects.len()),
            Err(e) => format!("unavailable ({e:?})"),
        }
    );
    drop(t1);

    let t2 = libva::Image::derive_from(&surface, probe_res);
    println!(
        "  tier 2 (derive_from / vaDeriveImage):       {}",
        match &t2 {
            Ok(_) => "AVAILABLE".to_string(),
            Err(e) => format!("unavailable ({e:?})"),
        }
    );
    drop(t2);
    println!("  tier 3 (create_from / vaGetImage copy):      USED (always available)");
}

// ── tier-3 readback: vaGetImage the decoded surface into NV12 bytes ──────────

/// Returns (width, height, mean luma) for the decoded surface.
fn readback_mean_luma(surface: &Surface<()>, res: (u32, u32)) -> Result<f64, String> {
    // Tier 3. derive_from is tried first since when it works it is cheaper; both
    // land in the same VAImage shape.
    let image = libva::Image::derive_from(surface, res)
        .map_err(|e| format!("derive_from failed: {e:?}"))?;
    let va_image = *image.image();
    let data: &[u8] = image.as_ref();
    let y_off = va_image.offsets[0] as usize;
    let y_pitch = va_image.pitches[0] as usize;
    let (w, h) = (res.0 as usize, res.1 as usize);

    let mut acc: u64 = 0;
    let mut n: u64 = 0;
    let mut row = 0usize;
    while row < h {
        let base = y_off + row * y_pitch;
        let mut x = 0usize;
        while x < w {
            if let Some(&px) = data.get(base + x) {
                acc += px as u64;
                n += 1;
            }
            x += 17; // sparse sample
        }
        row += 7;
    }
    if n == 0 {
        return Err("no luma sampled".into());
    }
    Ok(acc as f64 / n as f64)
}

fn main() {
    env_logger::init();

    let mut args = std::env::args().skip(1);
    let input_path = args.next().unwrap_or_else(|| {
        eprintln!("usage: vaapi-decode-probe <in.h264> [min_frames]");
        std::process::exit(2);
    });
    let min_frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let input = std::fs::read(&input_path).expect("read input .h264");

    // cros-libva's Display::open() only scans renderD128..191; on WSL the usable
    // node is the vgem card0, so allow an explicit override.
    let display = match std::env::var("WANDR_DRM_DEVICE") {
        Ok(p) => Display::open_drm_display(PathBuf::from(&p))
            .unwrap_or_else(|e| panic!("open libva display {p}: {e:?}")),
        Err(_) => Display::open().expect("open libva display"),
    };
    println!("VA display opened; probing capabilities…\n");
    probe_tiers(&display);
    println!();

    let mut decoder = StatelessDecoder::<H264, VaapiBackend<VaSurfaceFrame>>::new_vaapi(
        Rc::clone(&display),
        BlockingMode::Blocking,
    )
    .expect("create H264 VAAPI decoder");

    // The frame pool is trivial here: the decoder tells us the stream resolution
    // via FormatChanged, and each alloc hands back a placeholder whose
    // to_native_handle allocates a fresh VA surface.
    let res = Rc::new(RefCell::new(Resolution::from((0, 0))));

    let mut frames = 0usize;
    let mut nonblack = false;
    let mut dims = (0u32, 0u32);

    let mut drain = |dec: &mut StatelessDecoder<H264, VaapiBackend<VaSurfaceFrame>>,
                     frames: &mut usize,
                     nonblack: &mut bool,
                     dims: &mut (u32, u32),
                     res: &Rc<RefCell<Resolution>>| {
        while let Some(ev) = dec.next_event() {
            match ev {
                DecoderEvent::FrameReady(handle) => {
                    handle.sync().expect("sync decoded frame");
                    let r = handle.display_resolution();
                    *dims = (r.width, r.height);
                    let inner = handle.borrow();
                    match readback_mean_luma(inner.surface(), (r.width, r.height)) {
                        Ok(mean) => {
                            if mean > 2.0 {
                                *nonblack = true;
                            }
                        }
                        Err(e) => eprintln!("readback: {e}"),
                    }
                    *frames += 1;
                }
                DecoderEvent::FormatChanged => {
                    if let Some(info) = dec.stream_info() {
                        *res.borrow_mut() = info.coded_resolution;
                        println!(
                            "format: {:?} coded {}x{} display {}x{}",
                            info.format,
                            info.coded_resolution.width,
                            info.coded_resolution.height,
                            info.display_resolution.width,
                            info.display_resolution.height
                        );
                    }
                }
            }
        }
    };

    let mut ts: u64 = 0;
    for nal in NalIterator::<H264Nalu>::new(&input) {
        let bitstream = nal.as_ref();
        let mut off = 0usize;
        let mut stalls = 0u32;
        while off < bitstream.len() {
            let res_cb = Rc::clone(&res);
            let mut alloc_cb = move || {
                let r = *res_cb.borrow();
                (r.width > 0).then(|| VaSurfaceFrame { resolution: r })
            };
            match decoder.decode(ts, &bitstream[off..], &mut alloc_cb) {
                Ok(0) => {
                    // No progress on this input; drain and give up on this NAL.
                    drain(&mut decoder, &mut frames, &mut nonblack, &mut dims, &res);
                    break;
                }
                Ok(n) => {
                    off += n;
                    stalls = 0;
                    drain(&mut decoder, &mut frames, &mut nonblack, &mut dims, &res);
                }
                // Back-pressure: the decoder needs pending events consumed (or
                // output frames returned) before it can accept more input. Drain
                // and RETRY the same bytes — this is not an error.
                Err(DecodeError::CheckEvents) | Err(DecodeError::NotEnoughOutputBuffers(_)) => {
                    drain(&mut decoder, &mut frames, &mut nonblack, &mut dims, &res);
                    stalls += 1;
                    if stalls > 64 {
                        eprintln!("decode stalled on back-pressure, skipping NAL");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("decode error: {e:?}");
                    break;
                }
            }
        }
        ts += 1;
    }
    decoder.flush().ok();
    drain(&mut decoder, &mut frames, &mut nonblack, &mut dims, &res);

    println!("\ndecoded {frames} frames, {}x{}, non-black luma: {nonblack}", dims.0, dims.1);
    if frames >= min_frames && nonblack {
        println!("PASS — VA-API HW H.264 decode works (tier 3 readback, VA-allocated surfaces)");
    } else {
        eprintln!("FAIL — expected >= {min_frames} non-black frames, got {frames}");
        std::process::exit(1);
    }
}
