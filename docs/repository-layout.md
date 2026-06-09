# Repository layout & naming conventions

This is the canonical reference for "where does a new thing
live?" and "what should I name it?" in the ~/wandr tree. The
underlying rationale + research are in
[`tasks/52-monorepo-reorg.md`](../tasks/52-monorepo-reorg.md).

> **NOTE — current state, 2026-05-28**: the layout described
> below is the **target**, not yet executed. The repo is mid-
> reorganization. Existing dirs at the top level (`wandr-host/`,
> `markdown-renderer/`, `wandr-app/`, etc.) are still **separate
> git repos** sharing the wandr working directory; task 52
> reshapes the directory tree, task 53 collapses the sibling
> repos into a single monorepo via `git subtree` and adds
> submodules for the four vendored forks. Use this doc as the
> source-of-truth for where things *should* go going forward.

## Top-level categories

Buckets at the root, plus meta dirs (docs, tasks, CLAUDE.md). The
bucket a thing belongs to is determined by what it ships as, not
what language it's written in.

```
~/wandr/
├── apps/          # wandrpkgs — anything that lands on the device as a .wandrpkg
├── runtime/       # native Rust binaries — the host stack
├── crates/        # shared guest-side Rust libraries (compiled INTO wandrpkgs)
├── wit/           # canonical WIT contracts (single source of truth)
├── external/      # vendored / forked upstream code
├── tools/         # build scripts, patches, dev diagnostics
└── repros/        # focused reproducers / smoke artifacts
```

### `apps/` — wandrpkgs

```
apps/
├── system/        # bundled with the runtime, installed at zygote startup
│   ├── wandr.markdown.renderer/   # cdylib system component
│   ├── wandr.emoji.picker/        # cdylib system component
│   ├── wandr.fonts.loader/        # cdylib system component
│   ├── wandr.ime.keyboard/        # Compose IME guest
│   └── lang/                    # IME language plugins (grouped)
│       ├── wandr.lang.bg/
│       └── wandr.lang.fr/
└── user/          # first-party user-facing apps
    ├── wandr-app/                # the Compose demo
    └── wandr.dioxus.demo/         # reactive dioxus guest (task 59)
```

A wandrpkg is anything that:
- Targets `wasm32-wasip2` (Rust) or `wasmWasi` (Kotlin).
- Exports a WIT world that the runtime knows how to instantiate.
- Gets packaged into a `.wandrpkg` directory by
  `tools/scripts/build-system-wandrpkgs.sh` or a sibling script.

