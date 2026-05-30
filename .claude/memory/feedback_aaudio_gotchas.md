---
name: aaudio-direct-binder-gotchas
description: "Five non-obvious things that bite when talking to media.aaudio via rsbinder instead of libaaudio.so. All fixed in task 21 B5 but worth a fast lookup for follow-on tasks (camera, codec2)."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

When bypassing libaaudio.so and talking to `media.aaudio` (IAAudioService)
directly via rsbinder, these are the five gotchas task 21 B5 hit (in
debugging order). Future binder-shared-memory tasks (camera, codec2)
will likely hit at least #4 and #5 again because they use the same
`SharedFileRegion` + SPSC-ring pattern.

**#1 — `AAUDIO_SHARING_MODE_*` constants are NOT alphabetical.**
`AAUDIO_SHARING_MODE_EXCLUSIVE = 0, AAUDIO_SHARING_MODE_SHARED = 1`
(historical order from when exclusive was the first / "only" mode).
Setting `sharingMode=0` requests EXCLUSIVE, which forces the MMAP-only
path and skips the AudioFlinger SHARED fallback. Symptom: `openStream`
returns -889 (AAUDIO_ERROR_UNAVAILABLE) and the service log says
"could not open in EXCLUSIVE mode" then bails.

**#2 — `registerClient(IAAudioClient)` is required, not optional.**
The AIDL doesn't mark it `@nullable` and the service won't dispatch
SHARED-mode streams without a registered Bn-side callback sink.
Minimal stub: a struct that impl's `IAAudioClientAsyncService` with a
no-op `onStreamChange`, wrapped via `BnAAudioClient::new_async_binder`,
plus the tokio current-thread runtime from `sensors_impl.rs`. Symptom
without registerClient: openStream errors before any service-side
endpoint open is attempted.

**#3 — Pixel 2 XL's MMAP HAL only opens stereo.** The service hints
this via "suggested format=0x1, sample_rate=48000, channel_mask=0x3"
in its `AAudioServiceEndpointMMAP::openWithConfig()` log. Mono
(`AAUDIO_CHANNEL_MONO = 0x1`) is refused; stereo (`0x3` =
FRONT_LEFT|FRONT_RIGHT) works. The service-side `AAudioFlowGraph`
will convert our PCM-f32 to PCM-i16 transparently — we still send
floats. Likely device-specific; newer Pixels probably support more
configs. For new devices, openStream the service's suggested config
if the first attempt fails (out of scope for task 21).

**#4 — `SharedFileRegion::writeable=false` is a LIE.** AAudio always
sets `writeable=false` on the fds it hands clients, even on the data
ring + writeCounter that the client MUST write to. AOSP libaaudio
(`SharedMemoryParcelable::resolveSharedMemory`) hard-codes
`mmap(PROT_READ|PROT_WRITE, MAP_SHARED, ...)` and ignores the flag.
Symptom of honoring it: SEGV_ACCERR on the first store/copy into the
mapping. Fix: at the AAudio call site, pass `/*writeable=*/true` to
`BinderMappedMemory::map` regardless of the flag.

**Other services may follow the same convention** — assume
`writeable=false` is informational rather than load-bearing for binder
shared memory; map `PROT_RW` unless you specifically know the region
is read-only config data.

**#5 — Write-then-start, never start-then-write.** AAudio's mixer
thread spins up immediately on `startStream()`. If the data ring is
empty when start fires, the HAL advances readCounter past 0 before
the producer gets a chance to write; the next `write_pcm_f32` sees
`w − r` wrap to ~u64::MAX and the "ring full" guard rejects every
write. Pre-fill ring with at least one burst's worth before calling
start. AOSP libaaudio's reference code does the same.

**Bonus — the AAudio data plane has NO eventfd.** Pure SPSC ring with
release-acquire ordering on int64 counters. A3's `EventfdSignal`
primitive isn't used by audio_impl; it stays useful for camera +
codec2 follow-ons (both DO use eventfd for buffer-available).
Documented in tasks/21 appendix's "eventfd? — NO" section.

Related: [[rsbinder-aidl-recursive-parcelable-limitation]],
[[project_wasm_runtime]].
