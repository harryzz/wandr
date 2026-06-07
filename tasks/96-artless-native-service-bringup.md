# Task 96 — Churn-free, shim-first `--no-art` native-service bringup

> Status: ✅ DONE + device-verified (Pixel 2 XL, 2026-06-07). Zero platform-lib
> patches. Supersedes the C3 "patch `aidl/EventQueue.cpp`" proposal
> (`docs/sensor-access-conflicts-no-art.md`). Full analysis:
> `docs/artless-native-service-model.md`.
> Constraint of record (user, 2026-06-07): **do NOT patch platform libraries —
> use sensorservice / audioserver / `libsensorserviceaidl` / the HALs as-is.**
>
> ## Implementation (device-verified)
> - **`runtime/wart-framework-shim/`** (new C++ binary; retires `wart-activityms`):
>   registers the source-derived blocker set, started **shim-first** before
>   audioserver/sensorservice. `waitForService` blockers (`activity` [FATAL for
>   audioserver UidPolicy], `sensor_privacy`, `package_native`, `processinfo`) +
>   `checkService`/`getService` paths (`scheduling_policy`, `permission`,
>   `permission_checker`, `media.camera.proxy`). Per-call trace gated behind
>   `WART_SHIM_TRACE`. Build on a-03 (new module → `m` dies in LineageOS dexpreopt
>   kati; direct-ninja the soong intermediate).
> - **`tools/scripts/run-hybrid-stack.sh`** split into a **native+shim layer**
>   (`bring_up_native_shim`, idempotent, skipped when `native_shim_healthy`) and a
>   restartable **wart layer**; new **`--wart-only`** fast restart (no `--restore-art`
>   boot); framework-up gate dropped under `--no-art`.
> - **Key finding:** there is **no real `DEAD_OBJECT` race** on taimen — the single
>   standalone-sensorservice claim succeeds first try; the qcom SSC HAL just takes
>   ~13 s to enumerate the gyro (process stays alive). So the claim is **once + patient
>   poll** (retry only if the process actually dies / FATAL-aborts), not kill-retry churn.
> - **Verified:** cold `--restore-art`→`--no-art` = exactly one of each service,
>   once-fresh claim (no `DEAD_OBJECT`), ISensorManager registers, arbiter receives
>   live sensor data, audio fresh, idle CPU ~6 %, no EventQueue spin — stock libs.
>   `--wart-only` = **27 s**, native+shim pids unchanged, arbiter fresh, sensors/audio
>   keep working.
> - **Gotchas fixed during bring-up** (see `.task-state`): `cmd package
>   resolve-activity` exit-20 vs `set -e`/pipefail; `pkill -9 -f` self-matching its own
>   `su -c` cmdline (→ kill by device-side `pidof`); `service list` *pinging* dead
>   wart-layer services and blocking (→ `service check <name>` for the health probe).

## TL;DR

The `--no-art` bringup stands up framework-coupled native services (sensorservice,
audioserver, cameraserver) **after** the full framework has already started and
claimed HALs, then **orphans** them by stopping the framework. Everything after that
— sensorservice `DEAD_OBJECT`, the audioserver restart cycle, manual sensor
recovery, the duplicate-instance CPU spin, and the C3 EventQueue busy-spin — is us
**fighting that contamination with restarts**, i.e. **churn**. The fix is to make
the native-service bringup **churn-free and shim-first**: stand up a minimal
framework-shim **before** the native services come up, and start each native service
**once, fresh,** in the post-ART context — so nothing wedges, no HAL handoff races,
and no connection ever hangs up (so the EventQueue spin never occurs **without
patching the library**). C3 closes by *avoidance*.

## Why (the root cause)

See `docs/artless-native-service-model.md`. In one line: framework-up-then-stop
contaminates services that were claimed in framework context → restart-to-clean →
churn → all the symptoms. The two services even fail differently
(audioserver *wedges* — it's `class core`, survives, loses its system_server clients;
sensorservice *dies* — it's hosted in system_server — and our standalone replacement
hits the single-client sensors-HAL handoff race → `DEAD_OBJECT`), but both are the
same root.

## Goal

A `--no-art` native-service bringup that is **deterministic and idempotent**:
- exactly one of each service, no kill-restart retries, no manual recovery;
- no `DEAD_OBJECT` on the sensors-HAL claim;
- no audioserver wedge / re-registration cycle;
- no EventQueue busy-spin (stable single connection → no BitTube hangup);
- **zero platform-library patches.**

**And the day-to-day payoff — a fast `--no-art` wart-only restart, NO `--restore-art`
cycle.** Today, restarting the stack means `--restore-art` (boot the full Java
framework, ~1–2 min) → wait → `--no-art` (restart everything + stop the framework
again), because the script gates on the framework being up and re-boots it to "reset"
the native services. Once the native+shim layer is churn-free and idempotent, a stack
restart should be: **leave the native+shim layer running, restart only the wart layer
(arbiter / hosts / inputflinger) in place** — no framework boot. (The *first* entry
into `--no-art` from a cold ART-up boot still needs the one framework-stop; only
subsequent restarts become the fast path.)

## Approach (model → steps)

