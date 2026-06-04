# Task 83 — ART-less security context (run wart native procs as system_server's context)

> Status: 🔲 scoped. The shared prerequisite for ALL ART-off privileged native work
> (display power, standalone inputflinger, future SF/audio ops). See
> `post-art-roadmap.md` §6.6 and `[[project_art_shutdown]]`.

## Why

With the Java framework stopped, system_server's permission service is gone, so a
wart native process can only use the surviving native daemons (SurfaceFlinger,
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
`user`/`group`/`capabilities`/`seclabel` + a wart sepolicy module, ART services not
started, enforcing). That's deferred (needs the full `lineage_taimen` device build +
vendor blobs). **This task builds the dev stand-in** that mimics that context on the
rooted device, so runtime/service work isn't blocked on the image.

## Scope (dev stand-in)

- **`wart-launch` — a tiny setuid+caps launcher** (C, built on a-03 or NDK-static):
  `prctl(PR_SET_KEEPCAPS,1)` → `setgroups([system, input, …])` → `setgid` →
  `setuid(system)` → raise `CAP_BLOCK_SUSPEND` into permitted+effective(+ambient) →
  `execv(target, argv)`. Usage: `wart-launch /system-context /data/local/tmp/wart-inputflinger …`.
  (`su <uid>` drops caps; `setpriv` isn't on the device — hence a launcher.)
- **`run-hybrid-stack.sh`**: launch the SF-privileged wart procs (the standalone
  inputflinger, and whatever issues `setPowerMode`) through `wart-launch` under
  `--no-art`; keep `setenforce 0` as the dev sepolicy stand-in. The non-privileged
  hosts/arbiter stay as-is (root) since they don't need the system context.
- **Verify the requirements are satisfied**: a process launched via `wart-launch`
  shows `uid=system gid=input … CAP_BLOCK_SUSPEND` and no longer hangs on SF / aborts
  in EventHub.

## Out of scope (the deferred "right way")
- Flashable `lineage_taimen` image with our `init.rc` + sepolicy module + ART disabled
  (the production form; a separate milestone — §6.6).

## Verification (device, `--no-art`)
- Re-run the inputflinger spike via `wart-launch` → it gets **past** EventHub +
  the SF check → `addService("inputflinger")` succeeds → `start()` OK (with
  `setenforce 0`). Confirms the context recipe.
- Unblocks task 81: the `setPowerMode` caller via `wart-launch` toggles the panel
  under ART-off without hanging.

## Related
`post-art-roadmap.md` §6.6, `[[project_art_shutdown]]`, task 80 (input), task 81
(display power). InputFlinger architecture decision (path A vs host-reads) rides on
this being in place.
