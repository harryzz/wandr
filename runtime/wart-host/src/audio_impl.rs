//! AAudio playback via rsbinder + the A2/A3 primitives.
//!
//! Architecture (see tasks/21 appendix for full protocol):
//!   1. Look up `media.aaudio` (`IAAudioService`) lazily on first use.
//!   2. `create_track`: openStream → getStreamDescription → mmap every
//!      `SharedFileRegion` via `BinderMappedMemory` → resolve the three
//!      `SharedRegion`s of `downDataQueueParcelable` (readCounter,
//!      writeCounter, data) into raw pointers within the mmaps.
//!   3. `write_pcm_f32`: SPSC ring discipline with release-acquire
//!      ordering on the int64 counter pair. The counters live in
//!      shared memory between us and the AAudio service's HAL thread;
//!      they're our only signaling channel (AAudio data plane has no
//!      eventfd — confirmed in B1).
//!   4. `start` / `pause` / `close` map straight to binder calls.
//!
//! `registerClient(IAAudioClient)` is deliberately skipped: the AIDL
//! doesn't permit a null arg, but the service typically tolerates
//! missing client registration when the caller only writes data and
//! ignores stream-change events. If B5 device verify shows openStream
//! failing, we'll fall back to the BnAAudioClient stub pattern from
//! task 20.

use crate::bindings::my::skiko_gfx::audio::{
    ChannelLayout, Host, TrackConfig, TrackHandle,
};

