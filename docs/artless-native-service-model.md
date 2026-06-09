# The `--no-art` native-service model — why the bringup is a mess, and the clean shape

> Analysis doc (2026-06-07). Scope: why standing up the native media/sensor
> services under `--no-art` currently needs a pile of `pkill`/`spawn`/`sleep`
> workarounds in `run-hybrid-stack.sh`, what the *real* root cause is, and the
> churn-free / shim-first model that dissolves the whole class — **using the
> platform libraries and services as-is, with no library patches.**
> Companion task: `tasks/96-artless-native-service-bringup.md`.
> Supersedes the "patch `aidl/EventQueue.cpp`" proposal in
> `docs/sensor-access-conflicts-no-art.md` C3.

## The symptom inventory (what "the mess" actually is)

Under `--no-art` (Java framework stopped) the bringup has accreted these
workarounds, each addressing a different surface symptom:

- **`wandr-activityms` stubs** — re-implement the system_server binder services that
  native services block on (`activity` / `permission` / `sensor_privacy` /
  `scheduling_policy` / `package_native` / `media.camera.proxy`).
- **audioserver restart cycle** — start the stub, wait for `"activity"`, then
  `pkill -9 audioserver` so init respawns it and it re-registers `media.audio_*`
  fast instead of wedging ~20 s on `waitForService`.
- **sensorservice kill-claim-retry** — `pkill` + `spawn_detached /system/bin/sensorservice`,
  retry up to 3× until the gyro enumerates without `"Abort due to ISensors … DEAD_OBJECT"`.
- **manual single-instance recovery** — when the 3 retries give up, hand-restart
  sensorservice + `wandr-sensormanager`, being careful to leave exactly one of each.
- **the EventQueue busy-spin** (C3) — an orphaned AIDL event-queue poll thread pins
  a core; "fix" proposed was to patch `libsensorserviceaidl`.
- **the duplicate-instance CPU spin** — accidentally running >1 `wandr-sensormanager`
  → orphaned EventQueues spin → +CPU.

## These are NOT one mechanism — but they ARE one root

### Two distinct failure *classes* at the symptom level

| | how it's started | under `--no-art` | failure | the workaround |
|---|---|---|---|---|
| **audioserver** | **init service, `class core`** (survives `stop`; init respawns on kill) | **survives**, loses its system_server clients | **wedges** on `waitForService("activity")` | stub + `pkill` (init respawns one) → re-registers |
| **sensorservice** | **hosted inside system_server** (`SystemServer.java` `startService(SensorService.class)`); **NOT** an init service | **dies** with the framework | standalone `/system/bin/sensorservice` we start hits the **single-client sensors-HAL handoff race** → `DEAD_OBJECT` | kill + spawn + retry |

So **`DEAD_OBJECT` is not the same problem as the audioserver cycle.** audioserver
never touches a single-client HAL the way sensorservice does; its restart is purely
*dependency ordering*. sensorservice's `DEAD_OBJECT` is a **HAL ownership handoff
race**: the framework's sensorservice (inside system_server) held
`android.hardware.sensors@1.0::ISensors`; system_server dies; the HAL has not yet
released the dead client; our standalone instance tries to claim it → dead binder →
retry until the HAL cleans up.

### The single root cause underneath all of it

