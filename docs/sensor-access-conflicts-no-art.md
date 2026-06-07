# Sensor access under `--no-art` — paths, Android cross-check, conflicts

> Living tracker. Conflicts are marked **OPEN** / **RESOLVED** as they're fixed.
> Scope: how the wart stack reaches hardware sensors with the Java framework
> stopped, lined up against every standard Android way to reach sensors, to find
> where paths collide. Related: tasks 77 (arbiter SensorService), 85/86
> (orientation/brightness), 93/94 (camera + AIDL ISensorManager), 95 (gyro race).

## The access map (`--no-art`)

Everything funnels through the **one** `/system/bin/sensorservice`, the **sole**
opener of the single-client sensors HAL (device-verified via `lshal`:
`android.hardware.sensors@1.0::ISensors/default` is served only by the HAL service,
with **one** client = sensorservice). The old direct-HAL opener `wart-sensors` is
deleted, so the task-94 `DEAD_OBJECT` contention is genuinely gone.

```
                   sensors HAL @1.0 (single-client)  ← only sensorservice opens it
                              │
                       /system/bin/sensorservice  (the multiplexer + data source)
                              │
                wart-sensormanager (one process, two façades)
       ┌──────────────────────┴───────────────────────────────┐
  HIDL ISensorManager (instance #1)         AIDL ISensorManager (instance #2)
  getInstanceForPackage("…@1.0::…")         getInstanceForPackage("…ISensorManager@1")
       │                                          │
  createDirectChannel — NO poll thread       createEventQueue — ONE shared looper +
       │                                       ONE SCHED_FIFO prio-10 poll thread,
  qcom CAMERA HAL / EIS gyro                  synchronous onEvent binder PER event
  (via HIDL)                                  ┌────────────┴──────────────┐
                                       arbiter sensor_driver        each wart-host
                                       (task 77/94: orientation       (sensors WIT;
                                        always-on @5Hz, on-demand      task-43 orient
                                        proximity/light)               — RETIRED, see C1)
```

Both façades delegate to `::android::SensorManager::getInstanceForPackage()` (the
sensorservice binder), so wart-sensormanager is *not* a HAL owner — it requires the
standalone sensorservice running.

## Cross-check vs the standard Android ways to reach sensors

| Android mechanism | Under `--no-art` | Used by wart? |
|---|---|---|
| Java `SensorManager` (app) | **dead** (no framework) | no — guests use the `sensors` WIT instead |
| NDK `ASensorManager` (`android/sensor.h`) | works if sensorservice up | not used |
| libsensor `android::SensorManager` (native) | works | yes — inside both wart-sensormanager façades |
| frameworks `ISensorManager` **HIDL** @1.0 | needs a publisher (system_server gone) | yes — wart-sensormanager publishes it; camera EIS consumes |
| frameworks `ISensorManager` **AIDL** | needs a publisher | yes — wart-sensormanager publishes; arbiter + hosts consume via `wart-hal-sensors` |
| sensors HAL direct (`ISensors` HIDL/AIDL) | single-client | **only** sensorservice (✓ `wart-sensors` removed) |
| direct report channel (ashmem/gralloc) | works | camera EIS (HIDL direct channel) |

No second direct-HAL client exists; the single-client invariant holds.

## Conflicts

### C1 — Duplicate orientation pipelines (host vs arbiter) — ✅ RESOLVED (device-verified 2026-06-06)
**Was:** task 94 lit up the arbiter reading device-orientation itself
(`sensor_driver.rs` always-on type 27 → WM → `geometry` push), but the host's
task-43 path was left in place: `wart-host/src/standalone.rs` still enabled
device-orientation, polled it every frame, and pushed `report-orientation` up.
Result — **two clients enabled handle 0x17** (device-confirmed `active-count = 2`:
host pid + arbiter pid) and **two independent pipelines drove the same WM rotation**,
contradicting task 94's "one Rust/AIDL sensor path" goal.

**Fix (2026-06-06):** retired the host's sensor *read/report*; the host is now a pure
applier of the arbiter's pushed orient (`authoritative_orient` ← `geometry` line).
Verified the arbiter-side chain is complete without the host report:
`Event::SensorReading{DeviceOrientation}` (WM `lib.rs:544`) → `apply_device_rotation`
→ `Event::OrientationChanged` → `push_system_orientation` delivers `geometry … <orient>`
to the foreground editor/app + fans overlays. Removed from `wart-host/src`:
`device_rotation_to_orient`, `report_orientation_to_arbiter`, the `orient_sensor`
enable, the per-frame poll/report block, `last_dev_orient` / `awaiting_orient_since`
/ `ARBITER_ORIENT_TIMEOUT`, and the `sensors_impl` helpers
(`device_orientation_handle`/`enable_sensor`/`poll_device_rotation`/
`SENSOR_TYPE_DEVICE_ORIENTATION`). The orientation *lock* report
(`set-orientation-lock`, a manifest policy the host legitimately owns) stays.
`cargo build --target aarch64-linux-android --release` of wart-host passes.

