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
    ChannelLayout, Format, Host, StreamClass, TrackConfig, TrackHandle,
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
    // Kept for the Phase-B `usage` mapping (media tracks); unused while the
    // Phase-A spike hard-codes voice-comms on every track.
    #[allow(dead_code)]
    const AAUDIO_USAGE_MEDIA:         i32 = 1;
    #[allow(dead_code)]
    const AAUDIO_CONTENT_TYPE_MUSIC:  i32 = 2;
    // Comms-call playback. The AudioPolicyManager ducks/parks USAGE_MEDIA
    // streams to ~1% (volume=0.01) while the device is in IN_COMMUNICATION
    // mode (our calls set that via the arbiter), and the comms endpoint mixer
    // won't pull a media stream — so call audio MUST be tagged voice-comms or
    // the shared mixer's readCounter never advances. (Phase-A hard-code; Phase
    // B plumbs `usage` through the audio WIT track-config.)
    // Kept for reference / the matrix probe; the live path now sources usage +
    // content from the routing core's StreamPlan (task 76).
    #[allow(dead_code)]
    const AAUDIO_USAGE_VOICE_COMMUNICATION: i32 = 2;
    #[allow(dead_code)]
    const AAUDIO_CONTENT_TYPE_SPEECH:       i32 = 1;
    // AAUDIO_CHANNEL_MONO   = FRONT_LEFT          = bit 0  (0x1)
    // AAUDIO_CHANNEL_STEREO = FRONT_LEFT|RIGHT    = bits 0+1 (0x3)
    const AAUDIO_CHANNEL_MONO:        i32 = 0x1;
    const AAUDIO_CHANNEL_STEREO:      i32 = 0x3;

    /// Resolve `media.aaudio`, re-resolving if the cached handle is dead.
    ///
    /// `media.aaudio` is a LAZY service: it (re)registers a brand-new binder
    /// whenever audioserver (re)starts. A handle cached across an audioserver
    /// restart is stale → openStream fails with `DeadObject`. So we `ping_binder`
    /// the cached handle and, if it's dead, drop it and look the service up
    /// again (and re-`registerClient`). Returns an owned `Strong` clone (cheap,
    /// ref-counted) rather than a `&'static` so the cache can be swapped.
    fn service() -> Option<rsbinder::Strong<dyn IAAudioService>> {
        static SVC: OnceLock<Mutex<Option<rsbinder::Strong<dyn IAAudioService>>>> = OnceLock::new();
        let mut guard = SVC.get_or_init(|| Mutex::new(None)).lock().unwrap();

        if let Some(svc) = guard.as_ref() {
            if svc.as_binder().ping_binder().is_ok() {
                return Some(svc.clone());
            }
            log::warn!("audio: media.aaudio handle dead (audioserver restarted?) — re-resolving");
            *guard = None;
        }

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
        // Hold the latest callback alive so the service's weak ref doesn't drop
        // and trigger re-registration demands on every openStream. Re-resolve
        // replaces it (can't use a write-once OnceLock here).
        *aaudio_client_slot().lock().unwrap() = Some(cb);
        *guard = Some(svc.clone());
        Some(svc)
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
    /// Latest registered `IAAudioClient`, held alive for the process. A
    /// `Mutex<Option<>>` (not `OnceLock`) so re-resolve can replace it.
    fn aaudio_client_slot() -> &'static Mutex<Option<rsbinder::Strong<dyn IAAudioClient>>> {
        static C: OnceLock<Mutex<Option<rsbinder::Strong<dyn IAAudioClient>>>> = OnceLock::new();
        C.get_or_init(|| Mutex::new(None))
    }

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
        // `*mut` (not `*const`) because capture reverses the roles: for an
        // output stream WE write writeCounter + read readCounter; for a
        // capture stream WE write readCounter (consume) + read writeCounter.
        read_ctr_ptr:    *mut AtomicI64,
        write_ctr_ptr:   *mut AtomicI64,
        data_ptr:        *mut u8,
        capacity_frames: u32,
        bytes_per_frame: u32,
        channels:        u32,
        // The service→client up-message queue counters (control/event/timestamp
        // messages). We poll-drive and ignore the events, but MUST drain this or
        // the service's writeUpMessageQueue fills, it decides the client stopped,
        // and it suspends + closes the stream. `read` = ours, `write` = service's.
        up_msg_read_ptr:  Option<*mut AtomicI64>,
        up_msg_write_ptr: Option<*mut AtomicI64>,
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

    /// Mic→speaker loopback (`--probe-audio-loopback`): open a capture and an
    /// output stream and pump captured frames straight to the speaker for
    /// ~8 s. You hear yourself — end-to-end proof of create_capture +
    /// read_pcm_f32 against the real HAL (the full path a guest would drive).
    pub fn probe_loopback() {
        if let Err(e) = crate::binder::init() {
            log::warn!("probe-loopback: binder init failed: {e}");
            return;
        }
        let cfg = || super::TrackConfig {
            sample_rate:    48_000,
            channel_layout: super::ChannelLayout::Mono,
            format:         super::Format::PcmF32,
            class:          super::StreamClass::VoiceCall, // loopback ≈ a call
        };
        let cap = create_capture(cfg());
        if cap == 0 {
            log::warn!("probe-loopback: capture open failed");
            return;
        }
        // Output is best-effort: on taimen, holding an input MMAP endpoint can
        // block a second (output) one → -889. If it fails we still verify the
        // capture data path (RMS/peak) — just without audible playback.
        let out = create_track(cfg());
        if out == 0 {
            log::warn!("probe-loopback: output open failed (-889?) — capture-only mode (no playback)");
        }
        if !start(cap) || (out != 0 && !start(out)) {
            log::warn!("probe-loopback: startStream failed — aborting");
            close(cap); if out != 0 { close(out); }
            return;
        }
        log::info!(
            "probe-loopback: running ~8 s ({}) — speak into the mic…",
            if out != 0 { "mic→speaker loopback" } else { "capture-only" },
        );
        let t0 = std::time::Instant::now();
        let (mut total_frames, mut peak, mut sumsq) = (0u64, 0.0f32, 0.0f64);
        while t0.elapsed().as_secs() < 8 {
            let frames = read_pcm_f32(cap, 480); // ~10 ms @ 48k mono
            if !frames.is_empty() {
                for &s in &frames { peak = peak.max(s.abs()); sumsq += (s as f64) * (s as f64); }
                total_frames += frames.len() as u64; // mono: 1 sample = 1 frame
                if out != 0 {
                    // write may accept fewer than offered; retry the leftover.
                    let mut off = 0usize;
                    while off < frames.len() {
                        let wrote = write_pcm_f32(out, &frames[off..]) as usize;
                        if wrote == 0 { break; }
                        off += wrote;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let rms = if total_frames > 0 { (sumsq / total_frames as f64).sqrt() } else { 0.0 };
        log::info!(
            "probe-loopback: done — {total_frames} frames captured, peak={peak:.4}, rms={rms:.4} \
             (≈0 = silence/denied mic, >0 = live audio)"
        );
        close(cap);
        if out != 0 { close(out); }
    }

    /// Play a sine tone to the speaker for `ms` at `hz` — the host-side applier for
    /// the arbiter's `play-tone` push (and a CLI/warm-up way to make a sound). OUTPUT
    /// ONLY (no capture → avoids the taimen in+out MMAP -889 conflict). Mirrors the
    /// create_track → start → write_pcm_f32 → close flow the guest drives. Blocking;
    /// callers (the control socket) run it off-thread.
    pub fn play_tone(ms: u32, hz: f32, vol: f32) {
        if let Err(e) = crate::binder::init() {
            log::warn!("play-tone: binder init failed: {e}");
            return;
        }
        // Output on taimen must be F32 STEREO — mono → openStream -889
        // (AAUDIO_ERROR_UNAVAILABLE). See [[project_call_audio_output]].
        let cfg = super::TrackConfig {
            sample_rate:    48_000,
            channel_layout: super::ChannelLayout::Stereo,
            format:         super::Format::PcmF32,
            class:          super::StreamClass::Media,
        };
        let out = create_track(cfg);
        if out == 0 {
            log::warn!("play-tone: output open failed");
            return;
        }
        if !start(out) {
            log::warn!("play-tone: startStream failed");
            close(out);
            return;
        }
        let sr = 48_000.0_f32;
        let total_frames: usize = (sr * (ms as f32) / 1000.0) as usize;
        // Amplitude = caller's volume, clamped to [0,1]. This is the tone's
        // digital level (a relative gain in the PCM), NOT the device master
        // volume — absolute volume under --no-art is a separate problem.
        let amp = vol.clamp(0.0, 1.0);
        let chunk_frames = 480_usize; // ~10 ms @ 48k
        log::info!("play-tone: {hz} Hz for {ms} ms vol={amp:.2} stereo (handle={out})");
        let mut n = 0_usize; // frame index
        while n < total_frames {
            let end = (n + chunk_frames).min(total_frames);
            // Interleaved stereo: L,R per frame (same sine in both channels).
            let mut buf: Vec<f32> = Vec::with_capacity((end - n) * 2);
            for i in n..end {
                let t = (i as f32) / sr;
                let s = (2.0 * std::f32::consts::PI * hz * t).sin() * amp;
                buf.push(s); // L
                buf.push(s); // R
            }
            let mut off = 0_usize; // sample offset into the interleaved buf
            let mut spins = 0;
            while off < buf.len() {
                let wrote_frames = write_pcm_f32(out, &buf[off..]) as usize;
                if wrote_frames == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(3));
                    spins += 1;
                    if spins > 300 { break; } // ~0.9 s stuck → bail (don't hang forever)
                    continue;
                }
                off += wrote_frames * 2; // stereo: 2 samples per frame
                spins = 0;
            }
            n = end;
        }
        std::thread::sleep(std::time::Duration::from_millis(120)); // let the ring drain
        close(out);
        log::info!("play-tone: done");
    }

    /// Task 97 bug #1 repro — reproduce the SHARED-output **suspend stall** on the
    /// call path, and confirm the source-grounded mechanism (see
    /// `[[project_call_audioserver_crash]]` / `tasks/97`).
    ///
    /// Source chain (vendored `frameworks/av/services/oboeservice`): a SHARED
    /// output stream that **underflows** gets an `XRUN` service event written into
    /// its up-message FIFO *every mixer burst*
    /// (`AAudioServiceEndpointPlay::callbackLoop` → `incrementXRunCount` →
    /// `sendXRunCount` → `writeUpMessageQueue`, with **no** `isUpMessageQueueBusy`
    /// throttle — unlike timestamps). The FIFO is **128** deep
    /// (`QUEUE_UP_CAPACITY_COMMANDS`). If the client stops draining for ~0.5 s of
    /// continuous underflow it overflows → `writeUpMessageQueue` logs *"Queue full.
    /// Did client stop? Suspending stream"* → `setSuspended(true)` → the mixer then
    /// SKIPS the stream (`if (clientStream->isSuspended()) continue; // dead
    /// stream`) → the client FIFO read counter **`r` freezes** → `in_flight` pegs at
    /// capacity → `write_pcm_f32` returns 0 → silence. `play_tone` never trips this
    /// because it writes as fast as the ring frees (never underflows).
    ///
    /// This probe forces the condition: prime the stream (so it is `isFlowing`),
    /// then **stop feeding** for an underflow window (the mixer floods XRUN), then
    /// resume. It logs the ring + the up-message cursors each tick so we can see
    /// exactly when `r` freezes, whether the up-message queue was resolved/drained,
    /// and whether the stream recovers. `drain_in_pause` keeps draining the
    /// up-queue during the underflow window — the A/B that tests whether continuous
    /// draining prevents/recovers the suspend (H1: an unresolved/!drained up-queue
    /// → permanent stall needing relaunch).
    ///
    /// `secs` = length of the post-pause resume window (watch `wr_ok` climb then
    /// freeze). `speaker` = comms route (false = earpiece pin, the call default).
    pub fn probe_call_stall(secs: u32, speaker: bool, drain_in_pause: bool) {
        if let Err(e) = crate::binder::init() {
            log::warn!("probe-call-stall: binder init failed: {e}");
            return;
        }
        use crate::audio_routing::Route;
        let cfg = super::TrackConfig {
            sample_rate:    48_000,
            channel_layout: super::ChannelLayout::Stereo,
            format:         super::Format::PcmF32,
            class:          super::StreamClass::VoiceCall,
        };
        let out = open_routed(cfg, Route::Call { speaker });
        if out == 0 {
            log::warn!("probe-call-stall: call output open failed (-889?) — aborting");
            return;
        }
        if !start(out) {
            log::warn!("probe-call-stall: startStream failed");
            close(out);
            return;
        }

        // Snapshot the ring + the *pre-drain* up-message cursors (drain hides the
        // delta by setting read:=write). Returns (r, w, cap, up:Option<(ur,uw)>).
        let snap = |t: u32| -> Option<(i64, i64, u32, Option<(i64, i64)>)> {
            with_track(t, |st| {
                let r = unsafe { &*st.read_ctr_ptr }.load(Ordering::Acquire);
                let w = unsafe { &*st.write_ctr_ptr }.load(Ordering::Relaxed);
                let up = match (st.up_msg_read_ptr, st.up_msg_write_ptr) {
                    (Some(rp), Some(wp)) => Some((
                        unsafe { &*rp }.load(Ordering::Acquire),
                        unsafe { &*wp }.load(Ordering::Acquire),
                    )),
                    _ => None,
                };
                (r, w, st.capacity_frames, up)
            })
        };
        let drain_only = |t: u32| { let _ = with_track(t, drain_up_messages); };
        let log_state = |tag: &str, t: u32| {
            if let Some((r, w, cap, up)) = snap(t) {
                let in_flight = (w as u64).wrapping_sub(r as u64) as i64;
                match up {
                    Some((ur, uw)) => log::info!(
                        "probe-call-stall[{tag}]: r={r} w={w} in_flight={in_flight}/{cap} \
                         up_resolved=YES up_fill={} (uw={uw} ur={ur})",
                        uw - ur,
                    ),
                    None => log::info!(
                        "probe-call-stall[{tag}]: r={r} w={w} in_flight={in_flight}/{cap} \
                         up_resolved=NO (drain is a no-op → H1)",
                    ),
                }
            }
        };

        // Quiet stereo tone generator (peak fidelity is irrelevant; ring dynamics
        // are). 480 frames = ~10 ms @ 48k, the real call cadence.
        let sr = 48_000.0_f32;
        let mut phase = 0.0_f32;
        let mut chunk = |frames: usize| -> Vec<f32> {
            let mut buf = Vec::with_capacity(frames * 2);
            for _ in 0..frames {
                let s = (phase).sin() * 0.2;
                phase += 2.0 * std::f32::consts::PI * 440.0 / sr;
                if phase > 2.0 * std::f32::consts::PI { phase -= 2.0 * std::f32::consts::PI; }
                buf.push(s); buf.push(s);
            }
            buf
        };
        let mut feed_paced = |t: u32, ms: u32| -> (u64, u64) {
            // Feed one 10 ms chunk per ~10 ms wall-clock, like a real call producer.
            let (mut wr_ok, mut wr_zero) = (0u64, 0u64);
            let t0 = std::time::Instant::now();
            while t0.elapsed().as_millis() < ms as u128 {
                let buf = chunk(480);
                let mut off = 0usize;
                let mut wrote_any = false;
                while off < buf.len() {
                    let w = write_pcm_f32(t, &buf[off..]) as usize;
                    if w == 0 { break; }
                    off += w * 2;
                    wrote_any = true;
                }
                if wrote_any && off > 0 { wr_ok += (off / 2) as u64; } else { wr_zero += 1; }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            (wr_ok, wr_zero)
        };

        log::info!(
            "probe-call-stall: handle={out} route=Call{{speaker={speaker}}} \
             drain_in_pause={drain_in_pause} — phases: prime 400ms / underflow 1500ms / resume {secs}s"
        );
        log_state("open", out);

        // Phase 1 — PRIME: feed for 400 ms so the mixer marks the stream isFlowing
        // (underflow XRUN is only counted once data has flowed).
        let (p1_ok, _) = feed_paced(out, 400);
        log_state("primed", out);
        log::info!("probe-call-stall[primed]: wr_ok={p1_ok}");

        // Phase 2 — UNDERFLOW: stop feeding for 1500 ms. The mixer drains the primed
        // audio, then underflows → floods XRUN into the 128-deep up-queue. Watch r:
        // it keeps advancing (mixer consuming) until the stream is SUSPENDED, then
        // freezes. `drain_in_pause` keeps the up-queue drained (the A/B).
        log::info!("probe-call-stall[underflow]: STOP feeding for 1500ms (drain_in_pause={drain_in_pause})");
        let mut last_r = snap(out).map(|s| s.0).unwrap_or(0);
        let mut froze_at_ms: Option<u128> = None;
        let pause_t0 = std::time::Instant::now();
        let mut tick = 0u32;
        while pause_t0.elapsed().as_millis() < 1500 {
            if drain_in_pause { drain_only(out); }
            std::thread::sleep(std::time::Duration::from_millis(10));
            tick += 1;
            if let Some((r, _, _, _)) = snap(out) {
                if r != last_r { last_r = r; }
                else if froze_at_ms.is_none() && pause_t0.elapsed().as_millis() > 200 {
                    // r unchanged across a tick after the primed buffer should be
                    // draining → first sign the mixer stopped consuming (suspend).
                    froze_at_ms = Some(pause_t0.elapsed().as_millis());
                    log::info!("probe-call-stall[underflow]: r FROZE at {} ms into pause (r={r})",
                        froze_at_ms.unwrap());
                    log_state("froze", out);
                }
            }
            if tick % 30 == 0 { log_state("underflow", out); }
        }
        match froze_at_ms {
            Some(ms) => log::info!("probe-call-stall[underflow]: r froze {ms} ms into the underflow window"),
            None     => log::info!("probe-call-stall[underflow]: r kept advancing the whole window (NO suspend)"),
        }
        log_state("post-pause", out);

        // Phase 3 — RESUME: feed again at the call cadence. If suspended, r stays
        // frozen → wr_ok climbs only until in_flight hits capacity, then write
        // returns 0 forever (= bug #1's "wr_ok≈284 then frozen"). If recovered, the
        // ring drains and wr_ok climbs continuously.
        log::info!("probe-call-stall[resume]: feeding {secs}s — watch wr_ok climb then (if stalled) freeze");
        let resume_t0 = std::time::Instant::now();
        let (mut total_ok, mut total_zero) = (0u64, 0u64);
        let mut last_log = std::time::Instant::now();
        while resume_t0.elapsed().as_secs() < secs as u64 {
            let (ok, zero) = feed_paced(out, 250);
            total_ok += ok; total_zero += zero;
            if last_log.elapsed().as_millis() >= 500 {
                log::info!("probe-call-stall[resume]: t={}ms wr_ok+={ok} wr_zero+={zero} (cum_ok={total_ok} cum_zero={total_zero})",
                    resume_t0.elapsed().as_millis());
                log_state("resume", out);
                last_log = std::time::Instant::now();
            }
        }
        let stalled = total_zero > 0 && total_ok == 0;
        log::info!(
            "probe-call-stall: DONE — froze_in_pause={} resume cum_ok={total_ok} cum_zero={total_zero} → {}",
            froze_at_ms.is_some(),
            if stalled { "STALLED (bug #1 reproduced — r frozen, writes rejected)" }
            else if total_zero > total_ok / 10 { "PARTIAL stall (intermittent rejects)" }
            else { "healthy (stream kept pulling)" },
        );
        log::info!("probe-call-stall: grep logcat for AAudio \"Suspending stream\" / \"Queue full\" to confirm the suspend.");
        std::thread::sleep(std::time::Duration::from_millis(120));
        close(out);
    }

    /// Task 76 P1 — CALL-ORDER full-duplex capture probe. Opens the OUTPUT first
    /// (USAGE_MEDIA → legacy SHARED, like a live call), keeps it active, THEN
    /// opens a CAPTURE with the given `inputPreset` and reads ~4 s. The mic-only
    /// loopback can't reproduce the call (output -889'd there); this matches the
    /// call exactly. MMAP capture spins on "wait for valid timestamps" while an
    /// output is active (Oboe #1842), delivering ~0 frames. Sweep presets to find
    /// one routed to the non-MMAP (legacy) input. frames≈0 ⇒ MMAP spin (bad);
    /// frames≫0 ⇒ legacy capture coexists (good).
    pub fn probe_duplex(preset: i32) {
        if let Err(e) = crate::binder::init() {
            log::warn!("probe-duplex: binder init failed: {e}"); return;
        }
        // Output first → legacy fallback, exactly like a call.
        let out = create_track(super::TrackConfig {
            sample_rate: 48_000, channel_layout: super::ChannelLayout::Stereo,
            format: super::Format::PcmF32, class: super::StreamClass::VoiceCall,
        });
        if out == 0 {
            log::warn!("probe-duplex[preset={preset}]: output open failed (-889) — aborting");
            return;
        }
        start(out);
        // Capture with the requested inputPreset.
        let cap_params = StreamParameters {
            r#channelMask: AAUDIO_CHANNEL_MONO,
            r#sampleRate:  48000,
            r#sharingMode: AAUDIO_SHARING_MODE_SHARED,
            r#audioFormat: pcm_f32_format(),
            r#direction:   AAUDIO_DIRECTION_INPUT,
            r#inputPreset: preset,
            ..Default::default()
        };
        let cap = open_pcm_stream(cap_params, 1, /*capture=*/ true);
        if cap == 0 {
            log::warn!("probe-duplex[preset={preset}]: capture open failed");
            close(out); return;
        }
        start(cap);
        log::info!("probe-duplex[preset={preset}]: output+capture open — reading ~4 s…");
        let t0 = std::time::Instant::now();
        let (mut total, mut peak, mut sumsq) = (0u64, 0.0f32, 0.0f64);
        while t0.elapsed().as_secs() < 4 {
            let frames = read_pcm_f32(cap, 480);
            if !frames.is_empty() {
                for &s in &frames { peak = peak.max(s.abs()); sumsq += (s as f64) * (s as f64); }
                total += frames.len() as u64;
                // keep the output ACTIVE (mic→speaker) — reproduces the call.
                let mut off = 0usize;
                while off < frames.len() {
                    let w = write_pcm_f32(out, &frames[off..]) as usize;
                    if w == 0 { break; }
                    off += w;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let rms = if total > 0 { (sumsq / total as f64).sqrt() } else { 0.0 };
        log::info!(
            "probe-duplex[preset={preset}]: {total} frames, peak={peak:.4} rms={rms:.4} — {}",
            if total > 100_000 { "LEGACY/coexists ✓" } else { "MMAP-spin/no-data ✗" },
        );
        close(cap); close(out);
    }

    fn channel_of(cfg: &super::TrackConfig) -> (u32, i32) {
        match cfg.channel_layout {
            super::ChannelLayout::Mono   => (1, AAUDIO_CHANNEL_MONO),
            super::ChannelLayout::Stereo => (2, AAUDIO_CHANNEL_STEREO),
        }
    }
    fn pcm_f32_format() -> AudioFormatDescription {
        AudioFormatDescription {
            r#type:     AudioFormatType::PCM,
            r#pcm:      PcmType::FLOAT_32_BIT,
            r#encoding: String::new(),
        }
    }

    /// Open an output stream for an intent [`Route`], resolving usage / device /
    /// format from the routing core ([[project_audio_routing_arbiter]]): the
    /// arbiter decides the `Route` (for calls), the host applies it here by
    /// pinning `StreamParameters.deviceIds` to the resolved port. Channels stay
    /// guest-provided (`cfg.channel_layout`) for now — host-owned channel
    /// authority + mono→stereo upmix is a later refinement.
    pub fn open_routed(cfg: super::TrackConfig, route: crate::audio_routing::Route) -> u32 {
        let plan = crate::audio_routing::DeviceModel::get().resolve_output(route);
        let (channels, channel_mask) = channel_of(&cfg);
        log::info!(
            "audio: create_track route={:?} -> [{}] usage={} content={} deviceIds={:?} (ch={channels})",
            route, plan.label, plan.usage, plan.content_type, plan.device_ids,
        );
        let params = StreamParameters {
            r#channelMask: channel_mask,
            r#sampleRate:  cfg.sample_rate as i32,
            r#sharingMode: plan.sharing,
            r#audioFormat: if plan.f32_format { pcm_f32_format() } else { pcm_i16_format() },
            r#direction:   AAUDIO_DIRECTION_OUTPUT,
            r#usage:       plan.usage,
            r#contentType: plan.content_type,
            r#deviceIds:   plan.device_ids,
            ..Default::default()
        };
        open_pcm_stream(params, channels, /*capture=*/ false)
    }

    /// WIT `create-track` path. The guest declares a [`StreamClass`] intent; the
    /// host maps it to a [`Route`](crate::audio_routing::Route). A `voice-call`
    /// follows the **arbiter's current comms route** (earpiece/speaker the
    /// arbiter last decided) — which is what makes the routing decision actually
    /// move the call's USAGE_MEDIA stream (a per-stream `deviceIds` pin;
    /// `setForceUse(COMMUNICATION)` does not redirect it). `media`/`notification`
    /// are fixed applier mappings.
    pub fn create_track(cfg: super::TrackConfig) -> u32 {
        // Self-heal a respawned audioserver's uninitialized volume range (-1 → no
        // gain → silent call) before opening the stream. See ensure_initialized.
        crate::audio_policy_impl::ensure_initialized();
        use crate::audio_routing::Route;
        let route = match cfg.class {
            super::StreamClass::Media        => Route::Media,
            super::StreamClass::Notification => Route::Notification,
            super::StreamClass::VoiceCall    => Route::Call { speaker: super::comms_route_speaker() },
        };
        open_routed(cfg, route)
    }

    /// Open a PCM mic-capture stream (AAUDIO_DIRECTION_INPUT). Symmetric to
    /// create_track; capture handles share the track-handle space, so
    /// start / pause / pending-frames / close all work on them unchanged.
    /// The PCM frames flow the other way — drain them with read_pcm_f32.
    pub fn create_capture(cfg: super::TrackConfig) -> u32 {
        let plan = crate::audio_routing::DeviceModel::get().resolve_capture();
        let (channels, channel_mask) = channel_of(&cfg);
        let params = StreamParameters {
            r#channelMask: channel_mask,
            r#sampleRate:  cfg.sample_rate as i32,
            r#sharingMode: plan.sharing,
            r#audioFormat: if plan.f32_format { pcm_f32_format() } else { pcm_i16_format() },
            r#direction:   AAUDIO_DIRECTION_INPUT,
            r#inputPreset: plan.input_preset,
            ..Default::default()
        };
        open_pcm_stream(params, channels, /*capture=*/ true)
    }

    fn pcm_i16_format() -> AudioFormatDescription {
        AudioFormatDescription {
            r#type:     AudioFormatType::PCM,
            r#pcm:      PcmType::INT_16_BIT,
            r#encoding: String::new(),
        }
    }

    /// Task-76 capability-matrix probe: open ONE stream with a fully-specified
    /// config, log the openStream result + the granted `params_out` (device id
    /// / rate / channels / hardware format — the real AAudio-deviceId namespace
    /// vs the policy port id), then immediately close it. Returns the openStream
    /// return value: a positive stream handle on success, the negative AAudio
    /// result code on failure (e.g. -889 UNAVAILABLE), or `i32::MIN` on a binder
    /// transport error. Does NOT mmap or write any audio — no sound is produced.
    /// Read-only investigation only; never wires into the live track path.
    #[allow(clippy::too_many_arguments)]
    pub fn probe_open(
        label: &str,
        direction: i32,
        usage: i32,
        content_type: i32,
        sharing: i32,
        channel_mask: i32,
        f32_format: bool,
        device_ids: Vec<i32>,
        input_preset: i32,
    ) -> i32 {
        let Some(svc) = service() else {
            log::warn!("audio-caps[{label}]: media.aaudio unavailable");
            return i32::MIN;
        };
        let params = StreamParameters {
            r#channelMask: channel_mask,
            r#sampleRate:  48000,
            r#sharingMode: sharing,
            r#audioFormat: if f32_format { pcm_f32_format() } else { pcm_i16_format() },
            r#direction:   direction,
            r#usage:       usage,
            r#contentType: content_type,
            r#inputPreset: input_preset,
            r#deviceIds:   device_ids,
            ..Default::default()
        };
        let req = StreamRequest {
            r#params: params,
            r#attributionSource: Default::default(),
            r#sharingModeMatchRequired: false,
            r#inService: false,
        };
        let mut po = StreamParameters::default();
        let rc = match svc.r#openStream(&req, &mut po) {
            Ok(h)  => h,
            Err(e) => { log::warn!("audio-caps[{label}]: openStream binder err: {e:?}"); return i32::MIN; }
        };
        if rc > 0 {
            log::info!(
                "audio-caps[{label}]: OPEN ok handle={rc} granted_deviceIds={:?} \
                 rate={} chMask=0x{:x} hwRate={} hwSpf={}",
                po.r#deviceIds, po.r#sampleRate, po.r#channelMask,
                po.r#hardwareSampleRate, po.r#hardwareSamplesPerFrame,
            );
            let _ = svc.r#closeStream(rc);
        } else {
            log::info!("audio-caps[{label}]: OPEN failed rc={rc}");
        }
        rc
    }

    /// Task-76 matrix: do an OUTPUT and an INPUT stream open *simultaneously*?
    /// (Task 75 left this ambiguous — MMAP in+out historically -889'd, but a
    /// SHARED+SHARED pair seemed to coexist once.) Opens both SHARED/F32, logs
    /// each rc, then closes both. Returns (out_rc, in_rc). No audio produced.
    pub fn probe_coexist() -> (i32, i32) {
        let Some(svc) = service() else { return (i32::MIN, i32::MIN) };
        let mk = |dir: i32, mask: i32, usage: i32, preset: i32| StreamRequest {
            r#params: StreamParameters {
                r#channelMask: mask,
                r#sampleRate:  48000,
                r#sharingMode: AAUDIO_SHARING_MODE_SHARED,
                r#audioFormat: pcm_f32_format(),
                r#direction:   dir,
                r#usage:       usage,
                r#inputPreset: preset,
                ..Default::default()
            },
            r#attributionSource: Default::default(),
            r#sharingModeMatchRequired: false,
            r#inService: false,
        };
        let mut po = StreamParameters::default();
        let out_rc = svc.r#openStream(
            &mk(AAUDIO_DIRECTION_OUTPUT, AAUDIO_CHANNEL_STEREO, AAUDIO_USAGE_MEDIA, 0),
            &mut po,
        ).unwrap_or(i32::MIN);
        let in_rc = svc.r#openStream(
            &mk(AAUDIO_DIRECTION_INPUT, AAUDIO_CHANNEL_MONO, 0, AAUDIO_INPUT_PRESET_VOICE_RECOGNITION),
            &mut po,
        ).unwrap_or(i32::MIN);
        log::info!("audio-caps[coexist]: out_rc={out_rc} in_rc={in_rc} (both SHARED/F32)");
        if out_rc > 0 { let _ = svc.r#closeStream(out_rc); }
        if in_rc > 0 { let _ = svc.r#closeStream(in_rc); }
        (out_rc, in_rc)
    }

    /// Shared open path for both playback (capture=false → downDataQueue,
    /// host→HAL) and capture (capture=true → upDataQueue, HAL→host):
    /// openStream → mmap the endpoint's SharedFileRegions → resolve the
    /// ring's counter/data pointers → register a TrackState → return its
    /// handle (0 on failure).
    fn open_pcm_stream(params: StreamParameters, channels: u32, capture: bool) -> u32 {
        let Some(svc) = service() else { return 0 };
        // AttributionSourceState left as the empty-stub default — see the
        // .aidl file for why a full-shape version isn't currently emittable
        // by rsbinder-aidl 0.7.0. The service auto-fills pid/uid from the
        // binder caller context, which is enough to reach the SHARED-mode
        // dispatch path (verified for INPUT too — --probe-audio-capture).
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

        // Playback uses the down-queue (host→HAL). Capture is meant to use
        // the up-queue (HAL→host), but AAudio's Endpoint.aidl notes the
        // record ring "could share same queue" — and on the Pixel 2 XL the
        // up-queue comes back empty while the data ring lands in the
        // down-queue. So for capture, take whichever data queue the service
        // actually populated (non-zero capacity), preferring the up-queue.
        let rb = if capture {
            let up = endpoint.r#upDataQueueParcelable;
            if up.r#capacityInFrames > 0 {
                up
            } else {
                log::info!("audio: capture up-queue empty — using down-queue (shared ring)");
                endpoint.r#downDataQueueParcelable
            }
        } else {
            endpoint.r#downDataQueueParcelable
        };
        let bytes_per_frame = rb.r#bytesPerFrame as u32;
        let capacity_frames = rb.r#capacityInFrames as u32;
        if bytes_per_frame == 0 || capacity_frames == 0 {
            log::warn!(
                "audio: empty data queue (capture={capture}, bpf={bytes_per_frame}, cap={capacity_frames})"
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
        // Also resolve the service→client up-message queue counters so we can
        // drain it (see TrackState). Best-effort: if it can't be resolved we
        // still run, just risk the suspend-on-full the drain prevents.
        let up = &endpoint.r#upMessageQueueParcelable;
        let up_msg_read_p  = resolve(&up.r#readCounterParcelable).map(|p| p as *mut AtomicI64);
        let up_msg_write_p = resolve(&up.r#writeCounterParcelable).map(|p| p as *mut AtomicI64);
        let (Some(read_ctr_p), Some(write_ctr_p), Some(data_p)) =
            (read_ctr_p, write_ctr_p, data_p)
        else {
            log::warn!("audio: data-queue SharedRegion resolution failed");
            let _ = svc.r#closeStream(stream_handle);
            return 0;
        };

        // AtomicI64 has the same layout as i64 (`#[repr(C, align(8))]`).
        // The counter slots in AAudio's shared memory are 8-byte aligned
        // int64s by the protocol; the cast is sound.
        let state = TrackState {
            stream_handle,
            _mmaps:          mmaps,
            read_ctr_ptr:    read_ctr_p  as *mut AtomicI64,
            write_ctr_ptr:   write_ctr_p as *mut AtomicI64,
            data_ptr:        data_p,
            capacity_frames,
            bytes_per_frame,
            channels,
            up_msg_read_ptr:  up_msg_read_p,
            up_msg_write_ptr: up_msg_write_p,
        };
        let id = alloc_handle(state);
        log::info!(
            "audio: {} {id} ready — stream_handle={stream_handle} \
             cap_frames={capacity_frames} bpf={bytes_per_frame}",
            if capture { "capture" } else { "track" },
        );
        id
    }

    /// Drain the service→client up-message queue (timestamps / stream events) by
    /// advancing our read cursor to the service's write cursor and discarding the
    /// contents. We poll-drive and don't need the events, but if this queue fills
    /// the service decides the client stopped and suspends + closes the stream.
    fn drain_up_messages(st: &TrackState) {
        if let (Some(rp), Some(wp)) = (st.up_msg_read_ptr, st.up_msg_write_ptr) {
            // SAFETY: same shared-ring contract as the data-queue counters.
            let w = unsafe { &*wp }.load(Ordering::Acquire);
            unsafe { &*rp }.store(w, Ordering::Release);
        }
    }

    pub fn write_pcm_f32(track: u32, samples: &[f32]) -> u32 {
        with_track(track, |st| {
            drain_up_messages(st);
            // SAFETY: ptrs reference an 8-byte aligned i64 slot inside an
            // mmap shared with media.aaudio. Cross-process atomic ops
            // on the counter pair are AAudio's signaling contract.
            let read_ctr  = unsafe { &*st.read_ctr_ptr };
            let write_ctr = unsafe { &*st.write_ctr_ptr };

            let r = read_ctr.load(Ordering::Acquire) as u64;
            let mut w = write_ctr.load(Ordering::Relaxed) as u64;
            // Underrun resync. The HAL's read cursor advances at the sample clock
            // whether or not we feed it, so on starvation it catches up to / passes
            // our write cursor (r >= w). Computing `w - r` then wraps to a huge
            // value, the free-space guard sees zero room, and EVERY subsequent
            // write is rejected — only the initial prime lands and the speaker goes
            // silent. This is gotcha #5 for *streaming* playback (the batch-write
            // call-live repro pre-filled a huge buffer so the ring never drained).
            // Treat r >= w as an empty ring: jump our write cursor to the read head
            // (drop the silent gap) and write fresh audio at the play position — a
            // brief glitch, but continuous sound instead of a 40 ms blip.
            if r >= w {
                w = r;
                write_ctr.store(w as i64, Ordering::Relaxed);
            }
            let in_flight = w - r;
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

            // Per-app mute (host PCM gate): when this app is muted we write
            // SILENCE into the ring instead of the samples, but still advance
            // the write cursor so playback timing is preserved. (f32 0.0 is
            // all-zero bytes, so write_bytes(0) zeroes the floats.) This is
            // independent of the global policy mute — audio is audible only if
            // both gates are open.
            let muted = super::app_output_muted();
            // SAFETY: bounds checked above — base_off + first <= cap_bytes;
            // (src_bytes - first) <= base_off; samples has at least
            // to_write*channels f32s, so src_bytes <= samples.len()*4.
            unsafe {
                if muted {
                    std::ptr::write_bytes(st.data_ptr.add(base_off as usize), 0, first);
                    if first < src_bytes {
                        std::ptr::write_bytes(st.data_ptr, 0, src_bytes - first);
                    }
                } else {
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
            }

            // Publish — Release pairs with the service's Acquire load.
            write_ctr.store(
                w.wrapping_add(to_write as u64) as i64,
                Ordering::Release,
            );
            to_write
        }).unwrap_or(0)
    }

    /// Consumer mirror of write_pcm_f32 for capture streams: drain up to
    /// `max_frames` frames the HAL has produced into the up-queue ring.
    /// We are the READER — load the service's writeCounter (Acquire), copy
    /// out interleaved f32, then advance our readCounter (Release, pairing
    /// with the service's Acquire load). Returns the captured samples
    /// (`frames × channels`), or empty if nothing is ready yet.
    pub fn read_pcm_f32(capture: u32, max_frames: u32) -> Vec<f32> {
        with_track(capture, |st| {
            drain_up_messages(st);
            // SAFETY: same shared-ring contract as write_pcm_f32, roles
            // reversed — see TrackState's read_ctr_ptr note.
            let read_ctr  = unsafe { &*st.read_ctr_ptr };
            let write_ctr = unsafe { &*st.write_ctr_ptr };

            let w = write_ctr.load(Ordering::Acquire) as u64;
            let r = read_ctr.load(Ordering::Relaxed) as u64;
            let available = w.wrapping_sub(r);
            let to_read = available.min(max_frames as u64) as u32;
            if to_read == 0 { return Vec::new(); }

            let bpf       = st.bytes_per_frame as u64;
            let cap_bytes = st.capacity_frames as u64 * bpf;
            // wrap-aware byte offset into the data ring
            let base_off  = (r * bpf) % cap_bytes;
            let want      = (to_read as u64 * bpf) as usize;
            let first     = want.min((cap_bytes - base_off) as usize);

            let mut out = vec![0.0f32; (to_read * st.channels) as usize];
            // SAFETY: out is to_read*channels f32 = `want` bytes; base_off +
            // first <= cap_bytes; (want - first) <= base_off. Source is the
            // shared ring the HAL fills.
            unsafe {
                let dst = out.as_mut_ptr() as *mut u8;
                std::ptr::copy_nonoverlapping(
                    st.data_ptr.add(base_off as usize),
                    dst,
                    first,
                );
                if first < want {
                    std::ptr::copy_nonoverlapping(
                        st.data_ptr,
                        dst.add(first),
                        want - first,
                    );
                }
            }

            // Consume — Release pairs with the service's Acquire load of readCounter.
            read_ctr.store(
                r.wrapping_add(to_read as u64) as i64,
                Ordering::Release,
            );
            // Mic-mute (input gate): still DRAIN the ring (keep capture flowing,
            // no overflow) but hand the guest SILENCE, so it sends silence and
            // the peer hears nothing. Arbiter-decided; orthogonal to anything
            // else. Dormant until a guest actually opens capture.
            if super::mic_muted() { out.iter_mut().for_each(|s| *s = 0.0); }
            out
        }).unwrap_or_default()
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

// ── Comms route (arbiter-decided) ────────────────────────────────────────────
// The arbiter (wart-arbiter-audio) owns the call earpiece↔speaker decision and
// pushes it down as `audio-policy set-route` (handled in standalone.rs, which
// calls `set_comms_route`). A guest `create-track` (a call stream) is pinned to
// the resolved device for this route. Default earpiece (`false`). See
// [[project_audio_routing_arbiter]]. Live mid-call re-pin (speakerphone toggle
// after the stream is open) is a follow-up — the route applies at open time.
static COMMS_SPEAKER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Apply the arbiter's call route decision (`true` = loudspeaker / speakerphone,
/// `false` = earpiece). Affects subsequently-opened call streams.
pub fn set_comms_route(speaker: bool) {
    COMMS_SPEAKER.store(speaker, std::sync::atomic::Ordering::Relaxed);
    log::info!("audio: comms route = {}", if speaker { "speaker" } else { "earpiece" });
}
/// The arbiter's current call route (`true` = speaker, `false` = earpiece).
pub fn comms_route_speaker() -> bool {
    COMMS_SPEAKER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Per-app output mute — a process-wide gate (one wart-host process = one app)
/// the host applies at the PCM source in `write_pcm_f32` (writes silence). The
/// arbiter decides which app to mute and pushes `audio-policy app-mute`. This is
/// orthogonal to the global policy mute: audio is audible only if BOTH gates are
/// open. New tracks honour the flag automatically.
static APP_OUTPUT_MUTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_app_output_muted(muted: bool) {
    APP_OUTPUT_MUTED.store(muted, std::sync::atomic::Ordering::Relaxed);
    log::info!("audio: app output {}", if muted { "MUTED" } else { "unmuted" });
}
pub fn app_output_muted() -> bool {
    APP_OUTPUT_MUTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Mic-mute / input-disable — a process-wide gate (one wart-host = one app) the
/// host applies at the capture READ path in `read_pcm_f32` (returns silence
/// while still draining the ring). Arbiter-decided (`audio-policy mic-mute`).
/// Dormant until a guest opens capture (today Signal is RX-only). When outbound
/// mic (P1/TX) lands this mutes the mic to the peer + kills speakerphone
/// microphony.
static MIC_MUTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_mic_muted(muted: bool) {
    MIC_MUTED.store(muted, std::sync::atomic::Ordering::Relaxed);
    log::info!("audio: mic {}", if muted { "MUTED" } else { "unmuted" });
}
pub fn mic_muted() -> bool {
    MIC_MUTED.load(std::sync::atomic::Ordering::Relaxed)
}

// ── Host-internal playback API ───────────────────────────────────────────────
// Module-level free functions so a background thread (the ringer) can drive AAudio
// directly without the WIT `Host` trait (`&mut HostState`). Same cfg-switch as the
// trait methods below; non-Android is a no-op (returns 0/false).

pub fn create_track(cfg: TrackConfig) -> u32 {
    #[cfg(target_os = "android")]
    { return binder_path::create_track(cfg); }
    #[cfg(not(target_os = "android"))]
    { let _ = cfg; 0 }
}

/// Open an output track for an explicit intent [`crate::audio_routing::Route`]
/// (host-side callers that know their intent, e.g. the ringer → `Ringtone`).
pub fn create_track_routed(cfg: TrackConfig, route: crate::audio_routing::Route) -> u32 {
    #[cfg(target_os = "android")]
    { return binder_path::open_routed(cfg, route); }
    #[cfg(not(target_os = "android"))]
    { let _ = (cfg, route); 0 }
}

pub fn write_pcm_f32(track: u32, samples: &[f32]) -> u32 {
    #[cfg(target_os = "android")]
    { return binder_path::write_pcm_f32(track, samples); }
    #[cfg(not(target_os = "android"))]
    { let _ = (track, samples); 0 }
}

pub fn start(track: u32) -> bool {
    #[cfg(target_os = "android")]
    { return binder_path::start(track); }
    #[cfg(not(target_os = "android"))]
    { let _ = track; false }
}

pub fn close(track: u32) {
    #[cfg(target_os = "android")]
    { binder_path::close(track); }
    #[cfg(not(target_os = "android"))]
    { let _ = track; }
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

    fn open_capture(&mut self, cfg: TrackConfig) -> TrackHandle {
        #[cfg(target_os = "android")]
        { return binder_path::create_capture(cfg); }
        #[cfg(not(target_os = "android"))]
        { let _ = cfg; 0 }
    }

    fn read_pcm_f32(&mut self, capture: TrackHandle, max_frames: u32) -> Vec<f32> {
        #[cfg(target_os = "android")]
        { return binder_path::read_pcm_f32(capture, max_frames); }
        #[cfg(not(target_os = "android"))]
        { let _ = (capture, max_frames); Vec::new() }
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

/// Mic→speaker loopback verify (`wart-host --probe-audio-loopback`):
/// exercises the full capture path (create_capture + read_pcm_f32) and
/// the output path together — you hear yourself. See `binder_path::probe_loopback`.
#[cfg(target_os = "android")]
pub fn probe_loopback() {
    binder_path::probe_loopback();
}
#[cfg(not(target_os = "android"))]
pub fn probe_loopback() {
    log::warn!("probe-loopback: android-only build");
}

/// Task 76 P1 — call-order full-duplex capture probe with a given input preset.
#[cfg(target_os = "android")]
pub fn probe_duplex(preset: i32) { binder_path::probe_duplex(preset); }
#[cfg(not(target_os = "android"))]
pub fn probe_duplex(_preset: i32) { log::warn!("probe-duplex: android-only build"); }

/// Play a sine tone to the speaker (arbiter `play-tone` host applier / warm-up).
#[cfg(target_os = "android")]
pub fn play_tone(ms: u32, hz: f32, vol: f32) { binder_path::play_tone(ms, hz, vol); }
#[cfg(not(target_os = "android"))]
pub fn play_tone(_ms: u32, _hz: f32, _vol: f32) { log::warn!("play-tone: android-only build"); }

/// Task 97 bug #1 stall repro (`--probe-call-stall`). See `binder_path::probe_call_stall`.
#[cfg(target_os = "android")]
pub fn probe_call_stall(secs: u32, speaker: bool, drain_in_pause: bool) {
    binder_path::probe_call_stall(secs, speaker, drain_in_pause);
}
#[cfg(not(target_os = "android"))]
pub fn probe_call_stall(_secs: u32, _speaker: bool, _drain_in_pause: bool) {
    log::warn!("probe-call-stall: android-only build");
}

/// Task-76 capability-matrix open probe (`--probe-audio-matrix`, via
/// `audio_caps`): open one fully-specified stream, log the result + granted
/// params, close. Returns the AAudio openStream code (handle>0 ok, negative =
/// failure code such as -889, `i32::MIN` = binder error). Read-only.
#[cfg(target_os = "android")]
#[allow(clippy::too_many_arguments)]
pub fn probe_open(
    label: &str, direction: i32, usage: i32, content_type: i32,
    sharing: i32, channel_mask: i32, f32_format: bool,
    device_ids: Vec<i32>, input_preset: i32,
) -> i32 {
    binder_path::probe_open(
        label, direction, usage, content_type, sharing,
        channel_mask, f32_format, device_ids, input_preset,
    )
}
#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub fn probe_open(
    _label: &str, _direction: i32, _usage: i32, _content_type: i32,
    _sharing: i32, _channel_mask: i32, _f32_format: bool,
    _device_ids: Vec<i32>, _input_preset: i32,
) -> i32 { i32::MIN }

/// Task-76 matrix: open OUTPUT + INPUT simultaneously (both SHARED/F32), log
/// each rc, close both. Returns (out_rc, in_rc).
#[cfg(target_os = "android")]
pub fn probe_coexist() -> (i32, i32) { binder_path::probe_coexist() }
#[cfg(not(target_os = "android"))]
pub fn probe_coexist() -> (i32, i32) { (i32::MIN, i32::MIN) }

// Silence "unused" warnings when targeting desktop where binder_path is gone.
#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn _unused(c: ChannelLayout, f: Format) {
    let _ = (c, f);
}
