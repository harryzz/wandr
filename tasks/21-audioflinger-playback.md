# Task 21 — Audio playback via rsbinder (IAAudioService)

> **Status: 🟡 in-progress. A1 (vendoring) committed; A2–B5 queued for a fresh session.** Resumes from `wart-host` commit `f6a9f9a` (vendors `aosp-frameworks-av` + `aosp-system-hardware-interfaces` at android-15.0.0_r36, plus `AttributionSourceState` stub). Estimated 20–30 hours of focused work remaining (≈3 working days).

## Path decision (resolved 2026-05-17)

**Path B with the IAAudioService variant.** Originally the task scoped two paths (NDK AAudio vs rsbinder → IAudioFlinger). Investigation at kickoff surfaced a third option that beats both:

| | Path A (NDK AAudio) | Path B-orig (IAudioFlinger) | **Path B-AAudio (chosen)** |
|---|---|---|---|
| AIDL methods on target | n/a (FFI to libaaudio.so) | 50+ | **10** |
| Validates Pattern 5 for media | ❌ | ✅ | ✅ |
| Vendoring needed | none | frameworks/av + system/hardware/interfaces + frameworks/base stub + more | **frameworks/av + system/hardware/interfaces + 1 stub** |
| Shared-mem + fd primitives | n/a | required | required (same code) |
| Estimated total work | hours | 3–5 days | **3–4 days** |
| Unlocks camera + codec2 later? | ❌ | ✅ | ✅ |

`media.aaudio` and `media.audio_flinger` are both confirmed present on the Pixel 2 XL (Android 15). `media.aaudio` (the AAudio service daemon at `aaudio.IAAudioService` binder) sits **one layer above** AudioFlinger and is what NDK AAudio talks to internally. For app-level playback it's the right level — AudioFlinger is below it.

## Why factor primitives first

The shared-memory + fd + atomic-ring-buffer pattern that IAAudioService uses is **the same pattern** that IAudioFlinger, CameraService BufferQueue, and Codec2 use:

```
binder RPC (control plane) ←── rsbinder
       │
       ▼
[Service.openSession()] returns:
  ├─ ParcelFileDescriptor   (event fd for signaling)
  ├─ SharedFileRegion       (mmap'd shared memory)
  └─ Atomic ring buffer protocol on top
```

The first two boxes are common; the third differs per service. By factoring the common primitives into reusable modules (`binder_shared_memory` + `eventfd_signal`), the AAudio-specific work in B3 stays narrow, AND the same primitives unlock camera + codec2 with much less per-task effort.

## Existing crates we should compose (not reimplement)

Researched at kickoff — Rust ecosystem has the primitives:

- **`rsbinder::file_descriptor::ParcelFileDescriptor`** — already pulled in; gives us the fd
- **`memmap2`** — battle-tested mmap wrapper
- **`nix::sys::eventfd`** — eventfd primitive for kernel-level signaling
- **`ndk::shared_memory::SharedMemory`** — could compose if useful (we get fd via PFD so probably not needed)
- **`ashmem-rs`** (kinetiknz/ashmem-rs) — wraps ASharedMemory API; useful reference for the older-Android compat fallback even if we don't depend on it

