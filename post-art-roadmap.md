# Post-ART Roadmap

Working document for the long-term direction of WAR (the WASM Android
Runtime). Captures decisions and reasoning from the design discussion that
followed the rsbinder / aidl2wit conversation. Living document — sections
marked **Deferred** or **Open** are not yet committed.

---

## 1. End goal (confirmed)

Replace **ART** in AOSP. Keep:

- The **vendor HAL** layer (C/C++ HAL daemons under `/vendor/bin/hw/`)
- All **native services** (SurfaceFlinger, AudioFlinger, InputFlinger,
  CameraServer, SensorService, hwservicemanager, servicemanager)
- The kernel + drivers

Remove:

- The **Java framework** (system_server's Java services, zygote, ART itself)
- APK / DEX / dexopt
- The Java SDK surface that apps used to consume

Apps run on **WAR's wasmtime-based runtime** instead, as WASM components.

This is a strictly bigger project than the current "WASM app on Android"
PoC and is *not* the work tracked in CLAUDE.md tasks 01–14. Those tasks
remain valid as the foundation; this roadmap describes what comes after.

---

## 2. Architectural framing — two boundaries

The runtime sits between two boundaries that have different design rules:

```
┌────────────────────────────────────┐
│  WASM guest (Compose app, etc.)    │
├────────────────────────────────────┤ ← Boundary A: hand-authored WIT
│  WAR runtime  (Rust + wasmtime)    │
│   - canvas, input, lifecycle       │
│   - window/activity equivalents    │
│   - HAL brokers (rsbinder clients) │
├────────────────────────────────────┤ ← Boundary B: AIDL/HIDL over binder
│  Native HAL daemons + services     │
│  SurfaceFlinger, AudioFlinger, ... │
│  /vendor/bin/hw/*.vibrator, ...    │
├────────────────────────────────────┤
│  Kernel + drivers                  │
└────────────────────────────────────┘
```

The boundaries are not symmetric, and the same tool does not serve both.

**Boundary A (guest ↔ runtime)** is a *portability contract*. Apps must
not be able to tell whether they're running on Android, desktop, or
something else. WIT here is hand-authored in domain terms
(`haptics.perform(feedback)`, not `haptics.vibrate(uid, opPkg, ...)`).
Stays small and platform-neutral.

**Boundary B (runtime ↔ HAL)** is *internal*. It exposes Android-specific
AIDL/HIDL shapes because that's what stable HALs use under Treble/VINTF.
Rust-only — no WIT here. The right tooling is `rsbinder-aidl` generating
Rust client traits from AOSP's HAL AIDL files.

The diagram shows a single runtime process for clarity, but whether the
final architecture is one monolithic runtime hosting many WASM
components or one runtime process per app coordinated by a session
manager is an open design question — see §9.

The two boundaries meet inside per-feature `*_impl.rs` files in the
runtime, where Boundary A's WIT method translates to Boundary B's binder
call(s).

---

## 3. Boundary B: runtime ↔ HAL via rsbinder

### 3.1 Why binder, not sysfs / dlopen libhardware

The current `haptics_impl.rs` writes to `/sys/class/timed_output/vibrator`
or `/sys/class/leds/vibrator`. This fails on stock Android (`EACCES` for
non-`system` UID) and is brittle even on rooted devices because those
paths may not exist on modern kernels.

Direct `dlopen` of `/vendor/lib64/hw/vibrator.*.so` (Pattern 1 from
`~/aidl2wit/rust-stub-cpp-interop-strategy.md`) does not work for an
app-level process — SELinux blocks it, and HAL daemons expect to be the
singular owner. Pattern 1 is correct for *replacing the HAL daemon
itself*, which we are not.

The right path for runtime-to-HAL on the device-as-it-ships is **binder
to the vendor HAL service**, which is registered in either:

- `hwservicemanager` (HIDL HALs, older API levels)
- `servicemanager` (stable AIDL HALs, Android 11+)

This is the boundary the rest of Android already uses, and stable AIDL
HALs are version-stable by VINTF contract.

### 3.2 Pattern 5 — binder-mediated HAL access

(Extends the four patterns in
`~/aidl2wit/rust-stub-cpp-interop-strategy.md` — Pattern 5 is the
dominant case for an app-level WAR.)

```
WIT method body (hand-authored, per feature)
        │
        ▼
rsbinder-generated client trait (rsbinder-aidl from AOSP AIDL)
        │
        ▼
libbinder_ndk on device  →  servicemanager  →  vendor HAL daemon
```

Build-time:

- AIDL source: vendored from
  `hardware/interfaces/<feature>/aidl/android/hardware/<feature>/`
  pinned to a specific AOSP tag
- `build.rs` runs `rsbinder_aidl::Builder` to generate Rust client traits
- Runtime calls `rsbinder::hub::get_service::<dyn IFoo>("foo")` to obtain
  a `Strong<dyn IFoo>` proxy

Runtime requirements:

- `android.permission.VIBRATE` (or analogous) declared in manifest *if*
  we're still packaged as an APK
- Once we boot outside the app framework (see §6), permissions are
  granted by SELinux policy / our service's seclabel, not manifest

### 3.3 Worked example — haptics

To be implemented when this roadmap moves to execution. Sketch:

```
wart-host/
  aidl/android/hardware/vibrator/
    IVibrator.aidl
    IVibratorCallback.aidl
    Effect.aidl
    EffectStrength.aidl
  Cargo.toml                     (+ rsbinder, rsbinder-aidl)
  build.rs                       (+ rsbinder_aidl::Builder calls)
  src/haptics_impl.rs            (rewritten to use binder client; sysfs
                                  retained as last-resort fallback)
```

WIT side stays untouched — `Haptics.perform(Feedback)` is already the
right shape.

### 3.4 Risks specific to Boundary B

- AIDL drift between AOSP versions — pin to the version matching
  `min_sdk_version` (currently 29); add version-gated paths only when
  needed
- `@hide` / `@SystemApi` AIDLs may require platform signature; stick to
  the stable VINTF HALs in `hardware/interfaces/**/aidl/`
- `IBinder` token lifetimes — model them as `resource` on the WIT side
  when they cross to the guest, never as `u32`
- rsbinder transport: confirm it links against `libbinder_ndk.so` on
  Android, not raw `/dev/binder` ioctls (avoids SELinux surprises)

---

## 4. Why we are *not* building aidl2wit

After reading `~/aidl2wit/aidl-hidl-to-wit-analysis.md` and
`~/aidl2wit/rust-stub-cpp-interop-strategy.md`:

The design docs are mechanically sound — type tables, naming
conventions, interface-as-resource mapping, four-pattern C/C++ interop
matrix. But auto-translating AIDL → WIT is the wrong tool for both
boundaries in this project.

**For Boundary A (guest ↔ runtime):** auto-generated WIT leaks
Android-specific concepts (`uid`, `opPkg`, `IBinder token`) into a
contract that's supposed to be portable. The example output
`vibrate(uid: s32, op-pkg: string, ..., token: u32)` is precisely what
the *guest must not see*. The portability target is the abstraction
itself.

**For Boundary B (runtime ↔ HAL):** WIT is not used here at all. The
runtime calls HAL clients directly in Rust. `rsbinder-aidl` already
emits exactly the right artifact — Rust traits over binder.

**Other concrete issues with the proposed aidl2wit output:**

- `IBinder → u32` gives up on the case `resource` exists for. Token
  collisions become a guest concern instead of being modeled.
- "200 interfaces, 500 parcelables" emitted into a single WIT world
  would balloon wit-bindgen output and compound the O(N³) Kotlin/Wasm
  IR-lowering pain we already hit on compose-multiplatform-core.
- `@hide` / `@SystemApi` filtering isn't designed in; most of
  `frameworks/base/core/java/android/**/I*.aidl` is hidden and unsafe.
- `oneway → sync WIT func` reintroduces blocking semantics that didn't
  exist before. Wait for async WIT (Component Model async).
- Pattern 1 (bindgen on `libhardware`) shown for vibrator is the
  *wrong* path for an app-level runtime — should be Pattern 5 (binder)
  for the cases we care about.

**Decision:** drop aidl2wit. Keep the type-mapping rules from the design
doc as reference for hand-authoring WIT. Use `rsbinder-aidl` unchanged
for HAL bindings.

**Possible smaller tool worth revisiting later:** an `aidl-rsbinder
bridge generator` that takes one paired (WIT method, AIDL method)
declaration and emits the parcelable conversion glue. Defer until 3–4
features have been hand-written and the repetition is concrete.

---

## 5. Display path — ISurfaceComposer

**Decision: do not migrate yet.** Pair the migration with the boot-model
work (§6.1).

Current setup uses NativeActivity → `ANativeWindow*` → EGL → Skia GPU.
This implicitly goes through SurfaceFlinger. It works.

Switching surface allocation to `ISurfaceComposer` directly (via
rsbinder) inside the current APK process gains nothing today: privileged
SF methods (`createSurface`, `createDisplay`, `setActiveConfig`) require
`ACCESS_SURFACE_FLINGER`, which a normal app UID does not hold.
Escalating via `su` defeats the experiment.

The migration is necessary only when we boot outside the app framework
— then there's no NativeActivity to hand us a surface.

**De-risking step that costs nothing:** prototype a read-only
`ISurfaceComposer.getDisplayInfo` round-trip via rsbinder from inside
the current process. Validates the binder transport works against SF
without changing the render path. Can be done now if useful.

### 5.1 Considered & rejected — running on a Wayland compositor

An alternative to being a SurfaceFlinger client: bring a Wayland
compositor (weston / custom) onto the device and make the runtime a
Wayland client. **Rejected for the Android target.**

- Android has no Wayland. A compositor brought onto the device would
  either (a) nest inside a SurfaceFlinger layer — pointless, SF still
  underneath plus an extra composition hop — or (b) replace SF and
  drive the Hardware Composer HAL / DRM-KMS itself, which means
  re-implementing SF's vsync + HWC + vendor-display integration: a
  roadmap "Keep" (§6.1) turned into a major, vendor-specific build.
- SurfaceFlinger already *is* the compositor. The runtime needs
  exactly one fullscreen layer; SF provides that trivially via
  `ISurfaceComposer` (§5).
- The portability appeal of Wayland (clean protocol, desktop reuse) is
  already handled one layer up by `winit`, which abstracts
  `ANativeWindow` (Android) vs X11/Wayland (Linux). Desktop builds
  already run on Wayland via winit — no need to make Android speak it.

Wayland / direct DRM-KMS is the right display path **only** for a
future *bare-Linux / embedded* target with no SurfaceFlinger — the
same fork as §9's "display server" open question. For Android
hardware (keep the native daemons + HALs): SurfaceFlinger client.

---

## 6. system_server: replace / drop / keep

system_server hosts ~80 services. The bulk are policy layers over
native daemons that we keep running. WAR-relevant bucketing:

### 6.1 Keep (already native daemons, talk via binder)

| Service | Used for |
|---------|----------|
| SurfaceFlinger | Display composition (allocate Surface via `ISurfaceComposer`) |
| AudioFlinger + AudioPolicy | Audio output and routing |
| InputFlinger | Input device reading + dispatch (consume input channel) |
| SensorService | Sensor multiplexing |
| CameraServer | Camera HAL fronting |
| hwservicemanager / servicemanager | Binder name lookup |
| Vendor HAL daemons (`vendor/bin/hw/*`) | Vibrator, lights, fingerprint, NFC, ... |

### 6.2 Replace in WAR (small surface, mostly trivial)

| Replacement | Notes |
|-------------|-------|
| WindowManager | One WIT-allocated window per WASM component; fullscreen for PoC. No multi-app arbitration logic. |
| ActivityManager | WASM-component lifecycle = `instantiate → entry → suspend → resume → drop`. The existing `LocalLifecycleOwner` bridge is ~80% of this. |
| Input dispatch | Pull from InputFlinger via input channel pipe, route to focused component's WIT pointer/key handlers. |
| AudioFocus | ~50 lines of in-runtime arbitration + WIT `acquire-focus` / `release-focus`. |
| PowerManager (kernel parts) | Write directly to `/sys/power/wake_lock` and `/sys/power/state`. |

### 6.3 Drop entirely (don't apply to WAR's app model)

- PackageManager — see §7, replaced by a component graph loader
- StatusBar / SystemUI / WallpaperManager — runtime draws everything
- NotificationManager — replaced by WIT `post-notification` into a
  runtime-owned compositor row
- AlarmManager / JobScheduler — runtime's own scheduler
- StorageManager / ContentService / AccountManager / DPM / UserManager
  — out of scope
- Telephony stack — not a phone replacement

### 6.4 Defer to later milestones

- Wifi / Connectivity — talk to `wpa_supplicant` socket directly
- LocationManager — bind to GNSS HAL via rsbinder
- BluetoothManager — bind to bluetooth HAL daemon

### 6.5 Key insight

system_server was complex because it managed multi-app arbitration
**plus** multi-user, MDM, permissions over 200+ classes. WAR needs the
arbitration — multiple WASM apps will coexist and must coordinate on
who has the foreground window, audio focus, vibrator, camera,
wakelocks. What WAR does *not* need is the policy bloat: multi-user,
MDM, telephony, account framework, 200+ permission classes.

So the simplification is real but smaller than "drop everything."
What's left after dropping the irrelevant 70–80% is:
- ~10 binder client crates from `rsbinder-aidl` for vendor HALs
- A focused arbiter for foreground / audio / vibrator / camera /
  wakelocks — design depends on §9 runtime-model question
- WindowManager / ActivityManager / Input dispatch equivalents per
  §6.2, scoped to "many WASM components, one user, no MDM"

The arbiter's *location* (in-process in a monolithic runtime, or a
separate session-manager process coordinating per-app runtimes) is the
open question in §9.

