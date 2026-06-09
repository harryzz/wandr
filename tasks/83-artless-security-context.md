# Task 83 — ART-less security context (run wandr native procs as system_server's context)

> Status: 🟢 dev stand-in DONE + device-verified (2026-06-04). `tools/wandr-launch/`
> built (NDK, plain C); it drops root → uid system + gid system,graphics,input +
> retains CAP_BLOCK_SUSPEND/SYS_NICE/WAKE_ALARM (ambient, across exec). **Proven:**
> the path-A inputflinger spike run via `wandr-launch` (framework stopped, setenforce 0)
> cleared BOTH blockers — `addService(inputflinger) → 0`, `InputManager::start() → 0`,
> EventHub enumerated all devices (no abort), touchscreen reconfigured to display 0,
> and `service list` shows `inputflinger: [android.os.IInputFlinger]` on our uid-system
> pid. So this also **fully proves PATH A** (AOSP InputManager runs standalone as the
> inputflinger service, ART off). Remaining: wire `run-hybrid-stack --no-art` to launch
> SF-privileged procs via `wandr-launch` (next), and the flashable-image init.rc+sepolicy
> form (deferred). See `post-art-roadmap.md` §6.6.

## Why

With the Java framework stopped, system_server's permission service is gone, so a
wandr native process can only use the surviving native daemons (SurfaceFlinger,
audioserver, …) if it runs in roughly system_server's security context. The
inputflinger spike pinned the exact requirements (and they generalize):
- **uid `system` (1000)** — else SurfaceFlinger's `ACCESS_SURFACE_FLINGER` check
  *hangs* (routes to the dead permission service); SF short-circuits for system/graphics.
- **gid `input` (1004)** — EventHub `/dev/input` access.
- **`CAP_BLOCK_SUSPEND`** — EventHub aborts otherwise (`EventHub.cpp:894`, EPOLLWAKEUP).
- a **sepolicy domain** permitted to register/call those services.

Bare root fails: it *hangs* on the SF check and *aborts* in EventHub. This same
context is what **task 81 `setPowerMode`** needs (root hangs under ART-off too).

## Decision (2026-06-04): dev scaffold now, flashable image deferred

The production realization is build-time config in our own image (`init.rc`
`user`/`group`/`capabilities`/`seclabel` + a wandr sepolicy module, ART services not
started, enforcing). That's deferred (needs the full `lineage_taimen` device build +
vendor blobs). **This task builds the dev stand-in** that mimics that context on the
rooted device, so runtime/service work isn't blocked on the image.

## Scope (dev stand-in)

- **`wandr-launch` — a tiny setuid+caps launcher** (C, built on a-03 or NDK-static):
  `prctl(PR_SET_KEEPCAPS,1)` → `setgroups([system, input, …])` → `setgid` →
  `setuid(system)` → raise `CAP_BLOCK_SUSPEND` into permitted+effective(+ambient) →
  `execv(target, argv)`. Usage: `wandr-launch /system-context /data/local/tmp/wandr-inputflinger …`.
  (`su <uid>` drops caps; `setpriv` isn't on the device — hence a launcher.)
- **`run-hybrid-stack.sh`**: launch the SF-privileged wandr procs (the standalone
  inputflinger, and whatever issues `setPowerMode`) through `wandr-launch` under
  `--no-art`; keep `setenforce 0` as the dev sepolicy stand-in. The non-privileged
  hosts/arbiter stay as-is (root) since they don't need the system context.
- **Verify the requirements are satisfied**: a process launched via `wandr-launch`
  shows `uid=system gid=input … CAP_BLOCK_SUSPEND` and no longer hangs on SF / aborts
  in EventHub.

## Out of scope (the deferred "right way")
- Flashable `lineage_taimen` image with our `init.rc` + sepolicy module + ART disabled
  (the production form; a separate milestone — §6.6).

## Verification (device, `--no-art`)
- Re-run the inputflinger spike via `wandr-launch` → it gets **past** EventHub +
  the SF check → `addService("inputflinger")` succeeds → `start()` OK (with
  `setenforce 0`). Confirms the context recipe.
- Unblocks task 81: the `setPowerMode` caller via `wandr-launch` toggles the panel
  under ART-off without hanging.

## Related
`post-art-roadmap.md` §6.6, `[[project_art_shutdown]]`, task 80 (input), task 81
(display power). InputFlinger architecture decision (path A vs host-reads) rides on
this being in place.