The ~630 lines of AAudio C++ binding code in `frameworks/av/media/libaaudio/src/binding/` are domain-specific (atomic counters per AAudio's particular layout); but the primitives those 630 lines build on are well-trodden in Rust.

## Sub-task plan (A1 done; A2–B5 queued)

### ✅ A1 — Vendor AOSP submodules (committed `f6a9f9a`)
- `vendor/aosp-frameworks-av` (sparse: `media/libaaudio/src/binding/aidl` + `media/libshmem/aidl`) — 432 KB
- `vendor/aosp-system-hardware-interfaces` (sparse: `media/aidl`) — 532 KB
- `vendor/aidl-stubs/android/content/AttributionSourceState.aidl` — stub

### 🔲 A2 — `binder_shared_memory` Rust primitive (~3–4 hours)
New `wart-host/src/binder_shared_memory.rs`:
- Input: `SharedFileRegion { fd: ParcelFileDescriptor, offset: i64, size: i64, writeable: bool }`
- Process: extract raw fd via `into_raw_fd()`, `memmap2::MmapOptions::new().offset(offset).len(size).map_mut(...)` for writable regions or `.map(...)` for read-only
- Output: `BinderMappedMemory { mmap: Mmap or MmapMut }` that exposes `as_slice() -> &[u8]` and `as_mut_slice() -> &mut [u8]`
- Tests: round-trip mmap on a `memfd_create`d fd locally (host-side test, no device needed)
- **Reusable for camera + codec2.** Sized for that audience, not just audio.

Add `memmap2 = "0.9"` to `Cargo.toml` `[target.'cfg(target_os = "android")'.dependencies]`.

### 🔲 A3 — `eventfd_signal` Rust primitive (~1–2 hours)
New `wart-host/src/eventfd_signal.rs`:
- Wrap a `RawFd` obtained from PFD; provide `notify(count: u64) -> Result<()>` (write 8 bytes) and `wait() -> Result<u64>` (read 8 bytes, blocking)
- Use `nix::unistd::{read, write}` directly (we already have nix transitively via rsbinder)
- Optional: `wait_nonblocking()` variant for poll-style use
- **Reusable.** Same primitive for any service that signals via eventfd.

### 🔲 B1 — Inspect AAudio C++ protocol (~2–3 hours)
**Read, don't write.** Document the protocol carefully so B3 has a spec to implement:
- `media/libaaudio/src/binding/RingBufferParcelable.cpp` — counter layout
- `media/libaaudio/src/binding/AudioEndpointParcelable.cpp` — how Endpoint's `sharedMemories[]` maps to RingBuffer's `sharedMemoryIndex`
- `media/libaaudio/src/binding/SharedMemoryParcelable.cpp` — fd extraction + mmap pattern (verifies our A2 design)
- `media/libaaudio/src/binding/AAudioBinderClient.cpp` — control-plane sequence (open → getStreamDescription → enable → write loop)
- `media/libaaudio/src/core/AudioStreamInternal.cpp` (or similar) — the actual write-frames loop with atomic counter ordering

Output: short markdown in this task file's appendix describing the protocol (read counter, write counter, ordering, when to signal eventfd, framesPerBurst semantics).

### 🔲 B2 — IAAudioService AIDL codegen + WIT (~4–6 hours)
- Extend `build.rs` rsbinder-aidl `Builder` with `IAAudioService.aidl` + supporting parcelables (StreamRequest, StreamParameters, Endpoint, RingBuffer, SharedRegion, SharedFileRegion, AudioFormatDescription + transitive deps from `system-hardware-interfaces/media/aidl`)
- Expect transitive-import whack-a-mole — likely 2–4 more stubs needed (e.g. types from `frameworks/base` that AudioFormatDescription depends on). Pattern is the same as task 19/20 stubbing.
- Add WIT `interface audio` to `wit/skiko-gfx.wit`: `create-track / write-pcm-f32 / start / pause / close / pending-frames`. Out of scope: focus, routing, recording, codecs, PCM-i16.
- Sync mirror to skiko.

### 🔲 B3 — `audio_impl.rs` — AAudio protocol on primitives (~6–8 hours)
The hard part. Implements the protocol from B1 using primitives from A2 + A3:
- `OnceLock<Strong<dyn IAAudioService>>` for service
- Per-track state: `IAAudioStream` strong + mmap'd RingBuffers + eventfd
- `create_track(config)`:
  1. Build StreamRequest from WIT TrackConfig (sample rate, channels, AudioFormatDescription PCM-f32)
  2. `svc.openStream(request, out paramsOut)` → stream handle
  3. `svc.getStreamDescription(handle, out endpoint)` → Endpoint
  4. mmap each SharedFileRegion in endpoint.sharedMemories via A2 primitive
  5. Resolve RingBuffer's 3 SharedRegions (read counter, write counter, data) into slices
  6. Store all this in a `HashMap<u32, AudioTrackState>` keyed by guest-visible handle
- `write_pcm_f32(handle, samples)`:
  1. Look up state
  2. Compute available write space from atomic counter delta
  3. Copy as many frames as fit; update write counter
  4. Signal eventfd if necessary (per protocol from B1)
  5. Return frames actually written (may be less than asked — guest retries next frame)
- `start(handle)` → `svc.startStream(handle)`
- `pause(handle)` → `svc.pauseStream(handle)`
- `close(handle)` → `svc.closeStream(handle)`, drop state, mmap drops auto-unmap
- `pending_frames(handle)` → counter delta read

### 🔲 B4 — Hand-edit Kotlin bindings (~2–3 hours)
- `Audio` interface in `SkikoUi.kt`: TrackConfig (3 fields), enums for format / channel-layout
- `write-pcm-f32(handle, list<f32>)` — list<f32> arg means writing the float array to linear memory + passing (ptr, len). Pattern similar to canvas TextBlob.
- `pending-frames` returns u32

### 🔲 B5 — Build verify + device verify 440 Hz sine (~2–4 hours)
- Full pipeline through cargo apk build, skiko klib, wart-app .wasm, cwasm, deploy
- Smoke test in Main.kt: synthesize 1 second of 48000 Hz mono PCM-f32 sine at 440 Hz, `create-track + write-pcm-f32 + start`, hear a beep on the Pixel 2 XL speaker
- Negative test: `setenforce 1` → call should still work (media.aaudio typically allows untrusted_app) or gracefully return false

---

## Known risks (carried from earlier scoping)

1. **AIDL transitive import whack-a-mole** — every task in series 16/19/20 surfaced 1–3 new stubs needed. Audio likely 3–5 more (AudioFormatDescription pulls in AudioFormatType, AudioEncapsulationMode, etc).

2. **rsbinder + ParcelFileDescriptor maturity** — docs note PFD "still in development" in rsbinder 0.7.0. The fd-extraction primitive may need its own workaround.

3. **Async runtime for IAAudioService callbacks** — IAAudioClient callback (events) requires a Bn-side binder server, same as task 20's IEventQueueCallback. Reuse the tokio current-thread runtime pattern. If we DON'T need callbacks (poll model), we can pass a null IAAudioClient if the AIDL allows `@nullable`.

4. **Ring buffer atomic ordering** — the AAudio protocol uses release/acquire ordering on the counters. Getting this wrong = audio glitches or deadlocks. Document carefully in B1; implement carefully in B3.

5. **SELinux on stock devices** — `untrusted_app → media.aaudio` is usually allowed (apps need audio playback), but the chain through audio HAL may still be denied on non-rooted devices. Verify on first test.

## Out of scope (deferred)

- Audio focus / `acquireFocus` (lives in AudioPolicyService which we're dropping)
- Audio routing (USB / Bluetooth / HDMI / speaker selection)
- Recording (mic capture; different shared-memory direction; needs RECORD_AUDIO)
- Codecs (Codec2 is a separate native daemon — separate task)
- PCM-i16 (PCM-f32 only in v1; adding i16 is a follow-up commit)
- Spatial / Atmos audio
- IAAudioClient callbacks for event-driven model (poll-via-eventfd should be enough for v1)
