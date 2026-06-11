# Redox OS as a wandr host target — feasibility notes

*Discussion captured 2026-06-11 (post wasi:canvas migration). Companion to
the other "X on wandr" memos (avalonia / flutter / qt). Status: discussion
only — no spike run yet.*

## Why Redox is interesting

Philosophically the best-aligned target of everything we've surveyed:
all-Rust userspace, microkernel, capability-oriented — rhymes with the
WASI capability model wandr's WIT surface is built on. Redox's
everything-is-a-scheme daemon model is a *closer* fit to "arbiter + WIT
services" than Android's binder ever was: `wandr:*` interfaces would be
thin clients of Redox scheme daemons (audio/net/input), exactly the
host-private swap the contract was designed for.

Context that makes this real rather than hypothetical: the wasi:canvas +
wasi:input-handlers migration (2026-06-11) means every non-Kotlin guest
imports only `wasi:*` + the drafts + trimmed platform bits — guests are
already OS-agnostic. The task-101 desktop dev loop proves the same wasm
binaries run on a non-Android host (x86_64 Linux, winit + skia CPU
raster + softbuffer present).

## Part 1 — x86_64 Redox (VM): the host port

The desktop-loop host shape maps onto Redox nearly 1:1.

**Easy (surprisingly): window / present / input.** winit has a
Redox/Orbital backend; so does softbuffer. CPU raster matches Redox's
no-GPU-driver reality, and phone-resolution frames were fine on CPU in
the desktop loop. Rust std exists for `x86_64-unknown-redox` (cross via
redoxer), so the pure-Rust bulk of wandr-host compiles in principle.

**The three real blockers, ranked:**

1. **Skia.** skia-safe = cross-compiling a huge C++ codebase against
   relibc with a Redox clang toolchain. Nobody has ported Skia; slog.
   The escape hatch we built without meaning to: wasi:canvas is a
   CONTRACT, not a skia binding — a second host backend on
   **tiny-skia + parley** (both pure Rust, both Redox-friendly) could
   serve the same WIT. The Slint work already proved host-shaped text
   layout works with a parley-style stack. Valuable beyond Redox: it
   would make the host buildable without the C++ Skia dependency
   anywhere.
2. **wasmtime.** Not an officially supported platform. Cranelift is pure
   Rust (fine); the runtime needs executable memory mapping and (by
   default) signal-based trap handling — Redox's signal support is the
   historically shaky part. Fallback exists:
   `signals_based_traps(false)` (explicit bounds checks, some slowdown).
   JIT on x86_64 in a VM is the realistic execution mode; our AOT
   pipeline doesn't help until wasmtime grows a Redox compile target.
3. **Async runtime.** wasmtime-wasi p2 I/O sits on tokio/mio; mio's
   Redox support is the murkiest dependency in the stack. Might work
   via relibc's POSIX surface, might need the sync wasi variants, might
   need patches.

**The 80% spike:** "does wasmtime compile and run a hello-component
under redoxer?" — one afternoon (`repros/redox-wasmtime-probe` when/if
we do it). If it passes, the remaining work is the pure-Rust renderer
backend.

## Part 2 — arm64 / real phone hardware: the vendor-blob question

"Vendor blobs" are three different things that fail differently on a
non-Linux kernel:

1. **Device firmware (benign).** Modem/Wi-Fi (ath10k) firmware, GPU
   microcode, DSP images — opaque payloads uploaded INTO coprocessors.
   They don't execute on the CPU and don't care what kernel loaded
   them. Redox just needs a `request_firmware` equivalent. Work, not a
   wall.
2. **Userspace driver blobs (the killers).** Adreno EGL/gralloc, camera
   HAL, audio HAL — the things wandr's `--no-art` stack still leans on
   today. Bionic-linked ELFs making Linux syscalls + device-specific
   ioctls (KGSL for GPU). Dead three times over on Redox: wrong libc,
   wrong syscall ABI, wrong kernel driver surface. libhybris
   (SailfishOS / Ubuntu Touch) only works because it keeps the Android
   Linux kernel underneath — the trick does not transfer to a
   microkernel. The only "run the blob" path is a linuxulator-style
   emulation layer PLUS reimplementing KGSL's ioctl surface as Redox
   schemes — years of work to run code you can't debug.
3. **The path everyone actually takes: don't run the blobs.** Qualcomm
   SoCs are uniquely lucky — the open stack is nearly complete
   upstream: **freedreno/turnip** (Adreno GPU, no userspace blob),
   **ath10k** (Wi-Fi), qcom ALSA (audio), **camss** (camera).
   postmarketOS runs mainline Linux on taimen-class hardware this way.
   For Redox it still means PORTING those drivers (as userspace
   daemons — which a microkernel is shaped for), but it's open code +
   category-1 firmware. If targeting hardware: choose Qualcomm, inherit
   the open-driver ecosystem.

**4. The pragmatic path: keep Linux as the blob hotel.** Run Redox in a
VM on the phone — Pixels ship pKVM/AVF (Android Virtualization
Framework); crosvm exists precisely for this. Blobs stay in the Linux
host; Redox sees clean virtio-gpu/input/net devices — simple,
documented interfaces a young OS can drive. The wandr host fits this
unusually well: CPU-raster capable, winit/softbuffer Orbital backends,
and the WIT service layer abstracts whether a daemon speaks binder, a
Redox scheme, or a virtio queue.

## Verdict

- **x86_64 Redox in a VM**: plausible with today's pieces — gated on
  the wasmtime spike, then the tiny-skia+parley host backend.
- **arm64 bare-metal phone Redox**: moonshot, gated on open-driver
  ports (not blobs) — Qualcomm hardware only.
- **Redox-under-AVF on a Pixel with wandr as its app runtime**: the
  realistic convergence. There's a symmetry: wandr already demoted
  Android's Java layer to "boot service we ignore"; the VM path demotes
  the Linux kernel itself to "device firmware with extra steps."
