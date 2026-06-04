# Task 82 — key input routing / dedup (ART-less)

> Status: 🔲 scoped. Found in human testing of `--no-art` (2026-06-04).

## Why

Task 80 Step 2 routed **touch** per-host (input-region filtering), but **keys are
not routed**. With our standalone InputReader, *every* host's EventHub reads the
hardware keys, so one VOLUME-UP press produced `volume up` from **6 pids at once**
(the arbiter logged six `volume up → speaker`, and the volume stepped ~6×). Keys
need the same single-consumer discipline touches now have.

## Goal

A hardware key is handled once, by the right consumer:
- **Volume** (24/25): once, to the audio owner (the arbiter already decides; it just
  must receive ONE press, not one-per-host).
- **Power** (26): once → display toggle (task 81).
- Other keys: to the input-focused surface only.

## Approach (options)
- **Arbiter-side dedup (simplest first cut):** the arbiter coalesces identical
  key commands arriving within a small window (e.g. same keycode+action within
  ~50 ms) into one. Cheap; no host change. Covers volume/power immediately.
- **Host-side gating (cleaner):** only the input-focused host forwards keys (the
  arbiter grants key-focus, like the touch input-region). Requires the arbiter to
  designate a key-focus host + the host to gate. More work; correct for app keys too.

Lean: arbiter-side dedup for volume/power now (un-break the volume jump), host-side
key-focus as the proper follow-on alongside task-80 Step-2 routing.

## Files (first cut)
- `runtime/wart-arbiter/wart-arbiter-audio/src/lib.rs` (volume) — dedup window; or a
  shared dedup in the binary's command dispatch.
- Reuse task-80 Step-2 input-focus concepts for the host-gating follow-on.

## Verification (device, `--no-art`)
- One volume-up press → volume steps **once** (one `volume up` in the arbiter log,
  not six).

## Related
`[[project_art_shutdown]]`, task 80 (input), task 81 (power key).
