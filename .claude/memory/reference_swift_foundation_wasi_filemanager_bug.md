---
name: reference_swift_foundation_wasi_filemanager_bug
description: "RESOLVED UPSTREAM (PR #2134, merged 2026-08-04): swift-foundation #2120 — FileManager.contents(atPath:)/Data(contentsOfFile:) returned empty Data on WASI when fstat size=0. Fix on main, not yet in a tagged SDK release; workaround still fine to keep."
metadata: 
  node_type: memory
  type: reference
  originSessionId: efb9ba77-bb47-4ab5-bbac-3dcd59e2771e
  modified: 2026-08-05T05:18:59.277Z
---

**✅ RESOLVED UPSTREAM 2026-08-04** — PR https://github.com/swiftlang/swift-foundation/pull/2134
added `os(WASI)` to the `os(Linux) || os(Android)` chunked-read fallback guard (exactly the
root cause diagnosed below), merged to swift-foundation `main`. NOT yet in a tagged Swift
toolchain/SDK release as of 2026-08-04 (pinned SDK here is `swift-6.3.2-RELEASE_wasm`). When the
pin bumps to a release that includes #2134, the `FileManager.contents(atPath:)`/`Data(contentsOfFile:)`
workaround is no longer necessary — but there's NO urgency to switch back (the POSIX `readAsset`
path already works), so this is informational, not an action item.

**Filed 2026-07-17**: https://github.com/swiftlang/swift-foundation/issues/2120

`Data(contentsOfFile:)` (and `FileManager.contents(atPath:)`, which delegates to it) silently
returns empty, non-nil `Data` on `wasm32-unknown-wasip1` when `fstat`'s `st_size` reports `0` for
a real, non-empty file — confirmed happens for files read from a wasmtime component-model guest's
preopened directory (e.g. `/assets`). Root cause: `Data+Reading.swift`'s zero-size fast path only
does a real fallback chunked-read for `#if os(Linux) || os(Android)`; `os(WASI)` isn't included,
so it falls into the `#else` branch and returns empty data without attempting a real read.

**Why:** discovered while deciding whether eleev's original (excluded)
`Utils/Plist/PlistConfiguration.swift` could be un-excluded now that `Bundle.main` has a working
shim (`swift/apple-compat/Sources/SwiftUI/Bundle.swift`) — that file also calls
`FileManager.default.contents(atPath:)`, which turned out to have this separate, silent-failure
bug. Confirmed via an isolated temporary diagnostic in `WandrReactor.swift` (added, tested,
removed — see commit history around 2026-07-17/18), not guessed.

**How to apply:** `swift/apple-compat/Sources/SwiftUI/PlistConfiguration.swift`'s own doc comment
and `repros/swift-canvas-spike/Package.swift`'s exclude-list comment both reference this issue —
that's why `Utils/Plist/PlistConfiguration.swift` stays excluded and apple-compat's own POSIX
`readAsset` (not `FileManager`) is used instead. **Periodically check issue #2120** — if/when
upstream fixes it (or bumps the pinned SDK past whatever version includes the fix), there's no
urgent need to switch back to `FileManager` (the POSIX version already works fine), but it's
worth knowing the workaround is no longer strictly necessary. Also relevant: the pinned SDK here
is `swift-6.3.2-RELEASE_wasm`; a newer `swift-6.3.3-RELEASE` exists (checked 2026-07-17) but does
NOT include a fix for this (confirmed against swift-foundation `main` at the time of filing).