1. **First-class framework-shim, serving before native services start.**
   Promote the ad-hoc `wart-activityms` stubs into a designed shim (the arbiter, or a
   dedicated `wart-framework-shim`) that registers the *minimal* binder service set
   native daemons block on / query (`activity`, `permission`, `sensor_privacy`,
   `scheduling_policy`, `package_native`, `media.camera.proxy`, …) and is **up and
   serving before** audioserver / sensorservice / cameraserver are (re)started.
   - Verify the exact `waitForService` / `checkService` set each daemon needs
     (read the daemon sources in `runtime/wart-host/vendor/aosp-frameworks-*`), so
     the shim is minimal and complete — no more "discover-a-missing-stub-by-hang."

2. **Start each native service ONCE, fresh, in the `--no-art` context.**
   - **sensorservice** (hosted in system_server → dies with it): ensure the
     framework's instance is fully gone **and the sensors HAL has released its
     client** before our standalone `/system/bin/sensorservice` claims it — so the
     first claim succeeds (no `DEAD_OBJECT`, no retry). Sequence on a real signal
     (HAL client-released / service-gone), not `sleep`. It is **not** an init
     service, so we own its single launch; never run two.
   - **audioserver** (`class core`, init-respawns): with the shim already serving,
     a single `pkill audioserver` → init respawn re-registers `media.audio_*`
     cleanly — or, better, **avoid even that restart** if it can come up post-shim
     without having wedged. Confirm which on-device.

3. **One stable arbiter sensor connection — never churn it.**
   The arbiter's event-queue connection to `wart-sensormanager` must be established
   once and kept; the C3 reconnect path stays as a *recovery* mechanism but must not
   be exercised in steady state. Stable connection ⇒ no BitTube hangup ⇒ the
   unguarded `EventQueueLooperCallback::handleEvent` never busy-loops. **No
   `libsensorserviceaidl` patch.**

4. **Eliminate the contamination source where feasible.**
   The contamination comes from the `--restore-art` → `--no-art` dev cycle (full
   framework boots + claims HALs first). Evaluate a **clean transition / boot
   straight into `--no-art`** so native services are started once and never orphaned.
   `--restore-art` may remain a dev convenience, but the native-service bringup must
   be churn-free regardless of how we reached `--no-art`.

5. **Split the bringup into two layers → enable a fast wart-only restart (no
   `--restore-art`).** This is the mechanism for the restart goal above.
   - **Native + shim layer** — sensorservice, audioserver, the framework-shim
     (step 1), `wart-sensormanager`. Brought up **once, idempotently**; on a restart,
     **detect-and-skip if already healthy** (`service check` / `dumpsys`), never
     kill/respawn. This layer outlives wart-stack restarts.
   - **Wart layer** — arbiter, host zygote + hosts, inputflinger. Freely
     **restartable in place on top** of a live native+shim layer.
   - **Drop the framework-up gate.** The script currently requires the framework up
     to resolve the launcher via `cmd package resolve-activity` (bails exit-20
     otherwise). Cache/persist that (the arbiter already persists home) so the wart
     layer can be (re)started while already in `--no-art`, with no `--restore-art`.
   - **Clean teardown on restart.** A wart-layer restart still drops the arbiter's
     sensor event-queue connection — that's **bounded, clean churn** (arbiter process
     dies → its binder ref drops → `wart-sensormanager` tears that EventQueue down →
     new arbiter makes a fresh one), NOT the persistent spin (which needs an
     *orphaned* queue from a duplicate re-`addService` / sensorservice restart). Also
     re-establish chrome/input registration on the arbiter reconnect (the bare-arbiter-
     restart drops Chrome surface + `wart.windowreg` regs today) so the wart-only
     restart is actually clean.

## How to verify

- `--restore-art` → `--no-art` (and, if built, the clean transition):
  - sensors come up on the **first** sensorservice claim — no `DEAD_OBJECT` line in
    `sensorservice.log`, no retry, no manual recovery.
  - `dumpsys media.audio_policy` registered without an audioserver re-cycle (or with
    at most one deterministic restart).
  - `dumpsys sensorservice` shows **one** open event connection; `top -H` on
    `wart-sensormanager` shows **no** thread above a few % at idle (no spin).
  - exactly one `sensorservice` and one `wart-sensormanager`.
  - idle CPU at the task-86/CPU-fix baseline (~10%), held across a screen-off period.
- All of the above **with stock platform libraries** (diff `libsensorserviceaidl`
  against the device's — unchanged).
- **Fast wart-only restart:** while already in `--no-art`, restart the wart layer
  **without** `--restore-art` / a framework boot — the native+shim layer stays up
  (same sensorservice / audioserver / `wart-sensormanager` pids), sensors + audio
  keep working, no `DEAD_OBJECT`, no spin, and the UI (chrome + input) comes back
  registered. Restart completes in seconds, not the ~1–2 min ART-boot cycle.

## Out of scope / notes

- Not a system_server reimplementation — the shim is minimal (only what daemons
  block on/query).
- The calibrated auto-brightness curve (`config_autoBrightness*`) is a separate
  follow-up (`docs/artless-native-service-model.md` is about *bringup*, not policy).
- Related: tasks 87 (artless audio), 85/86 (sensors/brightness), 93/95 (camera),
  77 (arbiter SensorService), and the C3 entry this supersedes.
