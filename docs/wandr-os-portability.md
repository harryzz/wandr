# wandr OS portability — how the runtime plugs into existing OSes

> Investigation memo, 2026-06-16. Grounded in the current tree (not aspiration):
> `docs/architecture-runtime.md`, `docs/architecture-host-guest-boundary.md`,
> `runtime/wandr-host/src/*`, `runtime/wandr-arbiter/*`, `runtime/wandr-hal-*`.
> Companion: `docs/redox-wandr-feasibility.md` (the one OS deep-dive already done).

## TL;DR

wandr was built as an Android-ART replacement, but the architecture is **~90%
OS-agnostic Rust** with the OS-specific code quarantined behind `cfg(target_os =
"android")` gates and a handful of `wandr-hal-*` crates. The entire port surface
to a new OS is **three narrow seams**:

1. **Process + IPC model** — `fork()`-COW zygote, three UNIX sockets, POSIX
   signals for role transitions.
2. **Surface acquire + present** — SurfaceFlinger BBQ + EGL on device; winit +
   glutin GL on the desktop dev loop.
3. **HAL capabilities** — `wandr-hal-*` (display power / sensors / net / lights),
   audio (AudioFlinger-direct), and the binder HALs inside `wandr-host`.

Everything else — the guest, the WIT contracts, the wasmtime runtime core, the
event bus, and the arbiter's policy modules — ports for free. The proof this is
real, not theoretical: **the desktop dev loop (task 101) already runs the
identical guest wasm** on x86_64 Linux via the `cfg(not(target_os = "android"))`
path, changing only seam ②. There is already ~1.5 OSes.

## The layering (what is portable vs OS-bound)

```
┌─ Guest (wasm component) ───────────────────── 100% OS-agnostic, never changes
│   Compose / Avalonia / Slint / dioxus / Tetris / Signal …
├─ WIT contracts ────────────────────────────── OS-agnostic BY DESIGN
│   wasi:canvas · wasi:input-handlers · wandr:ui-shell · wandr:{device,chrome,
│   assets,ime,keyguard,events} · wasi:audio · wasi:tls · wasip2 std set
├─ Host runtime CORE (pure Rust) ───────────── OS-agnostic
│   wasmtime Engine/Store/Linker · the render loop (standalone.rs) ·
│   the arbiter event bus + policy modules (wm/shell/keyguard/power/audio/
│   alarm/notify/task-manager) · the per-display surface/role model
└─ OS SEAMS (only three) ────────────────────── the entire port surface
     ① process + IPC      ② surface acquire + present      ③ HAL capabilities
```

The decisive design property is that **policy is mechanically separated from
mechanism**. Arbiter modules decide (`Ctx::emit(Event)` →
`on_event` → returns `Effect`s) and never call the OS; the single place a policy
decision becomes an OS action is `apply_role()` in `wandr-arbiter-bin` (an
`Effect::SetRole` → a POSIX signal). That one function is the seam between
Layer 2 and seam ①, and it is a few lines.

## Seam ① — process + IPC model

The Hybrid runtime is a `fork()`-from-zygote model with three UNIX-domain
sockets and POSIX signals (full protocol in `docs/architecture-runtime.md`):

- **`wandr-zygote.sock`** — arbiter → zygote: `LAUNCH*` (fork) + `PRELOAD` +
  `SUBSCRIBE_EXITS`. The zygote preloads the wasmtime `Engine` + a registry of
  deserialized `.cwasm`s; forked children inherit it via COW.
- **`wandr-arbiter.sock`** — CLI + host children → arbiter: policy commands +
  IME routing.
- **`wandr-host-<pid>.sock`** — arbiter → each child: inbound events (key
  events, editor focus), fire-and-forget.
- **Signals** — `SIGUSR2`=Foreground, `SIGUSR1`=Background, `SIGRTMIN+1`=
  OverlayBehind, `SIGCHLD`=reap, `SIGTERM/INT/HUP`=graceful shutdown.

