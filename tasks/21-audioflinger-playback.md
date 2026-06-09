# Task 21 — Audio playback via rsbinder (IAAudioService)

> **Status: ✅ device-verified 2026-05-17 — 440 Hz beep audible on Pixel 2 XL speaker at app launch.** Full Path B-AAudio pipeline working end-to-end (WIT `audio` → Kotlin bindings → wasm guest → wandr-host `audio_impl.rs` → rsbinder → `media.aaudio` → AAudioFlowGraph → MMAP HAL → speaker). All sub-tasks A1–B5 complete.

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

### ✅ A2 — `binder_shared_memory` Rust primitive (done 2026-05-17)
`wandr-host/src/binder_shared_memory.rs`. Cross-platform (memmap2 +
libc memfd test) — slight deviation from the original spec, which put
memmap2 under `[target.'cfg(target_os = "android")'.dependencies]`. The
core mmap step has no Android-specific dependencies; keeping it
cross-platform means the round-trip test runs on the Linux dev host as
`cargo test --target x86_64-unknown-linux-gnu`, matching the spec's
"host-side test, no device needed" requirement. The
`ParcelFileDescriptor → OwnedFd` extraction is intentionally deferred to
B3 so this primitive doesn't depend on the rsbinder codegen that B2
produces.

- API: `BinderMappedMemory::map(fd: OwnedFd, offset: i64, size: i64, writeable: bool) -> io::Result<Self>`
- Accessors: `as_slice() -> &[u8]`, `as_mut_slice() -> Option<&mut [u8]>`, `len()`, `is_empty()`, `is_writeable()`
- Negative `offset`/`size` rejected with `InvalidInput` before mmap call
- 6 unit tests pass (read-only round-trip, writeable round-trip, ro
  `as_mut_slice()` returns None, neg-offset rejection, neg-size
  rejection, page-aligned non-zero offset window)
- `memmap2 = "0.9"` added to `[dependencies]`, `libc = "0.2"` added to
  `[dev-dependencies]`
- Android cross-build: clean in 2m 12s; only dead-code warnings on the
  new pub fns (A3/B3 will consume them).

### ✅ A3 — `eventfd_signal` Rust primitive (done 2026-05-17)
`wandr-host/src/eventfd_signal.rs`. Cross-platform (libc) — not nix, to
match A2's host-testable shape; libc was already pulled transitively and
is now an explicit dep covering both A2's memfd tests and A3's eventfd
syscalls. The `ParcelFileDescriptor → OwnedFd` extraction stays at the
B3 call site.

- API: `from_owned_fd(OwnedFd)`, `create_local(u32)` (blocking),
  `create_local_nonblocking(u32)` (`EFD_NONBLOCK`)
- Operations: `notify(u64) -> Result<()>` writes 8 bytes;
  `wait() -> Result<u64>` blocks and atomic-resets-to-zero;
  `wait_nonblocking() -> Result<Option<u64>>` maps EAGAIN to `None`
- 6 unit tests pass: notify+wait round-trip, accumulating writes
  (1+2+3→6 on single drain), initial-count preload, nonblocking idle
  returns None, nonblocking drain-then-idle, kernel-reserved `u64::MAX`
  notify gets `EINVAL`
- `libc = "0.2"` promoted to `[dependencies]` (was dev-dep from A2)
- Android cross-build: clean in 44.57s incremental; only dead-code
  warnings on the new pub fns (B3 will consume them).

### ✅ B1 — Inspect AAudio C++ protocol (done 2026-05-17)
Protocol documented in the appendix below. Highlights:

- Service name: `media.aaudio` (binder name from `AAudioBinderClient.cpp`).
- Control plane: 11 IAAudioService methods; v1 needs 5
  (`openStream`, `getStreamDescription`, `startStream`, `pauseStream`,
  `closeStream`).
- Data plane: `Endpoint` contains 4 RingBuffers + a pool of
  SharedFileRegions. Each RingBuffer's 3 SharedRegions (readCounter,
  writeCounter, data) index into the pool via `sharedMemoryIndex` +
  `offsetInBytes` + `sizeInBytes`. For playback only
  `downDataQueueParcelable` matters.
