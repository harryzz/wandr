# Task 60 — unblock dioxus ≥0.7 for wasm32-wasip2 (the subsecond / wasm-bindgen wall)

> **Status:** ✅ adopted (option B) + device-verified on dioxus **0.7.9**,
> 2026-05-29. Upstream PR (option C) **submitted: [DioxusLabs/dioxus#5602](https://github.com/DioxusLabs/dioxus/pull/5602)**
> (from fork `harryzz/dioxus`, branch `wasip2-subsecond-cfg`; +2/−2; notes it was
> Claude-written + user-approved). Spun out of task 59 (dioxus-canvas), which
> originally had to pin dioxus 0.6.
>
> **Adopted (B):** vendored `external/subsecond/` (patched copy of upstream
> 0.7.9 + README) + `[patch.crates-io] subsecond = { path = … }` in
> `crates/dioxus-canvas` and `apps/user/wandr.dioxus.demo`; both bumped to dioxus
> **0.7**; `events.rs` got the 0.7 tweaks (`convert_cancel_data` + `Modifiers`
> import). Verified: `cargo test -p dioxus-canvas` (render + click) passes;
> `wandr.dioxus.demo` builds for `wasm32-wasip2` at **516 KB with zero
> wasm-bindgen**; reinstalled + relaunched on the Pixel 2 XL — **renders the
> full flexbox UI with host fonts** (measure-text resolves). On-device
> tap-to-increment for the 0.7 build is covered by the host test + the prior
> 0.6 on-device run (identical renderer); this session's tap didn't register
> because `QuickstepLauncher` held InputDispatcher focus (dumpsys-confirmed) —
> the known standalone focus issue ([[project-standalone-keys]]), orthogonal to
> this task.

## Why this matters

dioxus-canvas (task 59) is wandr's *light reactive UI framework* for system
components — the real alternative to Kotlin/Compose given (a) the unsolved
wasmtime DRC GC leak ([[wasmtime-drc-no-autoschedule]]) and (b) the slow
evolution of the Kotlin/Wasm `wasm32-wasip2` toolchain. Being **stuck on dioxus
0.6** is a liability: no upstream fixes, no new features, drift from the
ecosystem. This task removes the one thing that forces the 0.6 pin.

## Root cause (precise)

dioxus 0.7 won't build for `wasm32-wasip2`. It is **not** a "component model
unsupported" problem — dioxus knows nothing about the component model (we add
that with wit-bindgen + `wasm-component-ld`). The real chain:

- `dioxus-core 0.7` has a **non-optional** dep on **`subsecond`** (dioxus's
  hot-patch / "subsecond" hot-reload engine; `dioxus-core 0.7` exposes only a
  `serialize` feature — subsecond cannot be turned off).
- `subsecond`'s `Cargo.toml` declares its browser deps under
  `[target.'cfg(target_arch = "wasm32")'.dependencies]`: `wasm-bindgen`,
  `js-sys`, `web-sys`, `wasm-bindgen-futures`. That `cfg` matches **both**
  `wasm32-unknown-unknown` (browser) **and** `wasm32-wasip2` — even though the
  code is browser-only.
- The single consumer is `subsecond::apply_patch` (`src/lib.rs:550`), behind
  `#[cfg(target_arch = "wasm32")]`: it `spawn_local`s an async task that does
  `web_sys::window().fetch(patch_url)` → `WebAssembly.compile` → instantiate.
  Pure **dev-time browser hot-reload**; meaningless on wasip2 (no `window`, no
  JS `WebAssembly`).
- Result: our wasip2 build drags in wasm-bindgen's JS-host intrinsics
  (`__wbindgen_*`), which `wasm-component-ld` cannot resolve/lower → the guest
  link fails. No feature disables it.

`main` (latest dioxus, checked 2026-05-29) still uses the broad
`cfg(target_arch = "wasm32")` — **not fixed upstream**.

## The fix (clean target separation — proven)

Narrow subsecond's wasm `cfg` from `target_arch = "wasm32"` to
`all(target_arch = "wasm32", target_os = "unknown")`. This is exactly the
browser/non-browser split:

| target | `target_arch` | `target_os` | wants wasm-bindgen? |
|--------|---------------|-------------|---------------------|
| `wasm32-unknown-unknown` (browser) | wasm32 | **unknown** | yes (JS host) |
| `wasm32-wasip2` (us, component)    | wasm32 | **wasi**    | no |