The protocol is deliberately **text + signals, not binder** — chosen because it
is "Unix cheap" and `cat`/`strace`-debuggable (`architecture-runtime.md` §"Why
this shape"). That bet is exactly what costs portability on non-Unix targets.

| Mechanism | Portability |
|---|---|
| AF_UNIX sockets (text, line-based) | Linux/Android/macOS/*BSD/iOS ✅; Windows ✅ since Win10 1803 but **no abstract namespace** (the `@wandr-inputflinger` form needs a file path or a named pipe); Redox = scheme-based equivalent |
| `fork()` + COW engine inherit | Unix ✅; **Windows: no `fork`** (→ spawn child + re-`deserialize_file`, losing COW but keeping the preload-from-disk win); **iOS: `fork`+`exec` forbidden for apps**; Redox ✅ |
| Signals for role transitions | Unix ✅; **Windows: no POSIX signals**; iOS: exist but the process model collapses anyway |

**The highest-leverage refactor:** fold the role-signals into the per-host
control socket as a message (e.g. a `role <fg|bg|overlay>` line, alongside the
existing `key-event`/`editor-attached` lines). The child already drains that
socket every frame in `ime_inbound`. Doing this **erases the signal dependency**,
making Linux/*BSD/macOS trivial and Windows tractable, and costs nothing on
Android (a socket write replaces a `kill`). This is the single change that most
widens the OS reach.

## Seam ② — surface acquire + present

Skia is cross-platform (GL / Vulkan / Metal / D3D backends), so this seam is
**only the surface acquisition + present**, not the renderer:

- **Device** (`cfg(target_os = "android")`): `libsf_surface.so` C++ shim →
  `SurfaceComposerClient::createSurface` → BLASTBufferQueue + `ANativeWindow` →
  EGL context → swap (`standalone.rs`, `sf_surface.rs`, `cpp/sf_surface.cpp`).
- **Desktop** (`cfg(not(target_os = "android"))`, task 101): winit window →
  glutin GL → skia GL surface → present; softbuffer blit fallback. Same render
  loop, minus the winit-specific resize/input plumbing.

Already two working backends. A third OS implements the same two operations
(get a drawable, present a frame) against its compositor: DRM/KMS or Wayland on
Linux, DXGI swapchain on Windows, CAMetalLayer on macOS/iOS. winit already
abstracts the windowed cases (Linux/Windows/macOS), so the desktop path largely
*is* the windowed port — what is missing is the multi-process shell (below), not
the surface code.

## Seam ③ — HAL capabilities

The genuinely OS-bound code, and it is well-quarantined. Each capability is a
`wandr-hal-*` crate with the identical shape (see `wandr-hal-display/src/lib.rs`):

```rust
#[cfg(target_os = "android")]   → rsbinder → AIDL service   (e.g. ISurfaceComposer.setPowerMode)
#[cfg(not(target_os = "android"))] → no-op stub
```

This is a clean per-capability seam: porting = adding a backend arm, the stub is
already the fallback so an unimplemented capability degrades gracefully (the same
capability-negotiation pattern used guest-side in `wasi:audio`/`wasi:video`).

| Capability | Android (today) | Linux | Windows | macOS |
|---|---|---|---|---|
| display power | `ISurfaceComposer.setPowerMode` (binder) | DRM/KMS | DXGI | CoreGraphics |
| sensors | `ISensorManager` (binder) | iio / evdev | WinRT Sensors | CoreMotion / IOKit |
| net (DNS/routes) | `IDnsResolver` / netd | netlink / resolv.conf | WinSock / WinRT | SystemConfiguration |
| lights / vibrator | `ILights` / `IVibrator` (binder) | sysfs LED | — | — |
| audio | AudioFlinger-direct (`audioclient-rs`) | ALSA / PipeWire | WASAPI | CoreAudio |
| input | wandr-inputflinger (evdev under `--no-art`) | evdev | Raw Input | IOKit HID |

## Per-OS verdict

| OS | Gating piece | Effort shape |
|---|---|---|
| **Android** | — | reference implementation, shipped |
| **Linux** | multi-process shell on the desktop path (the dev loop is single-process / no arbiter — "no arbiter, IME warns, insets 0") + Linux HAL backends | **Closest.** Layers 0–2 already run; ships as a Wayland client or a bare DRM/KMS shell |
| ***BSD** | same as Linux (native fork/sockets/signals; evdev on FreeBSD; sndio/OSS) | ~Linux, least friction |
| **macOS** | winit+Metal work; **`fork` without `exec` is Apple-framework-unsafe** (mitigated — zygote children re-`exec`); macOS HAL backends | Linux-class, with a fork-safety caveat to validate |
| **Windows** | **no `fork`, no POSIX signals** → rewrite seam ① (spawn + socket-message roles); AF_UNIX path-based, no abstract ns | Core + skia + winit all fine; **seam ① is a genuine port** |
| **iOS** | **no `fork`/`exec`, no JIT (W^X enforced)** → collapse to single-process multi-`Store` + the Pulley interpreter (or ship pre-AOT `.cwasm`); it would be an *app*, not an OS replacement | Most divergent — "eventually," and only after the single-process variant exists |
| **Redox** | wasmtime-under-Redox spike + pure-Rust render (tiny-skia + parley, no EGL) + driver story; realistic path = Redox-under-AVF with Linux holding the HAL blobs | Research-grade — see `docs/redox-wandr-feasibility.md` |

## The two real porting axes (in leverage order)

1. **Process / IPC model (seam ①).** `fork()` + signals is the one Unix-ism
   baked deep. The role-signal → socket-message refactor unlocks all non-Android
   Unixes and de-risks Windows. iOS forces single-process regardless (a separate,
   larger variant).
2. **HAL + surface backends (seams ② / ③).** Additive, per-capability, already
   has a clean stub/cfg seam *and* a working second backend (desktop) to copy.

Layers 0–2 (guest, WIT, runtime core, event bus, arbiter policy) require no work.

## Recommended first step (if/when this becomes a task)

Make **Linux a first-class target**, not just a dev loop: stand up the
zygote + arbiter + multi-host shell on Linux (the IPC primitives are all native
there), driving winit/glutin surfaces, with no-op HALs to start. That exercises
seam ① end-to-end on the friendliest OS, produces a real second platform, and
forces the role-via-socket refactor that pays off everywhere else. HAL backends
(DRM/KMS, evdev, PipeWire) then land incrementally behind the existing stub seam.

## Where this is grounded

| Claim | Source |
|---|---|
| 3-socket + 3-signal IPC, fork-COW zygote, "why text not binder" | `docs/architecture-runtime.md` |
| host owns the loop, guest is reactive, WIT canonical-ABI boundary | `docs/architecture-host-guest-boundary.md` |
| desktop dev loop runs the same wasm via `cfg(not(android))` (winit/glutin) | task 101 (`tasks/101-desktop-dev-loop.md`), `runtime/wandr-host/src/standalone.rs`, `main.rs` |
| HAL = `cfg(android)` rsbinder/AIDL + off-android no-op stub | `runtime/wandr-hal-{display,sensors,net,lights}/` |
| event bus = in-arbiter `Ctx::emit`/`on_event`/`Effect`; guest pub/sub = `wandr:events` over the per-host socket | `runtime/wandr-arbiter/wandr-arbiter-core/src/lib.rs`, `project_event_bus` |
| audio backend = AudioFlinger-direct, decoupled `audioclient-rs` | task 98 (`project_audioflinger_backend`) |
| Redox specifics | `docs/redox-wandr-feasibility.md` |
