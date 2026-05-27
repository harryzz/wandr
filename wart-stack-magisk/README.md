# wart-stack-magisk

Magisk module that auto-starts the wart Hybrid runtime stack
(`wart-host --zygote` + `wart-arbiter --daemon`) at boot. Replaces
the AOSP-init.rc framing — fully reversible from the device, no
custom build needed.

## What it does

At Magisk's `late_start_service` stage (after `/data` is mounted, SF
is up, binder is ready), `service.sh`:

1. Sanity-checks the wart binaries are deployed at `/data/local/tmp/`.
2. (Optional, commented out) Adds SELinux rules via `magiskpolicy --live`.
   Task 46 step 4 needed none — uncomment if denials show up.
3. Starts `wart-host --zygote` as a background process, waits for its
   UNIX socket.
4. Starts `wart-arbiter --daemon` as a background process, waits for
   its UNIX socket.

All output goes to `/data/local/tmp/wart-stack.log` (tail it with
`adb shell tail -f /data/local/tmp/wart-stack.log`).

## Install (one-shot)

From the dev machine:

```
scripts/install-wart-stack-magisk.sh
```

That copies this directory into `/data/adb/modules/wart-stack/` on
device + sets permissions. The module is NOT enabled until you reboot
(Magisk only scans `/data/adb/modules/` at boot). After reboot, the
daemons start automatically.

## Disable / remove (the "backup to restore" mechanism)

Magisk modules have built-in disable + remove flags. No system files
were modified; reverting is just "don't run the module."

| Goal                       | Command (on device, as root)                                  | Effect                                                              |
|----------------------------|---------------------------------------------------------------|---------------------------------------------------------------------|
| Skip the module next boot  | `touch /data/adb/modules/wart-stack/disable`                  | Module's scripts not run. Files stay in place. Reversible.          |
| Re-enable                  | `rm /data/adb/modules/wart-stack/disable`                     | Next boot runs scripts again.                                       |
| Remove on next boot        | `touch /data/adb/modules/wart-stack/remove`                   | Magisk deletes the module dir at boot; `uninstall.sh` runs first.   |
| Stop daemons right now     | `killall -9 wart-arbiter wart-host`                           | Doesn't touch the module — they'll restart on next reboot.          |
| Both at once               | `scripts/uninstall-wart-stack-magisk.sh`                      | Sets the remove flag + reboots. Clean removal.                      |

Live SELinux rules added by `service.sh` (none today; commented-out
template only) are scoped to the boot session. Rebooting WITHOUT this
module = baseline SELinux state. No `/sepolicy` backup needed.

## What's NOT in scope

- The module doesn't ship the binaries. wart-host + wart-arbiter +
  libsf_surface.so still come from `scripts/build-host-android.sh` +
  `adb push`. The module just ensures they auto-start.
- No init.rc service-class semantics (oneshot, critical, restart limits,
  etc.). If a daemon dies, the module doesn't restart it; the next
  boot will. For "restart on crash" you'd want a real init.rc entry
  (which would need an AOSP rebuild).
- No per-domain SELinux policy. We run under Magisk's broadly-permissive
  domain, which is fine for a dev device.

## Tracked by task

`tasks/46-wart-arbiter-mvp.md` step 5.
