# Task 101 — desktop dev loop (no device) + W3C key-input ✅

> DONE + user-verified 2026-06-11, in one session with task 100's wrap-up.
> Commits: `e9dc7ba4` (key-input v3 + desktop build un-broken), `396d5ef7`
> (softbuffer present), `2549c6a0` (resize/size/fonts + docs),
> `f793c9a5` (emoji probe). Recipe: `docs/build-pipeline.md`
> "Desktop dev loop".

## What shipped

**The x86_64 wandr-host runs the SAME guest wasm as the device**, JIT, in a
winit window (WSLg/Wayland/X11 — verified live under WSLg with the user
interacting):

```bash
WANDR_DESKTOP_SIZE=500x1000 \
  runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host \
  apps/user/<app>/components/ui.wasm
```

Verified end-to-end on wandr.slint.test (+ dioxus demo differential):
render parity with the phone (gradients/shadows/lists/glyphs/emoji), mouse,
HW keyboard with modifiers, live WM resize, ~the same on-demand idle.

## The pieces (each was a real gap)

1. **`my:skiko-gfx/key-input` (v3 keys, the W3C UIEvents model)** — the
   motivating item ("don't hardcode keyboards to mobile"; early testing ran
   a Wayland compositor on a Linux host). Probe-only world (frame-pacing
   pattern, additive): `key-event { down, repeat, code, text, alt, ctrl,
   meta, shift }` where `code` is the W3C UIEvents TOKEN ("KeyA",
   "ArrowLeft") — string not enum (the W3C table grows; winit `KeyCode`
   Debug names ARE the tokens; 1:1 with wasi:surface's enum). Hosts emit
   v1+v2+v3 from all three sources: winit desktop (real codes +
   `ModifiersChanged` tracking + repeat), InputFlinger (AKEYCODE→token +
   AMETA alt/ctrl/meta/shift), IME soft keys (key-id→token). slint-wandr
   exports + prefers it (modifier keys → Slint's special code points →
   ctrl+c works); v2 stays as fallback until the first v3 event arrives.
   USER-VERIFIED with a real HW keyboard.
2. **Desktop build un-broken** (pre-existing since task 93): `crypto.rs`
   hwcap tokens are aarch64-only → gated module (sw fallback reports
   false); `keyboard_host_impl`/`status_impl` referenced android-gated
   modules → cfg'd (desktop status-bar inset = 0).
3. **Present path** — ‼️ the desktop renderer drew into an offscreen skia
   CPU raster surface and `flush_and_swap` had an android-only body:
   desktop windows were ALWAYS black (render ok=true, nothing presented).
   Fix: softbuffer blit (no GL — reliable under WSLg): N32-premul BGRA
   rows reinterpreted as 0RGB u32 LE, per-frame resize+present.
4. **Resize forwarded to the guest** — desktop never dispatched
   `on-resize`; a WM resize left the guest laid out for the stale size.
   ‼️ This single gap masqueraded as THREE rendering bugs (gradient card
   black, slider track + ListView rows missing): Slint laid out for 600px
   so the ListView collapsed to zero leftover and content overflowed.
   Localized via differential (dioxus demo rendered perfectly → host
   canvas clean → geometry). Lesson: on missing UI elements, suspect
   layout-vs-surface size mismatch before draw bugs.
5. **`WANDR_DESKTOP_SIZE=WxH`** — phone-shaped viewport env.
6. **Fonts/emoji parity** — `/usr/share/fonts/truetype/noto` preopened as
   `/system-fonts` (matches the device preopen); Slint's emoji fallback
   binds on desktop. Verified with STATIC emoji in the demo (🎨/🦊 render
   in color) — the input-free probe, since neither HW keyboards nor the
   wandr soft keyboard can type emoji.

## Known desktop non-goals / edges

- No arbiter: IME attach/detach logs a warning (no keyboard overlay),
  chrome insets 0, no launcher/roles. `/state` persistence works.
- Scroll-wheel not mapped (PointerKind::Scroll dropped by slint-wandr;
  dioxus also ignores it) — fine on touch, a desktop nicety later.
- Screenshot trick for agents: WSLg XWayland is rootless (`xwd -root`
  fails) — run with `env -u WAYLAND_DISPLAY DISPLAY=:0` (X11 backend),
  find the id via `xwininfo -root -tree`, `xwd -id <id>`, parse the xwd
  header manually (PIL can't read xwd; BGRX rows after
  `header_size + ncolors*12`).
- `pkill -f` traps: the pattern matches the wrapping shell's own cmdline
  (kills the session shell, exit 144) — kill exact pids or `pgrep -f
  "ui.wasm" | xargs -r kill`.

## Why it matters

Edit `.slint`/Rust → `cargo build --target wasm32-wasip2` → run locally →
when it looks right, `wandr-host --install` the IDENTICAL artifact on the
phone. One binary, two hosts (AOT vs JIT, GPU-GL vs CPU-raster, touch+IME
vs mouse+HW-keys) — the live proof of the WIT contract's portability and
the strongest evidence for the wasi-canvas idea
(`docs/skiko-gfx-vs-wasi-gfx.md`).