#[cfg(target_os = "android")]
mod binder_path {
    use crate::binder_aidl::aaudio::{
        Endpoint::Endpoint,
        IAAudioClient::{BnAAudioClient, IAAudioClient, IAAudioClientAsyncService},
        IAAudioService::IAAudioService,
        SharedRegion::SharedRegion,
        StreamParameters::StreamParameters,
        StreamRequest::StreamRequest,
    };
    use crate::binder_aidl::android::media::audio::common::{
        AudioFormatDescription::AudioFormatDescription,
        AudioFormatType::AudioFormatType,
        PcmType::PcmType,
    };
    use crate::binder_aidl::android::media::SharedFileRegion::SharedFileRegion;
    use crate::binder_shared_memory::BinderMappedMemory;
    use std::collections::HashMap;
    use std::os::fd::OwnedFd;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Mutex, OnceLock};

    // AAudio C constants — values from AAudio.h (NDK headers). We hard-code
    // because we don't link libaaudio; the values are stable AOSP contracts.
    //
    // Got bitten once: SHARING_MODE_EXCLUSIVE=0, SHARING_MODE_SHARED=1 (not
    // alphabetical — exclusive came first historically). Setting EXCLUSIVE
    // by accident forces the MMAP/low-latency path and skips the
    // AudioFlinger fallback, so an unsupported format/channel pair fails
    // outright with -889 (UNAVAILABLE) instead of silently converting.
    const AAUDIO_DIRECTION_OUTPUT:    i32 = 0;
    const AAUDIO_DIRECTION_INPUT:     i32 = 1; // capture (mic)
    // Input source preset — VOICE_RECOGNITION = raw-ish mic, no AGC/NS, low latency.
    const AAUDIO_INPUT_PRESET_VOICE_RECOGNITION: i32 = 6;
    const AAUDIO_SHARING_MODE_SHARED: i32 = 1;
    const AAUDIO_USAGE_MEDIA:         i32 = 1;
    const AAUDIO_CONTENT_TYPE_MUSIC:  i32 = 2;
    // AAUDIO_CHANNEL_MONO   = FRONT_LEFT          = bit 0  (0x1)
    // AAUDIO_CHANNEL_STEREO = FRONT_LEFT|RIGHT    = bits 0+1 (0x3)
    const AAUDIO_CHANNEL_MONO:        i32 = 0x1;
    const AAUDIO_CHANNEL_STEREO:      i32 = 0x3;

    fn service() -> Option<&'static rsbinder::Strong<dyn IAAudioService>> {
        static SVC: OnceLock<Option<rsbinder::Strong<dyn IAAudioService>>> = OnceLock::new();
        SVC.get_or_init(|| {
            let svc = match rsbinder::hub::get_interface::<dyn IAAudioService>("media.aaudio") {
                Ok(s)  => { log::info!("audio: media.aaudio ready"); s }
                Err(e) => { log::warn!("audio: media.aaudio unavailable: {e:?}"); return None }
            };
            // Register a minimal IAAudioClient so the service has a callback
            // sink. Without this the SHARED-mode AudioFlinger fallback was
            // never tried on the Pixel 2 XL — the service's MMAP attempt
            // failed and it bailed instead of falling back, possibly because
            // a stream-change event had no client to deliver to. Reuses the
            // tokio current-thread runtime pattern from sensors_impl.rs.
            let cb: rsbinder::Strong<dyn IAAudioClient> =
                BnAAudioClient::new_async_binder(AAudioClientStub, TokioRuntime);
            match svc.r#registerClient(&cb) {
                Ok(())  => log::info!("audio: registerClient ok"),
                Err(e)  => log::warn!("audio: registerClient failed: {e:?}"),
            }
            // Hold the callback alive for the process lifetime so the
            // service's weak ref doesn't drop and trigger re-registration
            // demands on every openStream.
            let _ = AAUDIO_CLIENT.set(cb);
            Some(svc)
        }).as_ref()
    }

    /// Bn-side `IAAudioClient` stub. The service uses this to deliver
    /// stream-change events (STARTED/PAUSED/XRUN/DISCONNECTED). v1 ignores
    /// them — guest code poll-drives writes — so onStreamChange is a no-op
    /// with a debug log.
    struct AAudioClientStub;
    impl rsbinder::Interface for AAudioClientStub {}
    #[async_trait::async_trait]
    impl IAAudioClientAsyncService for AAudioClientStub {
        async fn r#onStreamChange(&self, handle: i32, opcode: i32, value: i32)
            -> rsbinder::status::Result<()>
        {
            log::debug!("audio: onStreamChange handle={handle} opcode={opcode} value={value}");
            Ok(())
        }
    }
    static AAUDIO_CLIENT: OnceLock<rsbinder::Strong<dyn IAAudioClient>> = OnceLock::new();

    /// tokio current-thread runtime for the Bn server. Same pattern as
    /// sensors_impl.rs (see notes there about why a real runtime is
    /// required even when callbacks never actually await).
    struct TokioRuntime;
    impl rsbinder::BinderAsyncRuntime for TokioRuntime {
        fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
            static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let rt = RT.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio current-thread runtime")
            });
            rt.block_on(f)
        }
    }

    /// State of one open track. Holds the mmaps to keep them alive; the
    /// raw counter / data pointers point into those mmaps and stay valid
    /// until close() drops them.
    struct TrackState {
        stream_handle:   i32,
        _mmaps:          Vec<BinderMappedMemory>,
        read_ctr_ptr:    *const AtomicI64,
        write_ctr_ptr:   *mut AtomicI64,
        data_ptr:        *mut u8,
        capacity_frames: u32,
        bytes_per_frame: u32,
        channels:        u32,
    }
    // SAFETY: raw pointers reference mmaps owned by this struct (stable
    // for the lifetime of `_mmaps`). Cross-process atomic ops on the
    // counters are the protocol contract (COHERENCY_ACQUIRE_RELEASE).
    // wasmtime's store is single-threaded, so no local concurrent access.
    unsafe impl Send for TrackState {}

    /// `(next_id, handles)`. Counter starts at 1 — sentinel 0 = invalid.
    type TrackMap = Mutex<(u32, HashMap<u32, TrackState>)>;
    fn track_map() -> &'static TrackMap {
        static MAP: OnceLock<TrackMap> = OnceLock::new();
        MAP.get_or_init(|| Mutex::new((1, HashMap::new())))
    }
    fn alloc_handle(state: TrackState) -> u32 {
        let mut m = track_map().lock().unwrap();
        let id = m.0;
        m.0 = m.0.wrapping_add(1).max(1);
        m.1.insert(id, state);
        id
    }
    fn with_track<F, R>(handle: u32, f: F) -> Option<R>
    where F: FnOnce(&TrackState) -> R
    {
        let m = track_map().lock().ok()?;
        let st = m.1.get(&handle)?;
        Some(f(st))
    }

    /// De-risk probe (mic input): open an AAUDIO_DIRECTION_INPUT stream and log
    /// whether the (root/su) caller is granted capture — the open question before
    /// building the full read path. Opens, reports the endpoint, and closes.
    /// Invoked via `wart-host --probe-audio-capture`.
    pub fn probe_capture() {
        // The probe runs standalone (no render loop), so init the binder
        // ProcessState ourselves (the standalone path does this on startup).
        if let Err(e) = crate::binder::init() {
            log::warn!("probe-capture: binder init failed: {e}");
            return;
        }
        let Some(svc) = service() else {
            log::warn!("probe-capture: media.aaudio unavailable");
            return;
        };
        let params = StreamParameters {
            r#channelMask:  AAUDIO_CHANNEL_MONO,
            r#sampleRate:   48000,
            r#sharingMode:  AAUDIO_SHARING_MODE_SHARED,
            r#audioFormat:  AudioFormatDescription {
                r#type:     AudioFormatType::PCM,
                r#pcm:      PcmType::FLOAT_32_BIT,
                r#encoding: String::new(),
            },
            r#direction:    AAUDIO_DIRECTION_INPUT,
            r#inputPreset:  AAUDIO_INPUT_PRESET_VOICE_RECOGNITION,
            ..Default::default()
        };
        let req = StreamRequest {
            r#params: params,
            r#attributionSource: Default::default(),
            r#sharingModeMatchRequired: false,
            r#inService: false,
        };
        let mut params_out = StreamParameters::default();
        match svc.r#openStream(&req, &mut params_out) {
            Ok(h) if h > 0 => {
                log::info!(
                    "probe-capture: openStream(INPUT) OK — handle={h} rate={} cap_frames={} \
                     — MIC CAPTURE GRANTED to this caller",
                    params_out.r#sampleRate, params_out.r#bufferCapacity,
                );
                let mut ep = Endpoint::default();
                match svc.r#getStreamDescription(h, &mut ep) {
                    Ok(0) => log::info!(
                        "probe-capture: endpoint OK — {} shared region(s) (upDataQueue carries capture PCM)",
                        ep.r#sharedMemories.len(),
                    ),
                    other => log::warn!("probe-capture: getStreamDescription failed: {other:?}"),
                }
                let _ = svc.r#closeStream(h);
            }
            Ok(neg) => log::warn!(
                "probe-capture: openStream(INPUT) returned {neg} — likely permission/policy denial"
            ),
            Err(e) => log::warn!(
                "probe-capture: openStream(INPUT) binder error: {e:?} — SELinux AVC / RECORD_AUDIO?"
            ),
        }
    }

    pub fn create_track(cfg: super::TrackConfig) -> u32 {
        let Some(svc) = service() else { return 0 };

        let (channels, channel_mask): (u32, i32) = match cfg.channel_layout {
            super::ChannelLayout::Mono   => (1, AAUDIO_CHANNEL_MONO),
            super::ChannelLayout::Stereo => (2, AAUDIO_CHANNEL_STEREO),
        };

        let params = StreamParameters {
            r#channelMask:         channel_mask,
            r#sampleRate:          cfg.sample_rate as i32,
            r#sharingMode:         AAUDIO_SHARING_MODE_SHARED,
            r#audioFormat:         AudioFormatDescription {
                r#type:     AudioFormatType::PCM,
                r#pcm:      PcmType::FLOAT_32_BIT,
                r#encoding: String::new(),
            },
            r#direction:           AAUDIO_DIRECTION_OUTPUT,
            r#usage:               AAUDIO_USAGE_MEDIA,
            r#contentType:         AAUDIO_CONTENT_TYPE_MUSIC,
            ..Default::default()
        };
        // AttributionSourceState left as the empty-stub default — see the
        // .aidl file for why a full-shape version isn't currently emittable
        // by rsbinder-aidl 0.7.0. The service auto-fills pid/uid from the
        // binder caller context, which is enough to reach the SHARED-mode
        // dispatch path.
        let req = StreamRequest {
            r#params: params,
            r#attributionSource: Default::default(),
            r#sharingModeMatchRequired: false,
            r#inService: false,
        };

        let mut params_out = StreamParameters::default();
        let stream_handle = match svc.r#openStream(&req, &mut params_out) {
            Ok(h) if h > 0 => h,
            Ok(neg)        => { log::warn!("audio: openStream returned {neg}"); return 0; }
            Err(e)         => { log::warn!("audio: openStream binder error: {e:?}"); return 0; }
        };
        log::info!(
            "audio: openStream ok — handle={} sample_rate={} channels={} bufferCapacity={}",
            stream_handle, params_out.r#sampleRate, channels, params_out.r#bufferCapacity,
        );

        let mut endpoint = Endpoint::default();
        match svc.r#getStreamDescription(stream_handle, &mut endpoint) {
            Ok(0)  => {}
            other  => {
                log::warn!("audio: getStreamDescription failed: {other:?}");
                let _ = svc.r#closeStream(stream_handle);
                return 0;
            }
        }

        // mmap every SharedFileRegion the service handed us.
        let mut mmaps: Vec<BinderMappedMemory> =
            Vec::with_capacity(endpoint.r#sharedMemories.len());
        for (i, sfr) in endpoint.r#sharedMemories.into_iter().enumerate() {
            let SharedFileRegion {
                r#fd: Some(pfd), r#offset, r#size, r#writeable,
            } = sfr else {
                log::warn!("audio: SharedFileRegion[{i}] null fd; aborting");
                let _ = svc.r#closeStream(stream_handle);
                return 0;
            };
            let owned_fd: OwnedFd = pfd.into();
            // Force a writeable mapping regardless of the `writeable` flag
            // the service hands us. AAudio always marks `writeable=false`
            // on its SharedFileRegions but the producer (us) MUST write
            // both into the data ring AND into the writeCounter. AOSP's
            // libaaudio does the same thing in `SharedMemoryParcelable
            // ::resolveSharedMemory()` — it hard-codes
            // `PROT_READ | PROT_WRITE | MAP_SHARED` and ignores the flag.
            let _ = writeable;
            match BinderMappedMemory::map(owned_fd, offset, size, /*writeable=*/true) {
                Ok(m) => {
                    log::debug!(
                        "audio: mmap shm[{i}] off={} size={} service_writeable={} (mapped RW)",
                        offset, size, writeable,
                    );
                    mmaps.push(m);
                }
                Err(e) => {
                    log::warn!("audio: mmap shm[{i}] failed: {e}");
                    let _ = svc.r#closeStream(stream_handle);
                    return 0;
                }
            }
        }

        let rb = endpoint.r#downDataQueueParcelable;
        let bytes_per_frame = rb.r#bytesPerFrame as u32;
        let capacity_frames = rb.r#capacityInFrames as u32;
        if bytes_per_frame == 0 || capacity_frames == 0 {
            log::warn!(
                "audio: empty downDataQueue (bpf={bytes_per_frame}, cap={capacity_frames})"
            );
            let _ = svc.r#closeStream(stream_handle);
            return 0;
        }

        // Resolve a SharedRegion (sharedMemoryIndex + offset + size) into
        // a raw pointer into the matching mmap. Used for the 3 regions
        // of the down-data RingBuffer.
        let mut resolve = |reg: &SharedRegion| -> Option<*mut u8> {
            let idx = reg.r#sharedMemoryIndex;
            if idx < 0 { return None; }
            let m = mmaps.get_mut(idx as usize)?;
            let off = reg.r#offsetInBytes as usize;
            // Cast through *const if read-only — the AAudio service may mark
            // a counter region read-only at the SharedFileRegion level, but
            // we still need a *mut for AtomicI64::store. The kernel mmap
            // permissions are the real enforcement; this cast is just type
            // gymnastics. (writeCounter is always our side to write.)
            let base = if m.is_writeable() {
                m.as_mut_slice()?.as_mut_ptr()
            } else {
                m.as_slice().as_ptr() as *mut u8
            };
            // SAFETY: off + reg.sizeInBytes <= mmap length, enforced by the
            // service when it sized SharedFileRegion. We trust the contract.
            Some(unsafe { base.add(off) })
        };
        let read_ctr_p  = resolve(&rb.r#readCounterParcelable);
        let write_ctr_p = resolve(&rb.r#writeCounterParcelable);
        let data_p      = resolve(&rb.r#dataParcelable);
        let (Some(read_ctr_p), Some(write_ctr_p), Some(data_p)) =
            (read_ctr_p, write_ctr_p, data_p)
        else {
            log::warn!("audio: downDataQueue SharedRegion resolution failed");
            let _ = svc.r#closeStream(stream_handle);
            return 0;
        };

        // AtomicI64 has the same layout as i64 (`#[repr(C, align(8))]`).
        // The counter slots in AAudio's shared memory are 8-byte aligned
        // int64s by the protocol; the cast is sound.
        let state = TrackState {
            stream_handle,
            _mmaps:          mmaps,
            read_ctr_ptr:    read_ctr_p  as *const AtomicI64,
            write_ctr_ptr:   write_ctr_p as *mut   AtomicI64,
            data_ptr:        data_p,
            capacity_frames,
            bytes_per_frame,
            channels,
        };
        let id = alloc_handle(state);
        log::info!(
            "audio: track {id} ready — stream_handle={stream_handle} \
             cap_frames={capacity_frames} bpf={bytes_per_frame}",
        );
        id
    }

    pub fn write_pcm_f32(track: u32, samples: &[f32]) -> u32 {
        with_track(track, |st| {
            // SAFETY: ptrs reference an 8-byte aligned i64 slot inside an
            // mmap shared with media.aaudio. Cross-process atomic ops
            // on the counter pair are AAudio's signaling contract.
            let read_ctr  = unsafe { &*st.read_ctr_ptr };
            let write_ctr = unsafe { &*st.write_ctr_ptr };

            let r = read_ctr.load(Ordering::Acquire) as u64;
            let w = write_ctr.load(Ordering::Relaxed) as u64;
            let in_flight = w.wrapping_sub(r);
            let free_frames = (st.capacity_frames as u64)
                .saturating_sub(in_flight) as u32;

            let frames_in_buf = (samples.len() as u32) / st.channels;
            let to_write = frames_in_buf.min(free_frames);
            if to_write == 0 { return 0u32; }

            let bpf       = st.bytes_per_frame as u64;
            let cap_bytes = st.capacity_frames as u64 * bpf;
            // wrap-aware byte offset into the data ring
            let base_off  = (w * bpf) % cap_bytes;
            let src_bytes = (to_write as u64 * bpf) as usize;
            let first     = src_bytes.min((cap_bytes - base_off) as usize);

            // SAFETY: bounds checked above — base_off + first <= cap_bytes;
            // (src_bytes - first) <= base_off; samples has at least
            // to_write*channels f32s, so src_bytes <= samples.len()*4.
            unsafe {
                let src = samples.as_ptr() as *const u8;
                std::ptr::copy_nonoverlapping(
                    src,
                    st.data_ptr.add(base_off as usize),
                    first,
                );
                if first < src_bytes {
                    std::ptr::copy_nonoverlapping(
                        src.add(first),
                        st.data_ptr,
                        src_bytes - first,
                    );
                }
            }

            // Publish — Release pairs with the service's Acquire load.
            write_ctr.store(
                w.wrapping_add(to_write as u64) as i64,
                Ordering::Release,
            );
            to_write
        }).unwrap_or(0)
    }

    pub fn start(track: u32) -> bool {
        with_track(track, |st| match service() {
            Some(svc) => match svc.r#startStream(st.stream_handle) {
                Ok(0)  => true,
                other  => { log::warn!("audio: startStream {other:?}"); false }
            },
            None => false,
        }).unwrap_or(false)
    }

    pub fn pause(track: u32) -> bool {
        with_track(track, |st| match service() {
            Some(svc) => match svc.r#pauseStream(st.stream_handle) {
                Ok(0)  => true,
                other  => { log::warn!("audio: pauseStream {other:?}"); false }
            },
            None => false,
        }).unwrap_or(false)
    }

    pub fn close(track: u32) {
        let removed = {
            let mut m = track_map().lock().unwrap();
            m.1.remove(&track)
        };
        if let Some(st) = removed {
            if let Some(svc) = service() {
                let _ = svc.r#closeStream(st.stream_handle);
            }
            drop(st);
        }
    }

    pub fn pending_frames(track: u32) -> u32 {
        with_track(track, |st| {
            let read_ctr  = unsafe { &*st.read_ctr_ptr };
            let write_ctr = unsafe { &*st.write_ctr_ptr };
            let r = read_ctr.load(Ordering::Acquire) as u64;
            let w = write_ctr.load(Ordering::Relaxed) as u64;
            w.wrapping_sub(r).min(u32::MAX as u64) as u32
        }).unwrap_or(0)
    }
}

