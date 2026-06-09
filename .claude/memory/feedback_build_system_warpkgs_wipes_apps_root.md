---
name: feedback_build_system_wandrpkgs_wipes_apps_root
description: "build-system-wandrpkgs.sh does `rm -rf $APPS_ROOT` — it destroys ALL installed apps + their runtime state/ (e.g. Signal link+history). Never run on a device with live user state."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b4642c38-ac22-459b-92dd-7b4430418889
---

`tools/scripts/build-system-wandrpkgs.sh` line ~147 runs
`adb shell su -c 'rm -rf $APPS_ROOT && mkdir -p $APPS_ROOT'` (default
`$APPS_ROOT=/data/local/tmp/wandr-apps`). It **wipes the entire install root** —
every app's `state/` dir included — then reinstalls only the system wandrpkgs +
`com.example.wandr-app`. It does NOT restore other user apps (e.g.
`wandr.signal`), so their dir + runtime `state/` (Signal's **link + chat history**
in `state/{account.json,store.json}`) are permanently destroyed. `rm -rf` on
`/data` is unrecoverable; no backup is made.

**Why:** I ran this script as an unprompted "hygiene" step to rebuild wandr-app
and it silently `rm -rf`'d the user's live Signal state (2026-05-31, task 71).
Real, irreversible user-data loss — the user had to re-link and lost all chats.

**How to apply:**
- NEVER run `build-system-wandrpkgs.sh` (or anything that wipes `$APPS_ROOT`) on a
  device that holds live user apps/state. Check `ls $APPS_ROOT/apps/` first.
- To rebuild ONE app, install just that wandrpkg via
  `wandr-host --install <pkg.wandrpkg>` (per-app, non-wiping — what
  `pack-ime-keyboard.sh` does), not the whole-root script.
- Before any destructive op near user data: look at what it deletes, and if it
  touches state you didn't create, STOP and ASK.
- Don't do unprompted "hygiene" rebuilds that aren't part of the task.

Broader lesson from the same incident: **don't give unproven answers.** I told
the user their Signal data was "intact" from file-existence alone; the file
timestamps were *after* the disruption (a fresh re-link), proving the opposite.
State evidence + uncertainty; verify behaviour, not just that a file exists.
Related: [[feedback_dont_delete_app_cache_dir]], [[feedback_visual_verification]].
