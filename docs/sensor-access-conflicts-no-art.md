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

### C2 — One shared SCHED_FIFO poll thread for all AIDL consumers — OPEN
`SensorManagerAidl::getLooper` (vendored `aidl/SensorManager.cpp`): *"One global
looper for all event queues."* That single thread runs **SCHED_FIFO prio 10**
(device-confirmed: the spinner is tid `binder:<pid>_N` with `rt_priority 10`,
`policy 1`, `epoll_wait` stack — it's the poll thread, mis-named because it's spawned
from the `createEventQueue` binder worker) and makes a **synchronous `onEvent` binder
call per event to each client**. So (a) a slow/blocked client stalls sensor delivery
for *everyone* (head-of-line blocking), and (b) when it runs hot it starves
normal-priority threads (render/input/adbd) on its core. This is the mechanism
behind "100% pins a core" and a frozen UI. C1 halves the AIDL clients but doesn't
remove the shared-thread coupling. Possible fixes: per-client queue draining off the
RT thread, or async/oneway `onEvent`, or drop the RT priority.

### C3 — Latent HANGUP busy-spin × non-reconnecting client cache — OPEN
The AIDL `EventQueueLooperCallback::handleEvent` (`aidl/EventQueue.cpp`) has **no
`ALOOPER_EVENT_HANGUP` / `read()<0` guard** (unlike `SensorEventQueue::waitForEvent`,
`libs/sensor/SensorEventQueue.cpp:134`), so a dead BitTube fd stays level-triggered
readable and the shared RT thread busy-spins. Compounding it, `wart-hal-sensors`
caches the event queue on success and **never rebuilds** (`src/lib.rs:173`), so after
a sensorservice restart a stale dead queue would spin forever. Latent — not observed
in current runs (strace showed normal `EAGAIN` draining, never `recvfrom`=0). Fix:
add the HANGUP guard (return 0 to drop the fd) in the vendored callback + recreate the
queue client-side on stall.

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