impl Host for crate::HostState {
    fn create_track(&mut self, cfg: TrackConfig) -> TrackHandle {
        #[cfg(target_os = "android")]
        { return binder_path::create_track(cfg); }
        #[cfg(not(target_os = "android"))]
        { let _ = cfg; 0 }
    }

    fn write_pcm_f32(&mut self, track: TrackHandle, samples: Vec<f32>) -> u32 {
        #[cfg(target_os = "android")]
        { return binder_path::write_pcm_f32(track, &samples); }
        #[cfg(not(target_os = "android"))]
        { let _ = (track, samples); 0 }
    }

    fn start(&mut self, track: TrackHandle) -> bool {
        #[cfg(target_os = "android")]
        { return binder_path::start(track); }
        #[cfg(not(target_os = "android"))]
        { let _ = track; false }
    }

    fn pause(&mut self, track: TrackHandle) -> bool {
        #[cfg(target_os = "android")]
        { return binder_path::pause(track); }
        #[cfg(not(target_os = "android"))]
        { let _ = track; false }
    }

    fn close(&mut self, track: TrackHandle) {
        #[cfg(target_os = "android")]
        { binder_path::close(track); }
        #[cfg(not(target_os = "android"))]
        { let _ = track; }
    }

    fn pending_frames(&mut self, track: TrackHandle) -> u32 {
        #[cfg(target_os = "android")]
        { return binder_path::pending_frames(track); }
        #[cfg(not(target_os = "android"))]
        { let _ = track; 0 }
    }
}

/// Mic-capture de-risk entry (`wart-host --probe-audio-capture`): does
/// openStream(INPUT) succeed for our (root/su) caller? See `binder_path::probe_capture`.
#[cfg(target_os = "android")]
pub fn probe_capture() {
    binder_path::probe_capture();
}
#[cfg(not(target_os = "android"))]
pub fn probe_capture() {
    log::warn!("probe-capture: android-only build");
}

// Silence "unused" warnings when targeting desktop where binder_path is gone.
#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn _unused(c: ChannelLayout, f: crate::bindings::my::skiko_gfx::audio::Format) {
    let _ = (c, f);
}