**Manifest**: every app dir owns a **`package.toml`** at its root — the
single source of truth for its `app_id` / `version` / `world` / `kind`
(app vs system) / `composition` / `orientation` / `label` / `[components]`
/ `[dependencies]`. The pack scripts (`build-system-wandrpkgs.sh`,
`pack-ime-keyboard.sh`) **copy** this file into the `.wandrpkg` verbatim —
they do not generate it. Edit the app's `package.toml` like any file; the
3rd arg to `pack_wandrpkg` is the component name and must match the toml's
`[components]` entry. (Before, these were heredocs inside the pack script —
moved into the app dirs so they're not regenerated on every pack.) The
`orientation` field (`"auto" | "locked"`, default locked) drives task-62/63
rotation: e.g. `wandr.launcher` is `locked` (home stays portrait + locks the
chrome), the bars + IME + user apps are `auto`. It is NOT part of the AOT
cache-key, so editing it doesn't invalidate the precompiled `.cwasm`.

**System vs user**: a system wandrpkg is owned by the runtime and
installed automatically by the launcher (Magisk module). A user
wandrpkg is something the user explicitly installs. The
`/data/local/tmp/wandr-apps/{apps,system-apps}/` split on device
mirrors this directory split.

### `runtime/` — native binaries

```
runtime/
├── wandr-host/         # Rust binary — wasmtime host + Compose render loop
├── wandr-arbiter/      # Rust binary — policy daemon
└── magisk-module/     # init/, sepolicy, manifest; auto-starts the stack
```

The "native" stack: nothing here is a Wasm component. These are
the ARM64 binaries that actually run as Linux processes.

### `crates/` — shared guest-side Rust libraries

```
crates/
└── dioxus-canvas/    # "tiny Blitz": dioxus VirtualDom → taffy → canvas WIT
```

Library crates (not `cdylib`s themselves) that compile **into** guest
wandrpkgs as a normal path dependency. Unlike `runtime/` (host binaries) or
`apps/` (shippable wandrpkgs), a `crates/` lib never ships on its own — it's
reusable guest framework code. `dioxus-canvas` is the reactive-UI renderer
that drives the canvas WIT from a dioxus app; `wandr.dioxus.demo` (and future
rich Rust guests) path-depend on it. Kept WIT-agnostic via a `CanvasSink`
trait so it stays host-testable; the consuming wandrpkg owns the trimmed WIT.

### `wit/` — canonical WIT

```
wit/
├── skiko-gfx.wit         # the runtime contract (Canvas, Paragraph, …)
├── ime.wit               # wandr:ime/ime — editor focus events
├── keyboard-lang.wit     # wandr:keyboard-lang/lang — plugin contract
├── markdown.wit          # wandr:markdown/renderer
├── emoji.wit             # wandr:emoji/picker
└── system-fonts.wit      # wandr:fonts/loader
```

These are the **single source of truth** for every WIT contract.
Each wandrpkg holds a mirror under its own `wit/deps/<pkg>/` (this
is a copy today; could become a symlink in a follow-up). When a
contract changes, edit it here, then sync mirrors.

### `external/` — vendored upstreams (git submodules)

```
external/
├── skiko/                          # codeberg.org/harryzz/skiko (fork)
├── wasmtime/                       # codeberg.org/harryzz/wasmtime (fork — was wasmtime-src/)
├── compose-multiplatform-core/     # codeberg.org/harryzz/compose-multiplatform-core (fork)
└── kotlin/                         # codeberg.org/harryzz/kotlin (build override)
```

Each is a **git submodule** pointing at our codeberg fork. The
fork tracks upstream + carries any local patches (e.g.
`external/wasmtime/` carries the KT-86415 adapter-State fix).
Bumping a fork is one explicit operation:

```
cd external/wasmtime
git pull origin main       # or rebase against upstream + push
cd ../..
git add external/wasmtime
git commit -m "external/wasmtime: bump to <sha>"
```

Cloning the monorepo:

```
git clone --recurse-submodules https://codeberg.org/harryzz/wart.git
# or, after a non-recursive clone:
git submodule update --init --recursive
```

This is the AOSP `external/` parallel: upstream-shaped and *kept*
upstream-shaped so rebases are clean.

### `tools/` — build + dev infrastructure

```
tools/
├── scripts/         # bash glue — build-system-wandrpkgs, pack-ime-keyboard, etc.
├── patches/         # upstream patches we apply locally
└── triage/          # diagnostic harnesses + upstream-issue artifacts
    └── wasmtime-issues/
```

### `repros/` — focused reproducers

```
repros/
├── wandr-leak-repro/         # task 24 — Kotlin/Wasm suspend leak (now reattributed to wasmtime DRC)
├── kt-memalloc-repro/       # KT-86415 minimal repro
├── md-smoke-rust/           # task 36 — Rust CLI consumer smoke
└── wandr-app-md-smoke/       # task 36 — Kotlin CLI consumer smoke
```

One-shot crates that demonstrate bugs or smoke specific
pipelines. Kept separate from production code so they don't
participate in `cargo build --workspace` etc.

## Naming conventions

| Class                                                | Pattern             | Examples                                       |
|------------------------------------------------------|---------------------|------------------------------------------------|
| Native binary (Rust, ships in /data/local/tmp)       | `wandr-<kebab>`      | `wandr-host`, `wandr-arbiter`                    |
| Warpkg / app (anything that ends up as a `.wandrpkg`)  | `wandr.<dot-id>`      | `wandr.ime.keyboard`, `wandr.lang.bg`, `wandr.markdown.renderer` |
| Vendored fork / external                             | upstream name, no `-src` suffix | `skiko`, `wasmtime`, `compose-multiplatform-core`, `kotlin` |
| Reproducer                                           | `<thing>-repro`     | `wandr-leak-repro`, `kt-memalloc-repro`         |
| Smoke artifact                                       | `<thing>-smoke`     | `md-smoke-rust`, `wandr-app-md-smoke`           |
| Tooling subdir                                       | descriptive lowercase | `scripts`, `patches`, `triage`               |

**Why two patterns** (`wandr-*` for native, `wandr.*` for wandrpkgs)?
- The native binaries (host, arbiter) are part of the runtime
  infrastructure. They use `wandr-` because they're tightly
  coupled to the runtime name.
- Warpkgs use `wandr.` (dot-separated) as a reverse-DNS-style
  namespace, matching their on-device `app_id` in
  `package.toml`. The directory name == the app_id makes
  install/launch debugging much easier.
- The single user app (`wandr-app`) is grandfathered — it's the
  reference demo, deeply wired into many scripts. Renaming it
  would churn for no operational benefit.

## How to add a new thing

| You want to add…                                       | Put it in…                                           | Naming                                                     |
|--------------------------------------------------------|------------------------------------------------------|------------------------------------------------------------|
| A new lang plugin (e.g. German)                        | `apps/system/lang/wandr.lang.de/`                      | Mirror `wandr.lang.bg/` exactly                              |
| A new system component (e.g. icon-set provider)        | `apps/system/wandr.icons.provider/`                    | Pick an `app_id` matching the dir name                     |
| A wandrpkg's manifest                                    | `apps/.../<app>/package.toml`                        | One per app dir; pack scripts copy it (don't add a heredoc)|
| A new IME (e.g. voice)                                 | `apps/system/wandr.ime.voice/`                         | Sibling of `wandr.ime.keyboard/`                             |
| A new first-party demo app                             | `apps/user/<demo-name>/`                             | dash-separated kebab-case OR `wandr.<id>` if "first-party but distributed" |
| A new native daemon (e.g. crash reporter)              | `runtime/wandr-<name>/`                               | `wandr-crash`, `wandr-trace`, …                              |
| A new WIT contract                                     | `wit/<pkg>.wit` + mirror to each consumer            | Package as `wandr:<id>@<sv>` for wandrpkg-facing, `my:<id>` for runtime-internal |
| A bug reproducer                                       | `repros/<thing>-repro/`                              | `-repro` suffix                                            |
| A smoke harness (one-shot consumer of a new dep)       | `repros/<thing>-smoke/`                              | `-smoke` suffix                                            |
| An upstream patch we apply locally                     | `tools/patches/<upstream>-<n>.patch`                 | Numeric ordering preserved                                 |
| Vendoring a new upstream repo                          | `external/<upstream-name>/`                          | Keep upstream's tree shape                                 |
| A build/dev script                                     | `tools/scripts/<name>.sh`                            | dash-separated kebab-case                                  |