- Counters: two int64 atomics — producer-written `writeCounter`,
  consumer-written `readCounter`. Standard SPSC ring discipline with
  release-acquire ordering (signaled via `COHERENCY_ACQUIRE_RELEASE`
  flag bit). 64-bit monotonic; modulo-capacityInFrames gives ring
  index.
- **AAudio does NOT use an eventfd in the data plane** — service polls
  `writeCounter` from its own HAL thread, client polls `readCounter`.
  Implication: A3's `EventfdSignal` is unused in `audio_impl.rs`;
  remains valuable for camera + codec2 follow-on tasks.
- `framesPerBurst`: HAL period unit (~96–192 @ 48 kHz on Pixel). Best
  to write multiples of it. `capacityInFrames` is full ring size.
- A2's outer mmap pattern matches `SharedMemoryParcelable.cpp:91`
  exactly. AAudio always passes `offset=0` at the SharedFileRegion
  level; A2 accepting page-aligned non-zero outer offsets is forward
  compat for other services.
- Vendor sparse-checkout doesn't include `src/core/AudioStreamInternal.cpp`
  (carries the actual write-frames loop). B3 implements the ordering
  from the COHERENCY flag contract + standard SPSC discipline; if a
  surprise comes up, broaden the sparse-checkout then.

### ✅ B2 — IAAudioService AIDL codegen + WIT (done 2026-05-17)
Faster than the 4–6h estimate (~1h). rsbinder-aidl auto-emits all
transitive parcelables from `include_dir`s once the sources reference
them — **no extra stubs needed beyond `AttributionSourceState` from
A1**. The whack-a-mole risk didn't materialize for this AIDL surface
(audio/common is more self-contained than sensors was).

- `build.rs`: added `aaudio_aidl`, `shmem_aidl`, `audio_common_aidl`
  include dirs; added `IAAudioService.aidl` + `IAAudioClient.aidl` as
  `.source()`. The existing `aidl-stubs/AttributionSourceState`
  satisfies `StreamRequest`'s frameworks/base import.
- Generated `aosp_hal_bindings.rs` (~10.5k lines) now includes:
  - `aaudio::{Endpoint, IAAudioClient, IAAudioService, RingBuffer, SharedRegion, StreamParameters, StreamRequest}`
  - `android::media::SharedFileRegion`
  - `android::media::audio::common::*` (AudioFormatDescription,
    AudioFormatType, PcmType, plus ~50 other audio/common types
    pulled transitively from the include_dir)
- `wit/skiko-gfx.wit` has new `interface audio` (TrackConfig record;
  Format + ChannelLayout enums; `create-track`, `write-pcm-f32`,
  `start`, `pause`, `close`, `pending-frames`); `import audio;` added
  to world. Mirrored to `skiko/skiko/wit/skiko-gfx.wit` per CLAUDE.md.
- `wandr-host/src/audio_impl.rs`: stub `Host` impl returning 0/false on
  every method so the bindgen-generated trait is satisfied. B3 fills
  it in.
- Build verify: Android cross-compile 49s; desktop x86_64 8m 26s
  first-time (skia-safe rebuild). Both clean apart from dead-code
  warnings on the unused new modules.

Caveat for future work: cargo's per-package build hash dirs can leave
a stale `wasm-android-host-*/out` from earlier session work (the rename
task left one); the live `OUT_DIR` is the newer hash. Inspecting
codegen output should always be done on the most recent build dir.