---

## 7. PackageManager replacement — component graph loader

**Forward-compatibility constraint:** The WASM Component Model is
moving toward *multi-component packages* — a "package" holds multiple
components plus an instruction file (likely `wac`) describing how to
link them. Single-`.cwasm`-per-app is fine for today's PoC but should
not be baked into the runtime's loader interface.

### 7.1 Package shape (target)

```
<app-id>-<version>/
  package.toml                  ← metadata + entry + declared world
  link.wac                      ← composition script (or inline)
  components/
    ui.wasm
    logic.wasm
    persist.wasm                ← optional
  assets/
    fonts/, images/
  ui.cwasm   logic.cwasm   persist.cwasm   ← per-component AOT cache
```

### 7.2 Loader job (what replaces PackageManagerService)

1. Read `package.toml`; verify declared world is satisfiable by the
   runtime's offered worlds
2. Resolve `link.wac` to a concrete component graph (which exports
   satisfy which imports, including host-provided WIT)
3. AOT-compile each component separately (per-component cache survives
   partial updates)
4. Instantiate the graph; hand the entry component to the lifecycle
   manager

### 7.3 Why this is nicer than APK semantics

- **Permissions = imports.** A component's WIT imports literally are
  its capability requests; the runtime grants or refuses by
  providing/withholding the host impl. No XML `<uses-permission>` list
  to keep in sync with code.