> We run **framework-coupled native services without the framework**, and we reach
> that state via **`--restore-art` → `--no-art`**: the services first come up in
> **full-framework context** (claim HALs, bind to system_server's services), and we
> then **orphan them** by stopping the framework. Every workaround after that is us
> **fighting the contamination** — restarting services to get clean state, which
> creates **churn**: HAL-handoff races, re-registration stalls, and BitTube hangups.

The **EventQueue spin is itself a churn symptom.** `EventQueueLooperCallback::handleEvent`
lacks an `ALOOPER_EVENT_HANGUP` guard, but that only bites when a BitTube **hangs
up**, which only happens because we **restart / duplicate / reconnect**. With **one
stable connection** the poll threads sleep — observed directly: after settling to a
single `wandr-sensormanager` with a single arbiter event-queue connection, no thread
spun. **The upstream library is fine as-is; the churn is ours.**

## Why patching the library is the wrong fix

- It mutates a platform `.so` (ship + side-load fragility) to paper over a hangup we
  **cause**. Remove the churn and the hangup never happens.
- It treats the symptom (spin on hangup) instead of the cause (we keep hanging up
  connections by restarting things).
- Constraint of record: **use the platform libraries/services as-is.** No patches.

## The clean shape

Two coherent moves remove the entire class — no library patches, services as-is:

### 1. A designed framework-shim, up *before* native services start

The stubs are **not** hacks to remove: providing the binder services that native
services depend on is *literally what replacing system_server means*. The hack is
that they're stood up **ad-hoc, after the fact, with kill-restart timing dances**.
The clean form is a **first-class shim** (the arbiter, or a dedicated
`wandr-framework-shim`) that registers the minimal service set (`activity`,
`permission`, `sensor_privacy`, `scheduling_policy`, `package_native`,
`media.camera.proxy`, …) and is **already serving before** audioserver /
sensorservice / cameraserver come up — so nothing ever wedges or needs a
re-registration restart.

### 2. Start native services *once, fresh,* in the `--no-art` context (no contamination, no churn)

Instead of "full framework boots and claims HALs → stop → restart-to-clean," bring
the framework-coupled services up **once**, in the post-ART context, with the shim
already present:

- **sensorservice** never gets a framework-context instance → our standalone claims
  the sensors HAL **fresh** → **no handoff race → no `DEAD_OBJECT`**.
- **audioserver** comes up (or is restarted exactly once) with the shim already
  serving → **no wedge, no re-registration cycle**.
- the arbiter establishes **one stable** sensor event-queue connection that never
  churns → **no BitTube hangup → no EventQueue spin → no library patch** (C3 closes
  by *avoidance*).
- exactly one of each service → **no duplicate-spin CPU**, no manual recovery.

The contamination comes from the **`--restore-art` dev convenience** (we restart the
full framework between tests, partly to resolve the launcher via `cmd package`). The
clean target is a **boot/transition straight into `--no-art`** (or a clean
framework→no-art handoff that releases HALs and tears down framework clients before
our replacements claim them), so native services are started **once** and never
orphaned.

## What this buys (one change, many symptoms gone)

| symptom | dissolved by |
|---|---|
| sensorservice `DEAD_OBJECT` | start-once-fresh (no HAL handoff race) |
| audioserver wedge + restart cycle | shim-first (deps present before it starts) |
| manual single-instance recovery | deterministic single bringup |
| duplicate-instance CPU spin | exactly one of each, no respawn races |
| EventQueue HANGUP busy-spin (C3) | stable connection → no hangup (no lib patch) |

**Plus the day-to-day win: a fast `--no-art` wandr-only restart, no `--restore-art`
cycle.** Once the native+shim layer is churn-free and idempotent, the bringup splits
into a **native+shim layer** (brought up once; skip-if-healthy on a restart) and a
**wandr layer** (arbiter / hosts / inputflinger, restartable in place). Restarting the
stack then leaves the native+shim layer running and restarts only the wandr layer —
**no full-framework boot** (the `--restore-art` → `--no-art` cycle is only needed for
the *first* entry into `--no-art`, not for restarts). See task 96 step 5.

## Non-goals / constraints

- **No platform-library patches.** Use `libsensorserviceaidl`, sensorservice,
  audioserver, the HALs **as shipped.**
- Keep the shim **minimal** — only the services native daemons actually block on /
  query; it is not a system_server reimplementation.
- This is the model, not an implementation plan for booting without ART — the dev
  flow's `--restore-art` may stay as a convenience, but the native-service bringup
  must become **churn-free and shim-first** regardless of how we got to `--no-art`.
