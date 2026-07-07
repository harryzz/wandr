# Shared runtimes & app size — why apps are big, and how (not) to share a framework

> **TL;DR.** Every wandr app statically bundles its whole UI framework — a
> Compose / SwiftUI / Avalonia app `.wasm` is tens of MB (measured: Avalonia
> 40 MB, Swift/OpenSwiftUI 65 MB → 191 MB AOT). You **cannot** publish the big
> framework once as a Component-Model library that thin apps "link against":
> the app↔framework boundary isn't WIT-shaped, and the toolchains whole-program-
> compile the framework into each app. The realistic way to share a heavy
> runtime in wandr is the **Android model — a framework-preloaded zygote that
> apps `fork()` from, COW-sharing its code pages in RAM.** That shares the
> runtime at *runtime* (memory), not on *disk*. Thin frameworks whose API fits a
> WIT seam (dioxus / Slint over `wasi:canvas`) *can* be shared as components;
> entangled ones (Compose / SwiftUI) cannot.

## Two costs, two different solutions

"Share the framework" conflates two independent axes. Keep them separate:

```
                        can it be shared?     mechanism
  ┌─────────────────┐
  │ RUNTIME (RAM)   │   YES (wandr's model)   zygote COW fork
  │ ~180 MB/app WS  │                         (preload once, fork per app)
  ├─────────────────┤
  │ DISK / DISTRIB. │   NOT TODAY             needs a separable framework
  │ 40–190 MB/.wasm │                         module the toolchain won't emit
  └─────────────────┘
```

- **Runtime memory** — many apps running Compose shouldn't each pay Compose's
  RAM cost. Solvable by sharing *code pages* across processes.
- **On-disk / distribution size** — each app package shouldn't re-ship the whole
  framework. Solvable only by *not inlining* the framework into each `.wasm`.

They look like one problem ("Compose is huge") but have different fixes, and
only the first is reachable today.

## Why you can't publish the framework as a linked shared component

The appealing idea: publish Compose once as a component on the device; ship each
app as a tiny component that links the already-published Compose. **This does not
work for a heavy UI framework**, for two independent reasons:

1. **The app↔framework boundary isn't expressible as WIT.** Components link at
   the WIT canonical-ABI seam — coarse functions, flat/handle types. An app uses
   Compose through rich *Kotlin-language* API: `@Composable` functions (compiler-
   plugin-rewritten), generics, lambdas/closures, coroutines, deep object graphs.
   None of that crosses a WIT boundary. You can cut `app | wasi:canvas host` at a
   component seam; you cannot cut `app | Compose` there — wrong shape, wrong
   granularity.
2. **The toolchain whole-program-compiles the framework into the app.**
   Kotlin/Wasm emits **one WasmGC module** = app + Compose + Material + skiko
   bindings, all linked. .NET NativeAOT (Avalonia) and Swift do the same into one
   module. There is **no separate-compilation / dynamic-linking path** that emits
   the framework once as a shared module for thin apps to reference. The Wasm
   "shared-everything linking" and component "shared libraries" proposals exist
   but are immature — and Kotlin/Wasm doesn't target them.

So both the *interface* and the *toolchain* are against it. This isn't a wandr
gap; it's where WasmGC toolchains are in 2026.

## What wandr *can* do: the framework-preloaded zygote (the Android model)

wandr's Hybrid runtime is explicitly modeled on Android's zygote: **preload
once, `fork()` per app, COW-share read-only pages** (`docs/architecture-runtime.md`).
This is the correct mechanism for sharing a heavy runtime — as *memory*, not as
linking.

**Where it stands today (measured):** the zygote currently COW-shares the
**native host** stack (wasmtime `Engine`, skia, AOT caches, fonts), giving only
**~5.6 MB Shared_Dirty per child**; the **~180 MB per-app working set dominates**
(`docs/architecture-runtime.md:373`, `project_app_lifecycle_and_packaging`). It
does **not** yet share the *guest's* Compose runtime — each child instantiates
its own.

**The upgrade that would actually amortize Compose:** a **framework-base
zygote** — a zygote image with the Compose runtime *instantiated* — so children
`fork()` from it and COW-share Compose's code pages. This is exactly how Android's
ART zygote preloads the framework and apps fork it. It's listed as future work
("grow COW savings to ~25–40 MB"). This shares Compose in **RAM across running
apps**; it does **not** shrink the per-app `.wasm` on disk (each still bundles the
framework).

Disk-size dedup (content-address the framework blob, have app packages reference
it) needs the toolchain to emit a *separable* framework module — which loops back
to the missing dynamic-linking support. So on-disk sharing stays blocked upstream.

## The exception: thin frameworks over a WIT seam *are* shareable

Shareability scales **inversely with how entangled the framework's API is**:

