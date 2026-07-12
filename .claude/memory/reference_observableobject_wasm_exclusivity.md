---
name: reference_observableobject_wasm_exclusivity
description: "OpenCombine ObservableObject/@Published/@ObservedObject DOES work on wasm (reactor incl.) — the crash was Swift dynamic exclusivity enforcement across export-call re-entry, NOT OpenCombine's lock. Fix = build ALL Swift modules with -enforce-exclusivity=unchecked."
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

**Verified 2026-07-12.** Real `ObservableObject` + `@Published` + `@ObservedObject`
(OpenCombine) reactivity WORKS on wasm — including the host reactor. This CORRECTS
the old stale claim (in the OpenSwiftUIDemo `GameLogic` comment) that it "corrupts
the Swift runtime's exclusivity state via a C++ UnfairLock, works only in a
command/main." That attribution was wrong.

Findings:
- **OpenCombine 0.15.1 already ships a `#if os(WASI)` NO-OP `UnfairLock`** (empty
  lock/unlock) — no `os_unfair_lock`, no C++. So the "lock corruption" mechanism
  cannot apply on `wasip1` (where `os(WASI)` is defined).
- **Command (bare wasmtime):** the minimal probe (`Counter: ObservableObject
  { @Published var n }` + `@ObservedObject`) fully re-renders on mutation
  (`n=1..5` → rendered N=1..5). Works out of the box. (NB: the 127 MB debug wasm
  takes ~90 s to JIT under wasmtime — don't mistake slow JIT for a hang.)
- **Reactor (wandr-host, repeated `onFrame` export calls):** WITHOUT the flag it
  crashed (`proc_exit(1)`) on the FIRST `@Published` mutation (the Swift fatal
  message is eaten by a "Broken pipe" in the host, so it looks like a silent
  ExitFailure(1)). Root cause = **Swift dynamic exclusivity enforcement** tripping
  across the reactor's cross-export-call re-entry.
- **THE FIX:** build with `-enforce-exclusivity=unchecked` applied to **ALL** Swift
  modules — crucially **OpenCombine** (where `objectWillChange`/`@Published` live),
  not just the app target. Evidence of the gradient: no flag → crash move 1;
  flag on app only (OpenCombine cached) → 6 moves; flag on every module → 41+
  moves, no crash (autoplay to score 2104, killed only by the test timeout).
- In `swift build`, `-Xswiftc -enforce-exclusivity=unchecked` passes to all
  targets, but a stale `.build` can leave OpenCombine compiled WITHOUT it — do a
  clean rebuild (or verify `Compiling OpenCombine …` appears) so the flag lands
  everywhere. It's a legitimate wasm-target setting (single-threaded → no runtime
  exclusivity checks needed), NOT an app modification.

Implication for "compile the real eleev/swiftui-2048 unmodified except Audio +
Store" ([[reference_swift_openswiftui_wandr]]): reactivity is a THIRD free
substitution — a **build flag**, not a source rewrite. `GameLogic` can stay real
`ObservableObject`. The gutted `WandrNoOpWillChange` + @State reactor hack is no
longer necessary. Remaining seams for the full app: `SwiftUI` shim module
(`@_exported import OpenSwiftUI`), `@AppStorage` (Store), `Audio`
(AudioServices→wasi:audio), `Image(systemName:)` (SF symbols).