- **Updates are per-component.** Re-AOT 5 MB instead of 60 MB.
  Impossible with APK / DEX.
- **Linking is declarative.** Auditable separately from the components.
  Signing the linking decisions is meaningful (an APK's behavior is
  determined by code that signatures can't summarize).

### 7.4 Current ecosystem state (Jan 2026)

- `wac` (composition language) — usable
- `wkg` / Warg (registry + package transport over OCI) — pre-1.0
- True runtime dynamic linking (lazy load on demand) — not stable in
  wasmtime; what *is* stable is build-time-style composition done at
  load time, which covers "install an app, run it"
- Cross-component resource/handle delegation — works, rough edges

### 7.5 Minimum manifest (subject to revision)

```toml
[package]
name    = "com.example.demo"
version = "1.2.0"
entry   = "ui"
world   = "war:app/main@1.0.0"

[components]
ui     = { path = "components/ui.wasm",     aot = "ui.cwasm" }
logic  = { path = "components/logic.wasm",  aot = "logic.cwasm" }

[link]
script = "link.wac"

[assets]
dir    = "assets"
```

### 7.6 Roadmap implication

Write `app-loader.rs` *now* with this interface even though today's
implementation only handles a single `.cwasm`. Keep the boundary
between loader, lifecycle, and window-manager forward-compatible with
multi-component packages without committing to wac/Warg specifics.

---

## 8. Deferred

| Topic | Reason |
|-------|--------|
| **Boot model** — run WAR as init.rc service (PID 1 child) instead of an APK | Not until current PoC is done. Pairs with §5 (ISurfaceComposer migration). |
| **Multi-app *implementation*** — actual support for >1 concurrent WASM app | Required eventually; single-app is enough for current PoC. Architectural shape (one runtime vs many) is open — see §9. |

---

## 9. Open questions (still to decide)

- **Runtime model — monolithic vs process-per-app.** Multi-app
  arbitration is required; the question is *where* the wasmtime
  instances live and *where* the arbiter runs.

  | Option | Shape | Pros | Cons |
  |--------|-------|------|------|
  | **Monolithic** | One WAR process. One `wasmtime::Engine`, many `Component` instances. Arbiter is in-process Rust. | Lowest memory (shared wasmtime/skia/fonts/AOT cache). In-process arbitration is trivial. Fewer binder hops to shared services. | OS-level isolation is per-runtime, not per-app — one host bug brings them all down. Per-app memory accounting must be tracked in-runtime (OS can't see it). Cross-thread GPU/EGL sharing needs care. |
  | **Process-per-app** | One WAR process per WASM app. Tiny **session-manager** process owns SurfaceFlinger composition policy, input routing, audio focus, vibrator/camera ownership. Arbitration over binder between session-mgr and app runtimes. | Strong OS isolation; oom-killer can target individual apps; per-app SELinux labels; mirrors Android's existing process model. | Per-app startup cost (wasmtime warmup, AOT load, skia init — seconds, not ms). Each runtime duplicates ~50+ MB of host code/data. Arbiter must use IPC for everything. |
  | **Hybrid** | Session-manager forks per-app runtime processes but shares heavy assets via shm and a shared font/atlas server. Process boundary per app, shared caches. | Compromise on memory and isolation. | Most complex to build. May not be worth it if process-per-app's overhead turns out tolerable on target hardware. |

  WASM linear memory is isolated *within* a single wasmtime by design,
  so "monolithic" does not mean apps can read each other's memory. The
  isolation argument for process-per-app is about host bugs, GPU/EGL
  contexts, and OS-level resource accounting — not WASM sandbox
  integrity.

  *Decision blocker:* need real numbers — measure cold-start time of a
  second wasmtime + skia + AOT load on the Pixel 2 XL, and per-app
  steady-state memory. If second-instance startup is <500 ms and per-app
  overhead is <20 MB, process-per-app is viable. Otherwise monolithic
  or hybrid.

  *Current lean (2026-05-20):* **monolithic-first** for the PoC and
  boot-model bring-up — it is where the runtime already is, the memory
  math favours it (a 2nd wasmtime+skia+AOT instance ≈ ~95 MB AOT image
  + ~50 MB host code/data duplicated — well past the viability
  thresholds above), and the in-process arbiter is trivial.

  **Caveat — monolithic is a single point of failure.** A host-side
  crash takes down the entire app + arbiter layer at once; only the
  kernel and the native daemons (SurfaceFlinger, AudioFlinger, …)
  survive. That is strictly *worse* isolation than stock Android,
  where each app is its own OS-isolated process and even a
  `system_server` crash is a recoverable soft-reboot, not a
  kernel-level failure.

  So the production target is most likely **Hybrid** — and Hybrid is
  essentially *how Android already organises this*: `zygote` preloads
  the ART runtime + core framework **once**, and every app process is
  `fork()`-ed from zygote, sharing the preloaded runtime
  copy-on-write while still being a separate OS-isolated process. The
  WAR Hybrid = a "zygote-equivalent": preload `wasmtime::Engine` +
  shared AOT / skia / font caches, `fork()` a process per app,
  COW-share the heavy read-only pages. *Caveat:* `fork()` after
  wasmtime worker threads or EGL/GPU init is unsafe — a WAR zygote
  must fork *before* those, exactly as Android's zygote forks before
  starting most threads.

  **Therefore:** build monolithic now, but keep the arbiter and the
  app-loader behind a boundary that does not bake in in-process
  assumptions, so the Hybrid (zygote-style) migration stays cheap.

- **Display server**: keep SurfaceFlinger and bind via `ISurfaceComposer`,
  or replace SF entirely with a runtime-owned KMS/DRM compositor?
  (Currently leaning: keep SF — it's already a native daemon, no Java
  involvement, well-understood binder interface.)

- **Input source**: read `/dev/input/event*` directly with `EVIOCGRAB`,
  or consume from InputFlinger's input channel? (Leaning: InputFlinger
  — it already handles touch processing, key remapping, gesture
  detection. Reading raw is reinventing it.)

- **What we do with apps that wanted Java framework APIs**: only the
  ones we explicitly port via WIT will work. There's no
  bytecode-translation layer. Out-of-scope apps simply won't run.

- **Per-component capability gating**: how to express "this component
  may not access haptics even though the package as a whole declares
  it." Likely solved by per-component `link.wac` import restriction.

### 9.1 Reference — how Android/ART organises this (comparison baseline)

The baseline the runtime-model decision above is measured against.

Boot chain:

```
kernel → init → zygote ──fork──► app process 1
                  │     ──fork──► app process 2  ...
                  └─────fork──► system_server
       init also starts the native daemons directly —
       surfaceflinger, audioserver, cameraserver,
       servicemanager, vendor HAL daemons — NOT via zygote, NOT ART.
```

- **zygote** preloads the ART runtime + core Java framework classes +
  common resources **once**. Every app process is `fork()`-ed from
  zygote, inheriting that preload **copy-on-write** — so an app
  process is cheap to start and cheap in memory, yet is a separate OS
  process with its own UID and SELinux domain. That is
  process-per-app isolation *without* the per-app cost of re-loading
  the runtime — i.e. exactly the "Hybrid" shape above.
- **system_server** is also forked from zygote; hosts ~80 services in
  one process.
- **Native daemons** (surfaceflinger, audioserver, …) are plain
  native processes started by init — no ART, not forked from zygote.

Failure domains:

| Crashes | Blast radius | Recovery |
|---|---|---|
| An app | that app only | app restarts; rest of system untouched |
| `system_server` | the Java framework layer | **soft reboot** — zygote + system_server restart; kernel + native daemons survive; ~seconds |
| Kernel | everything | full reboot |

Key point for WAR: Android is deliberately **not** a single point of
failure for apps. A monolithic WAR runtime collapses "system_server +
every app" into one process — a host crash drops the whole soft layer
(only kernel + native daemons survive). The Hybrid model (the WAR
"zygote") is how to recover Android's failure-domain properties.

---

## 10. Reference: current PoC state (do not regress)

This roadmap describes work that comes *after* the current PoC. The PoC
itself is described in CLAUDE.md and is working on the Pixel 2 XL as of
2026-05-15:

- Tasks 01–14 all complete
- Compose Multiplatform UIs render at ~10–20 ms/frame
- TextField + TextFieldState, Material3 widgets, LazyColumn,
  scrolling, soft keyboard, lifecycle resume — all verified
- One known issue: indeterminate ProgressIndicator leaks ~0.4 MB/s
  (Kotlin/Wasm continuation retention)

The PoC runs as a normal APK with NativeActivity on a rooted device.
Boundary B work (rsbinder for haptics is the first real instance) is
the first change to that model. Boundary A WIT remains stable.

---

## 11. Next step

Re-baselined 2026-05-20.

**Done since this doc was first written:**
- ~~Smallest credible Boundary B — haptics via rsbinder + VibratorHAL
  AIDL~~ — done, task 16 (device-verified). Sensors / power / thermal /
  lights / audio HALs followed in tasks 17–21.
- ~~De-risking step for §5 — `ISurfaceComposer` round-trip~~ — done,
  task 22 (SurfaceFlinger reachable, binder transport validated).

**Recommended next — boot-model bring-up:**

1. **Write the boot-model sub-roadmap** (§6.1 + §5) before executing
   anything: how the runtime launches as a privileged process
   (`init.rc` service vs `su`-run binary), surface acquisition, input
   acquisition, what gets `stop`-ed (SystemUI), and the recovery
   story. Turns the post-ART arc into ordered, verifiable steps.
2. **Standalone-surface spike** — step 1 of that sub-roadmap and the
   keystone de-risk. Today the host only runs as a `NativeActivity`.
   The spike: run it as a plain privileged process (no Activity),
   create a fullscreen `SurfaceControl` directly from SurfaceFlinger
   via `SurfaceComposerClient` (the libgui shim, §5), EGL-render one
   frame. Proves the entire post-ART display path; if it fails, it is
   the cheapest possible place to find out.

**Still open, lower urgency:**
- **Runtime-model spike (§9)** — measure 2nd-instance cold-start +
  per-app memory. §9 now leans monolithic-first *regardless*, so this
  measures *when* Hybrid becomes worth it, not a blocker.
- **Forward-compatible loader skeleton (§7.6)** — introduce the
  `app-loader.rs` interface even though it only handles single-`.cwasm`
  today.

---

## 12. Memory subsystem — fallback paths if wasmtime DRC stays MVP

Added 2026-05-18 after filing
[bytecodealliance/wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403)
(per-GC sweep cost grows unbounded on steady-state Kotlin/Wasm
workload). Documented here in case the upstream tracing-collector
work doesn't ship on a timeline that aligns with whatever we want
to do next.

### 12.1 Why wasmtime DRC is the wrong shape for an Android UI app

The current wasmtime GC implementation is a deferred ref-counting
(DRC) collector. From the file header comments:
*"Warning: this ref-counting collector does not have a tracing cycle
collector. This is not a moving collector; it doesn't have a nursery
or do any compaction."* It is explicitly an MVP per the
[wasm-gc RFC](https://github.com/bytecodealliance/rfcs/blob/main/accepted/wasm-gc.md).

For our workload (Compose-Multiplatform with continuous animation
allocating SafeContinuations at ~15K refs/sec), DRC produces a
super-linear cost growth in sweep duration — measured trajectory
from a 45-min soak: 478 ms → 1248 ms → 3000 ms across 3 sweeps,
with `N` (over-approximation-list size) going 1.2M → 2.3M → 4.6M.
On the Android main thread this hits the 5 s input-dispatch ANR
threshold within ~10–50 min depending on interaction load.

We have local mitigations (task 26: `Store` on a worker thread,
periodic `Store::gc(None)` every 5 s via `profiling::
check_and_run_deferred_gc`). These eliminate the ANR but the *Compose
recompose pipeline* still degrades over a session — tap-to-display
latency grows to several seconds because Compose's live retained
state (composition tree, snapshot subscribers, `LaunchedEffect`
jobs) accumulates and gc only reclaims dead refs, not live state.

### 12.2 What Android Runtime (ART) gets right that wasmtime DRC doesn't

ART's GC (Concurrent Copying, post-Android-8) is the reference design
for our use case:

| Property | ART | Wasmtime DRC |
|---|---|---|
| Generational (eden / old) | ✅ | ❌ (single heap) |
| Concurrent — mutator runs alongside sweep | ✅ | ❌ (STW per-store) |
| Compacting / defrag | ✅ | ❌ (free-list fragments) |
| Self-scheduling on alloc rate + idle | ✅ | ❌ (embedder must trigger) |
| Bump-pointer allocation in young gen | ✅ | ❌ (`first_fit` O(F)) |

Every pathology of our 5–6 s tap latency maps directly to one of
ART's solved problems. Specifically: most of our SafeContinuations
are eden-shaped (allocated per frame, dead by next frame); a
generational design would keep per-gc cost O(young-set) instead
of O(total-live-set).

### 12.3 Why we can't simply use ART's GC

ART runs Dex bytecode. We run WASM. Different runtime, different
ABI, different stack-map layout. Bridging is not a small project —
it's "rewrite the runtime."

### 12.4 Fallback paths if upstream wasmtime is stuck

Ordered by reversibility / cost:

1. **Wait (default).** Bytecode-Alliance's tracing-collector RFC is
   accepted; implementation pace unknown. Estimated 6–18 months
   for a first cut. The worker-thread + periodic-gc band-aids let
   the POC limp along in the meantime.

2. **Compile to Android-native via Kotlin/JVM (least dramatic
   refactor).** Drop the WASM layer on Android only. wart-app
   targets `androidTarget()` instead of `wasmWasi`, runs directly
   on ART. Lose: cross-platform-via-WIT story, security sandbox,
   the whole reason for the PoC. Gain: sub-frame tap latency, no
   GC tuning. Costs a few days of build-config work.

3. **Embed a real GC runtime alongside wasmtime** (e.g. V8 just
   for the GC heap, wasmtime for execution). Crazy plumbing —
   would need to bridge `VMGcRef` semantics across two runtimes.
   Not realistic.

4. **Switch to JCO + Node-on-Android.** V8 has the GC we want.
   But Node on Android is rough; JCO is server-shaped not
   embedded-shaped. Would also require swapping out our entire
   host architecture.

5. **Write a tracing GC for wasmtime ourselves and upstream it.**
   3–6 person-months of GC-engineer time. Catherine West's
   [`gc-arena`](https://github.com/kyren/gc-arena) is reference-
   only (its safety relies on Rust's borrow checker, which
   JIT-compiled WASM doesn't participate in); the algorithmic core
   would have to be hand-rolled against `GcRuntime` trait. Real
   but expensive.

### 12.5 Decision

Adopt **(1)** as the active plan. Hold **(2)** as a documented
contingency — if waiting becomes untenable, the Android-only
JVM-target compile is the least dramatic exit. Keep ART's design as
the reference we'd point at if anyone (us or upstream) ever writes
**(5)**. Do not pursue **(3)** or **(4)** without a much stronger
forcing function.

**Update 2026-05-20 — re-prioritised; task-26 status corrected.**

- Task 26 (`Store` on a worker thread + periodic `Store::gc`) was
  **attempted and reverted** — it removed the ANR but introduced
  worse input-lag accumulation (5–6 s after minutes), a net
  regression. So there is currently **no DRC mitigation in the
  deployed host**; DRC runs stock (sweeps only on a `memory.grow`
  failure). The §12.1 "local mitigations" note and the earlier
  "sufficient for the POC" line both pre-date that revert and no
  longer hold.
- The §9 **monolithic-first** decision moves DRC onto the critical
  path. A monolithic runtime shares one `Store` / GC heap across all
  apps, so one app's sweep stall freezes *every* app — shared fate.
  DRC must therefore get a real answer (a §12.4 fallback, or upstream
  #13403) **before** running more than one app concurrently — it is
  no longer a "someday" item. The single-app PoC is unaffected and
  remains demo-usable.