| Framework | App↔lib boundary | Shareable as a component? | App `.wasm` |
|---|---|---|---|
| Compose, SwiftUI, Avalonia | rich language API (composables, generics, plugins) | **No** — not a WIT seam; whole-program compiled | 40–65 MB |
| **dioxus / Slint over `wasi:canvas`** | thin renderer → `wasi:canvas` WIT | **Yes** — the renderer is the shareable layer | far smaller |

dioxus-canvas and slint-wandr already draw through `wasi:canvas`, so the heavy
part (the renderer) lives host-side behind a WIT boundary and isn't re-bundled
per app. That's the architecture that *does* get you "publish the shared part
once, ship thin apps" — but it only applies to frameworks whose surface can be
narrowed to a WIT contract.

## Composed components & the wasip3 shared event loop (guest side)

Sharing/reusing components raises a second question: when one component depends
on another, how do they interact *concurrently*? WASI 0.3 / Component-Model async
(wasip3) answers this with **one host-owned event loop**, and it's worth being
precise about what that means **from a guest's point of view**.

**The guest stops owning an executor.** In wasip2 a guest that wanted concurrency
drove `wasi:io/poll` itself — wandr's `wandr-step-executor` is exactly that, a
guest-side executor built to keep async alive across the host's per-frame export
calls (the documented *"step-executor doesn't span component calls"* constraint).
In wasip3 the guest just writes `async fn` + `.await`; **awaiting yields to the
runtime**, which resumes the task when the awaited value is ready. No poll loop,
no continuation stashing, and **async now spans the export call** — which is the
whole reason the step-executor exists, removed (for Rust guests).

**The loop spans guest↔guest, not just guest↔host** — that's the point of
"shared." When guest A awaits an async import from guest B, A yields to the
*runtime* (not to B); B runs (maybe awaiting host wasi or a third guest C); when
B's `future<T>`/`stream<T>` resolves the runtime resumes A. A value can cross
A→B→C boundaries and the one loop schedules whoever is waiting.

**But "one loop" is scoped to one component graph / one Store — NOT device-wide:**

```
  ┌────────── ONE app process (one Store / one embedder) ──────────┐
  │  guest A ──async──▶ guest B (composed dep) ──▶ host wasi        │
  │        └──────── all on the SAME shared loop ────────┘         │
  └────────────────────────────────────────────────────────────────┘

  ┌── app X (process) ──┐   IPC   ┌── app Y (process) ──┐
  │  its own loop        │ ◀─────▶ │  its own loop        │
  └──────────────────────┘         └──────────────────────┘
      separate embedders — talk via arbiter/event bus, NOT one loop
```

- **Two components composed into one app** (app + dep, wired by `link.wac` /
  `wire_dep_into_linker`, which instantiates the dep **into the consumer's
  Store**, `app_loader.rs:876`) → same graph, **same loop**. ✓
- **Two separate apps** (each zygote-forked into its own process) → **separate
  loops**; they use wandr IPC (arbiter sockets, event bus), and the shared-loop
  model does not apply. ✗

The deciding factor is **one Store / one instance graph**, not "is it a guest."

**wandr status:** composed deps already live in the consumer's Store (so they'd
share the loop), but the dep wiring is deliberately **sync-only today** —
*"Resources, top-level exported functions, and async dispatch are out of scope"*
(`app_loader.rs:858`), per-call `Val`-boxed at ~1 Hz. So wandr's cross-component
calls don't use the shared async loop yet; **wasip3 async is the upgrade** that
turns "composed dep, calls must be synchronous" into "dep exposes
`async`/`stream`/`future`, and the caller awaits it on the one loop."

**Caveats (guest side):** the loop is single-threaded and cooperative — a guest
that does CPU work without awaiting **blocks every task on the loop**, so yield at
suspension points. Rendering stays **host-pull / synchronous / frame-paced** (the
`on_frame` export); async is for the **I/O side** (networking, media, events)
running as cooperative tasks around frames. And this is **Rust-guests only**
today — a Kotlin/Compose component can't participate natively (KT-64568), so it
stays on the reactor/step-executor side even when composed with a Rust guest.

## Practical guidance

- **Don't** promise "Compose as a shared linked component" — it's blocked at both
  the interface and toolchain levels.
- **Do** treat the **framework-base zygote** as the path to sharing heavy
  runtimes' *memory* across apps (Android-style); it's already the architecture,
  and needs the framework-image preload work to pay off.
- **Prefer** WIT-narrow frameworks (dioxus / Slint / anything drawing through
  `wasi:canvas`) when small app size + genuine shared-runtime reuse matter.
- **Track upstream** WasmGC shared-everything / dynamic-linking and the component
  "shared libraries" proposals — landing those is the only route to on-disk
  framework dedup.

## See also

- [`architecture-runtime.md`](architecture-runtime.md) — the zygote / arbiter /
  host model + the COW numbers.
- [`overview.md`](overview.md) — the layer model; frameworks vs the portable
  contract layer.
- `[[project_app_lifecycle_and_packaging]]`, `[[feedback_wandr_zygote_fork_survival]]`
  — the packaging + fork-survival memories.