**Device-verified (2026-06-06)** after a fresh `--restore-art` → `--no-art` bringup
with the new host:
- `dumpsys sensorservice` → handle 0x17 (Device Orientation) `active-count = 1`, with
  the *only* connection being `aidl_client_pid_<arbiter>` (was `active-count = 2`:
  arbiter + host). The foreground host has no orientation connection — confirms it no
  longer reads the sensor.
- Apply path: `wart-arbiter report-orientation 1` → arbiter decided `orient=4` and
  pushed `geometry … orient=4`; logcat shows every surface applying it purely from the
  push — fullscreen app `renderer: orientation → orient 4, logical 2880x1156 (physical
  1440x2880 unchanged)`, plus statusbar/taskbar/IME overlay-rect flips to the side.
  Toggling 0→1 re-applies cleanly.
- The physical-sensor path is wired (the arbiter holds the live 0x17 connection at 5 Hz);
  visual confirmation on a physical rotate is a user check.

Note: the bringup hit a transient `sensorservice` HAL-claim `DEAD_OBJECT` ×3 (the AIDL
endpoint registered on a later retry) — a single-client-HAL handoff flake after the ART
restart, related to C4; self-recovered.

### C2 — Shared poll thread escalated to SCHED_FIFO (whole-system freeze) — ✅ RESOLVED (device-verified 2026-06-07)
**Was:** `SensorManagerAidl::getLooper` (vendored `aidl/SensorManager.cpp`): *"One
global looper for all event queues"* — a single thread that does
`sched_setscheduler(SCHED_FIFO, prio 10)` and makes a **synchronous `onEvent` binder
call per event to each client**. The **RT priority** was the dangerous part: when that
thread ran hot or busy-looped (C3) it *preempted* normal-priority render/input/adbd on
its core → "100% pins a core" + frozen UI (device-confirmed the spinner was tid
`binder:<pid>_N`, `rt_priority 10`, `policy 1`, `epoll_wait` stack — the poll thread,
mis-named because it's spawned from the `createEventQueue` binder worker).

**Fix (2026-06-07):** we don't need RT here — orientation/proximity/light are low-rate
and the camera EIS uses the HIDL *direct channel* (no poll thread), so the poll
thread's scheduling can't affect gyro timing. `wart-sensormanager` now **drops
`CAP_SYS_NICE` at the top of `main()`** (raw `capset` syscall, no libcap/Android.bp
change → ninja-only rebuild). Linux caps are per-thread and inherited by threads
created later, so every event-queue poll thread (spawned lazily by a binder worker)
inherits the restricted creds; its `sched_setscheduler(SCHED_FIFO)` returns EPERM (the
lib logs a warning and continues), and it runs **SCHED_OTHER** — a spin/hot loop now
time-slices fairly and degrades instead of starving the box.
File: `runtime/wart-sensormanager/cpp/wart_sensormanager.cpp` (`drop_rt_scheduling_capability`).

**Device-verified (2026-06-07)** after a fresh `--no-art` bringup with the rebuilt
binary: logcat `wart-sensormanager: dropped CAP_SYS_NICE …` + `E AidlSensorManager:
Could not use SCHED_FIFO for looper thread: Operation not permitted`; the poll thread
(`binder:<pid>_N`) now reports `/proc/<tid>/stat` `rt_priority 0`, `policy 0`
(SCHED_OTHER) — was `10`/`1` (SCHED_FIFO). Every wart-sensormanager thread is policy 0.

**Residual (NOT a freeze):** the shared-thread head-of-line coupling remains — one slow
client's synchronous `onEvent` still delays delivery to the others. Now benign (sensor
latency, not a system freeze). A fuller fix (oneway `onEvent` / per-client draining)
would need a platform-lib patch; deferred. The latent busy-spin trigger itself is C3.

### C3 — Latent HANGUP busy-spin × non-reconnecting client cache — 🟡 cache half RESOLVED (device-verified 2026-06-07); server spin = 🔴 KNOWN ISSUE / WON'T-FIX (reproduced live 2026-06-07, benign via C2)
Two coupled problems:
- **Non-reconnecting client cache:** `wart-hal-sensors` cached the resolved
  `ISensorManager` + event queue on success and **never rebuilt** them, so after a
  `wart-sensormanager` restart the arbiter/host latched a dead handle → sensors
  silently dead forever (until a full stack re-bringup).
- **Server busy-spin:** the AIDL `EventQueueLooperCallback::handleEvent`
  (`aidl/EventQueue.cpp`) has **no `ALOOPER_EVENT_HANGUP`/`read()<0` guard** (unlike
  `SensorEventQueue::waitForEvent`, `libs/sensor/SensorEventQueue.cpp:134`), so a dead
  BitTube fd stays level-triggered readable and the poll thread busy-loops.

**Fixed (2026-06-07) — cache half:** `wart-hal-sensors` now `ping_binder`s the cached
`ISensorManager` and event-queue handles (the established audio_impl.rs pattern) and,
on death, re-resolves the service + recreates the queue, **replaying the desired
enabled-sensor set** onto the fresh queue (tracked in `enabled_sensors()`). New
`ensure_connected()` is called each tick by the arbiter sensor-driver
(`sensor_driver.rs`) so recovery happens with no re-bringup. Files:
`runtime/wart-hal-sensors/src/lib.rs`, `runtime/wart-arbiter/wart-arbiter-bin/src/sensor_driver.rs`.