Two edits in subsecond:
1. `Cargo.toml` — `[target.'cfg(target_arch = "wasm32")'.dependencies]` →
   `[target.'cfg(all(target_arch = "wasm32", target_os = "unknown"))'.dependencies]`
   (the 4 browser deps).
2. `src/lib.rs:550` — `#[cfg(target_arch = "wasm32")]` →
   `#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]`.

For wasip2, `apply_patch` then has neither its `#[cfg(any(unix, windows))]`
branch nor the wasm branch — it falls straight through to its trailing
`Ok(())` (`lib.rs:690`). i.e. **hot-patching is a no-op on wasip2**, which is
correct: we never hot-reload a packaged guest. `aslr_reference()` already has a
`#[cfg(target_family = "wasm")] return 0` path, so nothing else breaks.

### Proof (2026-05-29)

- Copied `subsecond-0.7.9` → `/tmp/subsecond-patched`, applied the 2 edits.
- Throwaway cdylib depending on `dioxus 0.7.9` (`default-features=false`,
  `macro,html,signals,hooks`) + `[patch.crates-io] subsecond = { path = … }`.
- `cargo build --target wasm32-wasip2 --release` → **links** (subsecond emits
  only dead-code warnings for its now-unused native internals).
- `cargo tree`: **no `wasm-bindgen` / `web-sys` / `js-sys`** anywhere — only the
  patched subsecond.
- `wasm-tools component wit` on the output → a **valid component** (216 KB).

The dioxus-canvas renderer core itself is 0.7-compatible: only `events.rs`
needs two trivial tweaks (a new `convert_cancel_data` method on the
`HtmlEventConverter` impl + import `Modifiers` from `dioxus_html::` instead of
`dioxus_html::prelude`). `WriteMutations`/`Template`/`Mutation`/`VirtualDom`
are unchanged across 0.6→0.7.

## Adoption options (pick one)

**A. Fork dioxus as a submodule (consistent with skiko/wasmtime/compose forks).**
Fork to codeberg, apply the 2-edit patch to `packages/subsecond/subsecond`, add
as `external/dioxus`, and in each dioxus-0.7 guest:
`[patch.crates-io] subsecond = { path = "../../../external/dioxus/packages/subsecond/subsecond" }`.
Tracks upstream; matches the repo's fork-as-submodule pattern. Heavy submodule
for a 5-line diff.

**B. Vendor just the subsecond crate (lean).** Copy the one crate to
`external/subsecond/` (or `tools/patches/subsecond/`) with a provenance README
+ the diff; `[patch.crates-io] subsecond = { path = … }` in each guest. ~960 LoC
+ Cargo.toml, self-contained; manual bumps to track upstream.

**C. Upstream PR + wait.** Submit the `cfg` narrowing to DioxusLabs/dioxus
(small, obviously-correct, benefits all wasip2/WASI users). Best long-term;
doesn't unblock us until released — pair with A or B meanwhile.

Recommended: **B now** (leanest unblock) + **C** in parallel (so we can drop the
vendor once it lands upstream). `[patch]` is per-build-root and our guests are
standalone crates, so the patch line lives in each dioxus guest's Cargo.toml
(today: `apps/user/wandr.dioxus.demo` + the `crates/dioxus-canvas` dev-dep build).

## Migration steps (when adopted)

1. Land the patched subsecond per the chosen option above.
2. Bump `crates/dioxus-canvas` + `apps/user/wandr.dioxus.demo` deps 0.6 → 0.7;
   add the `[patch.crates-io] subsecond` line to the guest(s).
3. Re-apply the `events.rs` 0.7 tweaks (`convert_cancel_data` + `Modifiers`
   import — both were already validated during this investigation).
4. `cargo test -p dioxus-canvas` (host) — render + click pipeline.
5. Rebuild the demo wandrpkg, reinstall, relaunch via the arbiter, re-screenshot
   (host unchanged — no wandr-host rebuild needed; guest-only change).
6. Remove the "PINNED to 0.6" comment in `crates/dioxus-canvas/Cargo.toml`.

## Related

- `tasks/59-dioxus-canvas-renderer.md` — the renderer this unblocks; its
  "Dependency / version notes" records the 0.6 pin.
- [[reference_dioxus_taffy_rust_ui]], [[wasmtime-drc-no-autoschedule]].
