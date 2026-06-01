---
name: project_arbiter_window_server_design
description: "Design of record — wart-arbiter becomes the native shell/window-server (AMS+WMS+IMMS), modular core+crates, derived state. Doc + first slice landed 2026-06-01."
metadata: 
  node_type: memory
  type: project
  originSessionId: b4642c38-ac22-459b-92dd-7b4430418889
---

**Design of record:** `docs/visual-sizing-design-patterns.md` (committed-tree
doc). The non-hardcoded sizing problem is a classic responsive-UI design-pattern
problem (dp, intrinsic sizing + constraints, insets/safe-area, reactive
recompute, negotiated sizing) — NOT a wasm/c++ problem. wart differs from
Android only in that the "system" is split across processes, so the patterns run
**over the WIT/IPC boundary** (host measures + pushes constraints; guest computes
its own intrinsic size in dp; host applies verbatim).

**Target:** the arbiter is wart's native `system_server`-coordinator — it
DECIDES, never renders. Absorbs three responsibilities (the logic, not Java/
binder): **AMS** (foreground/roles/tasks — already there), **IMMS** (editor focus
→ keyboard — mostly there after task 71), **WMS** (geometry/insets/z-order/
visibility — the missing piece, still mostly in the host's `recompute_transform`/
`overlay_rect`). Keeps Android's policy/mechanism split: arbiter=policy,
SurfaceFlinger=compositor, InputFlinger=input, per-app host=renders, guest=UI.
Note: Android's WMS/IMMS/AMS are **Java in system_server over binder** — the ART
layer we drop, so NOT reusable; we reimplement the responsibilities natively.

**Modularity (planned, not built):** thin `wart-arbiter-core` (event loop +
shared-state store + event bus + `ArbiterModule` trait + registry); each
responsibility = its own crate (`-wm`, `-ime`, `-am`, `-notify`, `-alarm`,
`-audiofocus`). Modules never call each other — emit/observe events, read/write
the store via `Ctx` (Open/Closed). Strangler migration: add core alongside, wire
NEW responsibilities as modules, migrate existing logic only when already
touching it. Hard cost = getting the core contracts right up front.

**Two bake-in-now decisions** (cheap now / costly later, both anti-hardcodes):
key all state **per-display**; model focus as a **generalized resource-focus**
(window/input/IME/audio) so notifications, audio-focus, alarms, wallpaper,
keyguard later are each "one more resource/role," not a refactor. Foreseen
system_server policy the arbiter will inherit: DisplayManager, audio focus,
**AlarmManager/JobScheduler (real gap — background message receipt for Signal)**,
Notification/StatusBar, Power, Keyguard/Wallpaper/Dream, Clipboard, runtime perms.

**Status:** doc written; FIRST real slices landed in task 71 (see
[[project_keyboard_overlay_lifecycle]]) — IMMS-style `reconcile_overlay` derived
state + the WMS `present` visibility push — but inside the still-MONOLITHIC
arbiter. `wart-arbiter-core` + the crate split are NOT started; WMS geometry is
NOT yet moved out of the host. Today's *principle* carries forward; today's
*placement* (global singletons in main.rs) gets relocated when the modular core
lands. Language-neutrality survey for the demo:
`docs/wasm-component-language-support.md`.