Recreating the client queue also drops the stale server-side `EventQueue` (its dtor
`removeFd`s the hung fd from the shared looper), so this **also stops the server spin
for the wart-sensormanager-restart case**.

**Device-verified (2026-06-07):** killed `wart-sensormanager` under a live arbiter →
logcat `event queue dead — recreating + replaying enables` / `ISensorManager handle
dead … re-resolving` → after the new instance registered, `event queue created
(replayed 1 enables)`; `dumpsys sensorservice` showed the arbiter's Device Orientation
connection back, `active-count = 1` — all without a stack re-bringup.

**Residual — 🔴 KNOWN ISSUE (WON'T-FIX for now, user decision 2026-06-07):** whenever a
client's BitTube to wart-sensormanager hangs up while its server-side `EventQueue` is
still alive, the unguarded `handleEvent` busy-loops the poll thread at ~100% of one
core. Triggers: a `sensorservice` restart (wart-sensormanager's *internal* BitTube to
sensorservice hangs up), or any orphaned client connection (see the repro below). It
is **benign** — C2 made the poll thread SCHED_OTHER, so it wastes one core's *slice*
(plus battery/heat) but never freezes the box — so we are **leaving it documented and
unfixed** rather than shipping a patched platform lib.

**Reproduced live (2026-06-07).** A **duplicate** `wart-sensormanager` was started
(manual sensor recovery launched a 2nd instance without killing the 1st). The new
instance's `AServiceManager_addService` **replaced** the older one's
`ISensorManager/default` registration → the arbiter's event-queue connection into the
*older* instance was torn down, but that instance's server-side `EventQueue` stayed
alive → its poll thread (`binder:<pid>_N`, the createEventQueue binder worker's name)
spun at 102%, `wchan=0` (running, not blocked in `pollAll`), **PR 20 / NI 0 =
SCHED_OTHER** (C2 confirmed holding). **Recovery:** `kill -9` the orphan instance →
spin gone, AIDL still registered by the survivor, arbiter still consuming. So the
earlier "never reproduced" note is superseded — the mechanism is confirmed exactly as
read from `aidl/EventQueue.cpp`.

**Operational guards (so this doesn't recur from us):** never run two
`wart-sensormanager` at once; the `--no-art` bringup already `pkill`s old instances
before starting (`run-hybrid-stack.sh`), but **manual** sensor recovery MUST kill the
existing instance first (re-`addService` silently orphans the prior connection). If a
spin appears, find the spinning instance (`top -H`, the hot `binder:<pid>_N`) and, if a
duplicate exists, `kill -9` the orphan; otherwise restart `wart-sensormanager` (drops
the stale server-side `EventQueue`).

**The real fix — by AVOIDANCE, not a library patch (user decision 2026-06-07).**
The spin only occurs when a BitTube **hangs up**, which only happens because of our
**connection churn** (service restarts / duplicate instances / reconnects). With one
stable `wart-sensormanager` + one stable arbiter event-queue connection, the poll
threads sleep — no hangup, no spin (device-observed). So the agreed fix is to make
the `--no-art` native-service bringup **churn-free and shim-first** so the hangup
never happens — using `libsensorserviceaidl` **as-is, unpatched**. See
**`docs/artless-native-service-model.md`** and **`tasks/96-artless-native-service-bringup.md`**,
which supersede the earlier "patch `aidl/EventQueue.cpp`" idea (rejected: no platform-lib patches).

### C4 — Cross-path HAL perturbation onto the camera — OPEN
The wart AIDL consumers and the camera's timing-sensitive EIS gyro share the one
sensors HAL via sensorservice. A misbehaving high-rate AIDL consumer (observed: a
stale arbiter binary holding the **gyroscope @ 200 Hz**, ~200 events/sec each costing
a binder round-trip) loads the same HAL the camera EIS depends on — a plausible
aggravator of the task-95 gyro startup race. By-design multiplexing, but not isolated;
keep AIDL consumers to the minimum sensors + lowest rates (orientation @5Hz on-change).

## Non-conflicts (verified good)
- Single-client sensors HAL invariant holds (`lshal`: only sensorservice). `wart-sensors`
  deleted — no second direct opener.
- HIDL (camera) and AIDL (wart) are **separate** `android::SensorManager` instances in
  wart-sensormanager (different package names) → independent ISensorServer connections.
- Java SensorManager being dead is fine — guests reach sensors through the `sensors` WIT
  (`wart-host/src/sensors_impl.rs` → `wart-hal-sensors` → AIDL ISensorManager).

## Note on the `--probe-video imagereader` freeze (2026-06-06)
Observed "display unresponsive" after the probe was a **hang, not a spin** (every
thread `S`, nothing `R`, probe process already exited; wart-sensormanager at 0%).
That points at the camera-open path (task 93/95), not the sensor looper — though C2/C4
are exactly the cross-path coupling that could turn a camera-open stall into a
whole-system freeze.