## Git organization

**Hybrid model**: single monorepo for everything in
`apps/`, `runtime/`, `wit/`, `tools/`, `repros/`, `docs/`,
`tasks/`. Submodules for the four upstream forks under
`external/`.

Why hybrid:
- First-party code co-evolves at WIT boundaries. One commit
  beats N. The current N-repo arrangement made the
  task-49-step-5 plugin shipment take 4 separate commits across
  4 repos that couldn't be bisected together.
- Shared `Cargo.lock` dedups Rust deps across host, arbiter,
  and every system wandrpkg.
- External forks are slow-moving + huge — submodules amortize
  the size cost; bumping a fork is one explicit operation.

## Cross-references

- [`tasks/52-monorepo-reorg.md`](../tasks/52-monorepo-reorg.md)
  — directory shape + naming conventions (the *why*).
- [`tasks/53-monorepo-merge.md`](../tasks/53-monorepo-merge.md)
  — the git-side merge plan: subtree-import 13 sibling repos,
  wire 4 submodules, archive the obsolete codeberg repos.
- [`docs/architecture-runtime.md`](architecture-runtime.md) —
  where the native binaries fit + their socket protocol.
- [`docs/architecture-ime.md`](architecture-ime.md) — where the
  IME + lang plugins fit + their plugin contract.
- [CLAUDE.md](../CLAUDE.md) "Repository layout" section — the
  legacy flat-layout view, retired once task 52 + 53 execute.