### ✅ B3 — `audio_impl.rs` — AAudio protocol on primitives (done 2026-05-17)
Implementation landed in one pass after two trivial compile fixes (a
missing `ChannelLayout` import and a `let resolve` that needed `let mut`
because the captured `Vec<BinderMappedMemory>` is `get_mut`'d). No
eventfd plumbing (per B1's finding).

- `OnceLock<Strong<dyn IAAudioService>>` reaching
  `rsbinder::hub::get_interface("media.aaudio")`.
- Per-track state (`TrackState`): `stream_handle: i32`, owned
  `Vec<BinderMappedMemory>`, three raw pointers into the mmap'd ring
  (`read_ctr_ptr: *const AtomicI64`, `write_ctr_ptr: *mut AtomicI64`,
  `data_ptr: *mut u8`), `capacity_frames`, `bytes_per_frame`,
  `channels`. `unsafe impl Send` (wasmtime store is single-threaded).
- `Mutex<(u32, HashMap<u32, TrackState>)>` for the next-id counter +
  open tracks, behind a `OnceLock` for lazy init.
- `create_track(cfg)`:
  1. Build `StreamParameters` (PCM-f32, requested sample-rate,
     channelMask, OUTPUT direction, MEDIA usage).
  2. `svc.openStream(&req, &mut params_out)` → handle (≤ 0 = error).
  3. `svc.getStreamDescription(handle, &mut endpoint)`.
  4. For each `SharedFileRegion`: extract `OwnedFd` via the rsbinder
     `From<ParcelFileDescriptor>` impl and `BinderMappedMemory::map`.
  5. Resolve the three `SharedRegion`s of `downDataQueueParcelable`
     into raw pointers within the mmaps. Cast the 8-byte counter slots
     to `AtomicI64` pointers (`#[repr(C, align(8))]` makes this sound).
  6. Insert state into the map, return guest-visible handle.
- `write_pcm_f32`: load readCounter Acquire / writeCounter Relaxed,
  compute `in_flight = w − r`, available = `capacity − in_flight`,
  copy `min(requested, available) * bytesPerFrame` bytes into the ring
  with a wrap-aware two-`copy_nonoverlapping` split, store
  writeCounter Release.
- `start` / `pause` / `close` pass through to the binder calls.
  `close` drops the state; mmaps auto-`munmap` via memmap2, OwnedFds
  auto-close via std.
- `pending_frames`: same counter load pair, returns `w − r` clamped to
  `u32::MAX`.
- AAudio C constants hard-coded (`DIRECTION_OUTPUT=0`,
  `SHARING_MODE_SHARED=0`, `USAGE_MEDIA=1`, `CHANNEL_MONO=0b01`,
  `CHANNEL_STEREO=0b11`). We don't link libaaudio; values are stable
  AOSP contracts.
- `registerClient(IAAudioClient)` deliberately skipped. If B5 device
  verify shows openStream rejecting on the missing client, fall back
  to the `BnAAudioClient` stub pattern from task 20.
- Build: Android cross-compile 51s; desktop x86_64 cached 0.27s.
  Both clean; 8 dead-code warnings on A3's eventfd_signal (unused per
  B1) + a couple of unused BinderMappedMemory accessors.
- 12 host unit tests (A2 + A3) still pass.

### ✅ B4 — Hand-edit Kotlin bindings (done 2026-05-17)
Pattern matched the existing `createLinearGradient(stops: List<Float>)`
list<f32> shape exactly; clipboard.setText was the secondary reference
for the linear-memory `withScopedMemoryAllocator` ABI.

- `generated/InternalSkikoUi.kt`: appended 6 `@WasmImport` externs
  — `__wasm_import_audio_{createTrack, writePcmF32, start, pause,
  close, pendingFrames}`. `create-track` flattens the record into 3
  i32 params (sample-rate, channel-layout ordinal, format ordinal);
  `write-pcm-f32` takes (track, ptr, len) for the list<f32>.
- `generated/SkikoUi.kt`: appended `@WitInterface("my:skiko-gfx/audio@0.1.0")
  interface Audio` with `Format`/`ChannelLayout` enums + `TrackConfig`
  data class, plus the `companion object Import : ...wasi.wit.Audio`
  delegating to the externs.
- `write-pcm-f32` Kotlin body: `allocate(samples.size * 4)`,
  per-element `storeInt(el.toBits())`, then call import. Empty list
  passes ptr=0, len=0 (the host stub returns 0 in that case — safe
  no-op). Bulk-copy optimization deferred; v1 throughput is gated by
  HAL period rate, not Kotlin loop speed.
- `skiko-wasm-wasi` klib republished via
  `./gradlew publishWasmWasiPublicationToMavenLocal` in 47s — build
  clean, no compile warnings on the new code.

### ✅ B5 — Build verify + device verify 440 Hz sine (done 2026-05-17)
**Beep audible on Pixel 2 XL speaker at app launch.** Five issues
surfaced during the on-device pass, each documented below; once all
were fixed the smoke test reported
`track=1 wrote=1536/9600 frames started=true pending=1536`. The 1536
written = the MMAP buffer capacity at 32 ms / 48 kHz; the remaining
8064 frames are dropped silently (out of scope for v1, where the
goal is "does the pipeline play sound" not "is steady-state
streaming wired").

**Iss #1 — AAUDIO_SHARING_MODE constants were swapped.** The AOSP
header lists `AAUDIO_SHARING_MODE_EXCLUSIVE = 0,
AAUDIO_SHARING_MODE_SHARED = 1` (historical order, not alphabetical).
B3 had hard-coded `SHARED=0` so every openStream was actually
requesting EXCLUSIVE, forcing the MMAP-only path with no fallback.
Fixed by swapping the constant values + adding a "got bitten once"
comment so the next maintainer doesn't repeat it.

**Iss #2 — `registerClient(IAAudioClient)` was required.** B3
deliberately skipped it on the bet that the service would tolerate a
missing client. It does not — without a registered Bn-side callback
sink, the SHARED-mode dispatch refuses to proceed. Added an
`AAudioClientStub` (Bn server with a no-op `onStreamChange`) and a
tokio current-thread runtime in the same shape as
`sensors_impl.rs::EventCollector`, registered once on first `service()`
call.

**Iss #3 — Pixel 2 XL's MMAP HAL only supports stereo.** With SHARED
mode + registerClient, the service was reaching MMAP open with our
mono mask (0x1), and the HAL was rejecting and "suggesting"
`channel_mask=0x3`. The fix was to switch the WIT smoke test to
`ChannelLayout::STEREO`, duplicating the 440 Hz sine into both
channels (interleaved L,R). Mono support would require the legacy
AudioFlinger path (which AAudio's SHARED endpoint does support but
this particular HAL exposes only stereo for MMAP). Out of scope.

**Iss #4 — `writeable=false` SharedFileRegions must still be
mmap-PROT_RW.** AAudio sends ALL SharedFileRegions with
`writeable=false`, even the data buffer + writeCounter. A2's primitive
honored the flag strictly and mapped read-only, which caused a
SEGV_ACCERR at the first `AtomicI64::store` / `copy_nonoverlapping`.
AOSP's libaaudio (`SharedMemoryParcelable::resolveSharedMemory`)
hard-codes `PROT_READ|PROT_WRITE|MAP_SHARED` and ignores the flag.
Mirrored that at the AAudio call site by passing
`/*writeable=*/true` to `BinderMappedMemory::map` regardless of the
flag value. A2 stays semantically correct for other callers — the
writeable flag IS load-bearing for some services (e.g. read-only
config payloads).

**Iss #5 — Pre-fill before start, not after.** B3 + the original
smoke test called `start()` first, then `writePcmF32()`. Once
`start()` returns, the service's mixer/HAL thread is live and
polling readCounter against an empty ring; `readCounter` advances
before we get a chance to write, so our `w − r` wraps to ~u64::MAX
and the "ring is full" guard rejects every subsequent write
(`wrote=0/9600, pending=u32::MAX`). Standard AAudio pattern is
write-then-start; rewrote the smoke test accordingly.

**AttributionSourceState — left as empty stub.** A mid-debug
hypothesis was that `packageName=null` was blocking the SHARED
dispatch path. Tried upgrading the stub to the real shape with
`packageName="com.example.wasmruntime"`; build failed because
`AttributionSourceState[] next` (recursive) generates
`Vec<Box<Self>>` and rsbinder-aidl 0.7.0 lacks `SerializeArray` /
`DeserializeArray` impls for `Box<T>`. Substituting `int[]` for
`next` compiled but the service rejected the parcel with BAD_TYPE
(field-count mismatch). Reverted to the empty stub — once Iss #1–#5
were fixed the service accepts our `attributionSource` and
auto-fills pid/uid from the binder caller context, which is enough.
If a future task needs the real AttributionSourceState (camera
service uses it for audit-log routing), the fix is either to
upgrade rsbinder-aidl past 0.7.0 or hand-write the parcel encoder
(the task 16 workaround pattern).

**Scripts added.** `scripts/env-android.sh` (sourced helper exporting
ANDROID_NDK_HOME / ANDROID_HOME / CC_/CXX_/AR_aarch64_linux_android
+ PATH for the API-versioned NDK r27 tools) and `scripts/build-apk.sh`
(`cargo apk build --release` wrapper that treats the known
"Bin is not compatible with Cdylib" panic-after-signing as success).
`scripts/build-host-android.sh` refactored to source the same env
helper.

---

## Known risks (carried from earlier scoping)

1. **AIDL transitive import whack-a-mole** — every task in series 16/19/20 surfaced 1–3 new stubs needed. Audio likely 3–5 more (AudioFormatDescription pulls in AudioFormatType, AudioEncapsulationMode, etc).

2. **rsbinder + ParcelFileDescriptor maturity** — docs note PFD "still in development" in rsbinder 0.7.0. The fd-extraction primitive may need its own workaround.

3. **Async runtime for IAAudioService callbacks** — IAAudioClient callback (events) requires a Bn-side binder server, same as task 20's IEventQueueCallback. Reuse the tokio current-thread runtime pattern. If we DON'T need callbacks (poll model), we can pass a null IAAudioClient if the AIDL allows `@nullable`.

4. **Ring buffer atomic ordering** — the AAudio protocol uses release/acquire ordering on the counters. Getting this wrong = audio glitches or deadlocks. Document carefully in B1; implement carefully in B3.

5. **SELinux on stock devices** — `untrusted_app → media.aaudio` is usually allowed (apps need audio playback), but the chain through audio HAL may still be denied on non-rooted devices. Verify on first test.

## Appendix: AAudio protocol (B1, 2026-05-17)

Read of `wandr-host/vendor/aosp-frameworks-av/media/libaaudio/src/binding/`
+ the AAudio AIDL package. Vendor sparse-checkout only includes
`src/binding/`, not `src/core/AudioStreamInternal.cpp` (which carries the
client-side write-frames loop with atomic ordering); below uses the
binding layer as the protocol spec and infers ordering from the
`COHERENCY_*` flags + standard SPSC-ring discipline.

### Service name + control plane

- Service is registered as `media.aaudio` (AAUDIO_SERVICE_NAME in
  `AAudioBinderClient.cpp:33`).
- AIDL: `aaudio.IAAudioService` — 11 methods:
  `registerClient(IAAudioClient)`, `openStream(StreamRequest) → handle + StreamParameters (out)`,
  `closeStream(handle)`, `getStreamDescription(handle) → Endpoint (out)`,
  `startStream`, `pauseStream`, `stopStream`, `flushStream`,
  `registerAudioThread(handle, tid, periodNs)`, `unregisterAudioThread`,
  `exitStandby(handle) → Endpoint (out)`.
- v1 minimum subset for write-only playback: `openStream`,
  `getStreamDescription`, `startStream`, `pauseStream`, `closeStream`.
  Skip `registerClient` (the `IAAudioClient` event sink) if the AIDL
  marks it nullable — see Known Risks #3. If not nullable, we need a
  minimal Bn-side server (pattern from task 20).

### Endpoint layout (data plane)

`aaudio.Endpoint` (Endpoint.aidl) contains four `RingBuffer` slots plus a
shared `sharedMemories: SharedFileRegion[]`:

| slot | direction | purpose | playback uses? |
|------|-----------|---------|----------------|
| `upMessageQueueParcelable`   | server → client | stream events (STARTED/PAUSED/XRUN/DISCONNECTED) | optional v1 |
| `downMessageQueueParcelable` | client → server | (mostly unused in practice) | no |
| `upDataQueueParcelable`      | server → client | recording PCM | no — out of scope |
| `downDataQueueParcelable`    | client → server | **playback PCM** | **yes** |

`sharedMemories` is the pool of fds. Each `RingBuffer` carries three
`SharedRegion`s — `readCounterParcelable`, `writeCounterParcelable`,
`dataParcelable` — and each `SharedRegion` is a `(sharedMemoryIndex,
offsetInBytes, sizeInBytes)` tuple pointing into the pool. In practice
all three regions of a RingBuffer typically share the same
`sharedMemoryIndex` (see `setupMemory(sharedMemoryIndex, dataMemoryOffset,
dataSizeInBytes, readCounterOffset, writeCounterOffset, counterSizeBytes)`
in `RingBufferParcelable.cpp:57`) — single mmap, three offsets within it
— but the protocol allows each region to live in a different fd.

Resolution sequence on the client side
(`RingBufferParcelable::resolve` + `SharedRegionParcelable::resolve`):

1. mmap each `SharedFileRegion` in `Endpoint.sharedMemories[]` via the A2
   primitive (one `BinderMappedMemory` per index).
2. For each RingBuffer slot we care about (just `downDataQueue` for v1):
   for each of its three SharedRegions, look up the mmap at
   `sharedMemoryIndex`, take a slice from `offsetInBytes` to
   `offsetInBytes + sizeInBytes`, and reinterpret:
   - `readCounter`  → `*const AtomicI64` (consumer-written, producer-read)
   - `writeCounter` → `*mut   AtomicI64` (producer-written, consumer-read)
   - `data`         → `&mut [u8]` (capacityInFrames × bytesPerFrame bytes)
3. Resulting `RingBufferDescriptor` (see
   `AAudioServiceDefinitions.h:69-77`) mirrors:
   ```
   struct RingBufferDescriptor {
       uint8_t* dataAddress;
       int64_t* writeCounterAddress;
       int64_t* readCounterAddress;
       int32_t  bytesPerFrame;
       int32_t  framesPerBurst;
       int32_t  capacityInFrames;
       RingbufferFlags flags;        // bitmask, expected COHERENCY_ACQUIRE_RELEASE
   }
   ```

### SPSC counter protocol

Both counters are 64-bit monotonically-increasing total-frame counts (never
wrap; wrap-aware modulo gives the ring index). For playback:

- **client (producer)**:
  ```
  let r = read_counter.load(Acquire);           // most recent service read
  let w = write_counter.load(Relaxed);          // last value WE wrote
  let in_flight  = w - r;                       // frames not yet consumed
  let free_frames = capacity_in_frames - in_flight;
  let to_write   = min(requested, free_frames);
  // copy `to_write` frames into data[(w % capacity) ..], wrapping if needed
  fence(Release);
  write_counter.store(w + to_write, Release);   // publish
  ```
- **service (consumer)** (mirror; we never run this side):
  ```
  let w = write_counter.load(Acquire);
  let r = read_counter.load(Relaxed);
  let in_flight = w - r;
  // consume up to `framesPerBurst` per HAL period
  read_counter.store(r + consumed, Release);
  ```

Atomic ordering: the `RingbufferFlags::COHERENCY_ACQUIRE_RELEASE` bit
(`AAudioServiceDefinitions.h:63`) is the contract — release-store on
counter publish + acquire-load on counter read. The buffer flags also
allow `COHERENCY_DMA` (no software ordering, hardware coherency) and
`COHERENCY_AUTO`; for safety treat anything other than `DMA` as
release-acquire.

### framesPerBurst semantics

`framesPerBurst` is the unit the HAL hands to the DSP per period
(typically 96–192 frames at 48 kHz on a Pixel — varies by chipset). The
client should write in multiples of `framesPerBurst` when the producer
can keep up; the protocol doesn't require it, but writing odd amounts
risks XRUN if the HAL is woken with less than one burst available.
`capacityInFrames` is the full ring size (usually some small multiple of
the burst, e.g. 2× or 3× for double/triple-buffering).

### eventfd? — NO, not in AAudio data plane

The original task scoping assumed AAudio uses an eventfd to signal
data-ready between client and service. Reading the protocol: **no
eventfd**. The service runs its own MMAP/HAL thread that polls
`writeCounter` at the HAL period rate; the client polls `readCounter`
to know free space. The two counter atomics ARE the signaling mechanism.

This means **A3's `EventfdSignal` is NOT consumed by `audio_impl.rs`
directly.** A3 remains valuable as a reusable primitive for the next
binder-data-plane services (CameraService BufferQueue, Codec2 ports —
both use `eventfd` for buffer-available signaling) — keeping it pays
forward into Path B-Camera and Path B-Codec2 from `post-art-roadmap.md`,
not retroactively into B3.

The `upMessageQueueParcelable` *is* available for out-of-band stream
events (STARTED/PAUSED/XRUN/DISCONNECTED). v1 can ignore it — the data
plane works without — but a production impl would poll it once per
audio frame to surface XRUN counts to the guest.

### Open-stream → write-loop sequence

1. `binder.rs::init()` — already done by lib.rs::resumed().
2. `let svc = ServiceManager::wait_for_service("media.aaudio")?;`
   `let svc = IAAudioService::interface(svc)?;`
3. (Maybe) `svc.registerClient(client_bn)?;` — skipped iff AIDL marks
   the arg `@nullable`; see Known Risks #3.
4. `let (handle, params_out) = svc.openStream(stream_request)?;`
   `stream_request.params.audioFormat` = PCM-f32, `sampleRate` = 48000,
   `channelMask` = mono, `direction` = OUTPUT, `usage` = MEDIA.
5. `let endpoint = svc.getStreamDescription(handle)?;`
6. For each `region` in `endpoint.sharedMemories`:
   `BinderMappedMemory::map(extract_fd(region.fd), region.offset, region.size, region.writeable)`
   stored at index `i`.
7. Resolve `endpoint.downDataQueueParcelable` into a
   `RingBufferDescriptor`-equivalent struct (3 raw pointers + scalars).
8. `svc.startStream(handle)?;` — async; service eventually sends STARTED
   on `upMessageQueue` (ignored in v1).
9. Per WIT `write_pcm_f32(handle, samples)` call: load counters with
   the ordering above, copy as many frames as fit (wrapping across the
   ring), publish writeCounter, return frames-written. Guest retries
   leftover frames on the next call.
10. On `close`: `svc.closeStream(handle)?;` then drop our state — mmaps
    auto-`munmap` via memmap2, OwnedFds auto-close.

### Confirmation of A2 design

`SharedMemoryParcelable::resolveSharedMemory`
(`SharedMemoryParcelable.cpp:91`) does exactly what A2 does:
`mmap(nullptr, sizeInBytes, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0)`,
keeping the fd open for the mapping's lifetime. AAudio always passes
`offset=0` at the outer SharedFileRegion level and uses
SharedRegion's `offsetInBytes` to slice within the mmap — the inner
slicing is just pointer arithmetic on `BinderMappedMemory::as_mut_slice()`.
A2 also accepts a non-zero outer offset (page-aligned) for forward
compatibility with services that don't follow AAudio's "outer offset is
always zero" convention.

---

## Out of scope (deferred)

- Audio focus / `acquireFocus` (lives in AudioPolicyService which we're dropping)
- Audio routing (USB / Bluetooth / HDMI / speaker selection)
- Recording (mic capture; different shared-memory direction; needs RECORD_AUDIO)
- Codecs (Codec2 is a separate native daemon — separate task)
- PCM-i16 (PCM-f32 only in v1; adding i16 is a follow-up commit)
- Spatial / Atmos audio
- IAAudioClient callbacks for event-driven model (poll-via-eventfd should be enough for v1)
