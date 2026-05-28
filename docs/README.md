# wart — architecture docs

Living technical docs for the wart runtime: a wasmtime-based
replacement for Android's ART, running Compose Multiplatform apps
compiled to WASM components on real device hardware.

These docs answer the questions that come up repeatedly while
reading the code. They are NOT user-facing reference. For setup +
build instructions see `~/wart/CLAUDE.md` and each subproject's
`BUILD.md`. For the task narrative see `~/wart/tasks/`.

## Index

| Doc | Audience | What it answers |
|---|---|---|
| [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md) | Anyone touching a WIT contract or `wart-host/src/*_impl.rs` | What is `renderFrame(nanos)` — is it inlined? What does "host-driven" mean? How does a single frame flow across the WASM Component-Model boundary? |
| [`architecture-runtime.md`](architecture-runtime.md) | Anyone touching `wart-host` / `wart-arbiter`, or debugging app launch / focus / lifecycle | What are the three processes (zygote, arbiter, host child) and how do they talk? Full transport tables for the three UNIX sockets + the three signals. End-to-end trace of `wart-arbiter launch <app>`. |
| [`architecture-ime.md`](architecture-ime.md) | Anyone touching `war.ime.keyboard`, the lang plugins, or `ime_inbound` / `keyboard_host_impl` | How does a soft-keyboard tap become a Compose `KeyEvent` in the focused TextField? How are lang plugins (`war.lang.bg` / `.fr`) loaded, and what's TODO (task 51) to make plugin loading dynamic? |

## Conventions

- Docs are append-only narrative — when something changes, prefer
  editing the relevant section over deleting it. Keep historical
  rationale where it explains *why* the current code looks the
  way it does.
- Link liberally to `tasks/<N>-*.md` for the historical task
  context, and to `~/.claude/projects/-home-harry-wart/memory/*.md`
  via double-bracket `[[memory-slug]]` syntax for design notes
  that aren't task-scoped.
- Each doc opens with a TL;DR + an ASCII process / boundary
  diagram so the reader can build mental model before reading
  prose.

## See also

- [`~/wart/CLAUDE.md`](../CLAUDE.md) — master setup, status
  table, repo layout, current task list.
- [`~/wart/post-art-roadmap.md`](../post-art-roadmap.md) —
  long-term direction (Hybrid §9 runtime model + beyond).
- [`~/wart/tasks/`](../tasks/) — per-task notes (scoping +
  results, e.g. `45-wart-zygote-spike.md`, `46-wart-arbiter-mvp.md`,
  `49-ime-content-control.md`).
