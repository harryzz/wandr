# `wasi:input-handlers` — draft proposal (wandr, 2026-06-11)

Push-model input + frame driving for **host-driven (reactor) runtimes**:
the guest EXPORTS handlers, the embedder calls them. The reactor-model
counterpart to `wasi:surface`'s guest-pulled pollables — precedent for
handler exports is `wasi:http/incoming-handler`. Pairs with
`wasi:canvas` (+ its `embedding` interface) to make a guest UI component
that needs nothing host-specific. Status: wandr-local draft, validated
(`wasm-tools component wit wit/`), implemented + live on wandr.

## Interfaces

- `pointer-handler` — multi-touch pointer events (id, kind incl. cancel,
  position, pressure, scroll deltas, modifiers).
- `key-handler` — the W3C UIEvents key model (physical `code` token +
  resolved `text` + modifiers + repeat). Identical record to
  `my:skiko-gfx/key-input`, which it supersedes for new-style guests.
- `frame-handler` — `on-frame(nanos)` (where a wasi:canvas guest brackets
  drawing with `embedding.begin/end-frame`) + `on-resize`.

Probe-only worlds per interface: hosts bind each independently after
instantiation; a guest may export any subset.

## Routing rule (the de-dup design)

Per input type, a host routes **EXCLUSIVELY** to a bound handler; legacy
event paths (wandr: the `my:skiko-gfx/renderer` exports) fire only when
the handler is absent. Guests can therefore export BOTH the new handlers
and a legacy fallback without ever seeing duplicates — and degrade
gracefully on hosts that predate the handlers.

## Status in wandr

Implemented host-side (not feature-gated — probing is free), routed at
every input source: winit desktop (real modifiers + repeat + mouse),
Android InputFlinger (AKEYCODE→W3C map + AMETA modifiers), IME soft keys.
Proving consumer: `slint-wandr` exports all three (the legacy renderer
exports remain as old-host fallback); verified rendering + routing on
the Linux desktop host and the Pixel 2 XL.

## Known not-yet

- Scroll events: the record carries deltas but no host source emits
  kind=scroll yet (desktop mouse-wheel mapping is the obvious first).
- Kotlin-class guests can't consume these (handler records with strings —
  the cabi_realloc export restriction); the legacy renderer interface
  remains load-bearing for Compose, by design.
- Lifecycle/scheduler callbacks stay on the legacy interface (candidates
  for a later `lifecycle-handler` if a consumer needs them).
