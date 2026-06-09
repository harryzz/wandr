---
name: project_true_dp_geometry
description: "Arbiter Increment 3b — the arbiter authors chrome heights/insets in dp (dp×density), host reports panel+density up, all physical-px env hardcodes (132/150/1200, WANDR_INSET_*) removed. Done + device-verified."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Arbiter Increment 3b — true-dp geometry. DONE + device-verified (Pixel 2 XL,
2026-06-01). Commit `31b3d00e` on main (after chrome-coherence).** Satisfies the
no-hardcoding rule ([[feedback_no_hardcoding]]) for chrome geometry; the
arbiter-as-window-server direction ([[project_arbiter_window_server_design]]).

**Problem:** chrome heights were physical-px hardcodes — status bar 132, taskbar
150, keyboard default 1200 — env-tunable (`WANDR_INSET_*`/`WANDR_STATUSBAR_PX`/
`WANDR_TASKBAR_PX`) and resolution-dependent.

**What shipped — the arbiter is the geometry authority:**
- **dp constants are the ONE source** (`wandr-arbiter-core`): `STATUS_BAR_DP=38`,
  `TASKBAR_DP=43`, `KEYBOARD_DEFAULT_DP=343` (back-derived from the tuned px at
  density 3.5 to preserve the look; scale on other densities). `DisplayGeometry::
  dp_to_px(dp)` = `(dp×density).round()`; `chrome_insets()`; `density_known()`.
- **Unifying idea:** the `geometry <inset_top> <inset_bottom> <keyboard_px>
  <orient>` line's inset fields ARE the chrome strip heights `(sb, tb)`, pushed to
  EVERY surface — fullscreen reserves them as content insets; chrome sizes its
  strip to them; the IME anchors off them. No new wire fields.
- **report-panel <w> <h> <dpi>** (wm verb): host reports panel + density; arbiter
  stores `density=dpi/160`; **reply carries the authored insets** so the caller
  `set_insets` before its first frame (no launch-time push/socket race — child
  pulls, ongoing changes push). `has_surface_policy` = density known.
- **register-chrome <app-id> <pid> <anchor>** (shell, extended): reply carries the
  strip `height` (`dp_to_px` by anchor: top→SB, bottom-bar→TB) so the overlay
  sizes its surface to the arbiter value. `WmModule` is now stateless (insets from
  the store, not env).
- **Host** (`wandr-host`): fullscreen sends report-panel at startup + applies reply
  insets; chrome registers-with-anchor + creates its overlay at the reply height;
  the geometry line's inset fields are cached host-side (`CHROME_SB/TB_PX` in
  standalone.rs) for `overlay_rect`; `status_bar_height_px()`/`taskbar_height_px()`
  return the cached arbiter value with a `dp×read_dpi` fallback (fallback dp consts
  MIRROR core's — keep in lockstep; arbiter is authoritative). `window_impl::read_dpi`
  made pub. `run-hybrid-stack.sh` drops all the WANDR_*_PX/WANDR_INSET_* env.

**Device proof:** density 560→3.5 reported up; arbiter authored insets `(133,151)`
(≈ old 132/150, 1px from rounding); statusbar/taskbar surfaces `1440x133` / `1440x151`;
fullscreen logical `1440x2596` (2880−133−151). Rotation + keyboard + chrome stay
coherent — `overlay_rect` uses the cached sb/tb, the IME anchors at `tb=151`, the dp
insets ride the geometry line through the flip. 23 arbiter unit tests; host build green.

**Gotcha (regression fixed, commit `393081bf`):** the geometry line's inset fields are
the chrome heights (sb,tb) pushed to EVERY surface, but the host must apply them as
content insets ONLY on a fullscreen app (`OverlayMode::None`). A chrome overlay IS the
chrome — it renders its full strip; insetting a 133px status bar by (133,151) shrank its
content to nothing → blank clock / blank taskbar icons. Overlays still *cache* sb/tb (for
`overlay_rect`) but keep zero content insets. (Pre-true-dp this was masked because chrome
got `INSET_HOST_OWNED` → set_insets skipped.)

**Gotcha:** host fallback dp consts (`FALLBACK_STATUS_BAR_DP=38` etc. in standalone.rs)
duplicate core's — used ONLY when the arbiter never provided a value (arbiter-down /
pre-report); the race is benign (same dp × same density source → same px). Keep them
mirrored if the core dp values ever change.

Build: `build-host-android.sh` (host changed) + `run-hybrid-stack.sh`. Plan:
`~/.claude/plans/cat-task-state-steady-stallman.md`.
