# Repository layout & naming conventions

This is the canonical reference for "where does a new thing
live?" and "what should I name it?" in the ~/wart tree. The
underlying rationale + research are in
[`tasks/52-monorepo-reorg.md`](../tasks/52-monorepo-reorg.md).

> **NOTE — current state, 2026-05-28**: the layout described
> below is the **target**, not yet executed. The repo is mid-
> reorganization. Existing dirs at the top level (`wart-host/`,
> `markdown-renderer/`, `wart-app/`, etc.) move to the new paths
> as part of task 52 step 3. Use this doc as the source-of-truth
> for where things *should* go going forward.

## Top-level categories

Five buckets at the root, plus meta dirs (docs, tasks,
CLAUDE.md). The bucket a thing belongs to is determined by what
it ships as, not what language it's written in.

```
~/wart/
├── apps/          # warpkgs — anything that lands on the device as a .warpkg
├── runtime/       # native Rust binaries — the host stack
├── wit/           # canonical WIT contracts (single source of truth)
├── external/      # vendored / forked upstream code
├── tools/         # build scripts, patches, dev diagnostics
└── repros/        # focused reproducers / smoke artifacts
```

### `apps/` — warpkgs

```
apps/
├── system/        # bundled with the runtime, installed at zygote startup
│   ├── war.markdown.renderer/   # cdylib system component
│   ├── war.emoji.picker/        # cdylib system component
│   ├── war.fonts.loader/        # cdylib system component
│   ├── war.ime.keyboard/        # Compose IME guest
│   └── lang/                    # IME language plugins (grouped)
│       ├── war.lang.bg/
│       └── war.lang.fr/
└── user/          # first-party user-facing apps
    └── wart-app/                # the Compose demo
```

A warpkg is anything that:
- Targets `wasm32-wasip2` (Rust) or `wasmWasi` (Kotlin).
- Exports a WIT world that the runtime knows how to instantiate.
- Gets packaged into a `.warpkg` directory by
  `tools/scripts/build-system-warpkgs.sh` or a sibling script.

**System vs user**: a system warpkg is owned by the runtime and
installed automatically by the launcher (Magisk module). A user
warpkg is something the user explicitly installs. The
`/data/local/tmp/wart-apps/{apps,system-apps}/` split on device
mirrors this directory split.

### `runtime/` — native binaries

```
runtime/
├── wart-host/         # Rust binary — wasmtime host + Compose render loop
├── wart-arbiter/      # Rust binary — policy daemon
└── magisk-module/     # init/, sepolicy, manifest; auto-starts the stack
```

The "native" stack: nothing here is a Wasm component. These are
the ARM64 binaries that actually run as Linux processes.

### `wit/` — canonical WIT

```
wit/
├── skiko-gfx.wit         # the runtime contract (Canvas, Paragraph, …)
├── ime.wit               # war:ime/ime — editor focus events
├── keyboard-lang.wit     # war:keyboard-lang/lang — plugin contract
├── markdown.wit          # war:markdown/renderer
├── emoji.wit             # war:emoji/picker
└── system-fonts.wit      # war:fonts/loader
```

These are the **single source of truth** for every WIT contract.
Each warpkg holds a mirror under its own `wit/deps/<pkg>/` (this
is a copy today; could become a symlink in a follow-up). When a
contract changes, edit it here, then sync mirrors.

### `external/` — vendored upstreams

```
external/
├── skiko/                          # ~/skiko fork (symlink or in-tree)
├── wasmtime-src/                   # wasmtime fork (own engine)
├── compose-multiplatform-core/     # Compose Multiplatform port
└── kotlin/                         # Kotlin build override (was `kotlin-src/`)
```

These are upstream-shaped and *kept* upstream-shaped — diffs
against the upstream tree should remain minimal so we can rebase.
This is the AOSP `external/` parallel.

### `tools/` — build + dev infrastructure

```
tools/
├── scripts/         # bash glue — build-system-warpkgs, pack-ime-keyboard, etc.
├── patches/         # upstream patches we apply locally
└── triage/          # diagnostic harnesses + upstream-issue artifacts
    └── wasmtime-issues/
```

### `repros/` — focused reproducers

```
repros/
├── wart-leak-repro/         # task 24 — Kotlin/Wasm suspend leak (now reattributed to wasmtime DRC)
├── kt-memalloc-repro/       # KT-86415 minimal repro
├── md-smoke-rust/           # task 36 — Rust CLI consumer smoke
└── wart-app-md-smoke/       # task 36 — Kotlin CLI consumer smoke
```

One-shot crates that demonstrate bugs or smoke specific
pipelines. Kept separate from production code so they don't
participate in `cargo build --workspace` etc.

## Naming conventions

| Class                                                | Pattern             | Examples                                       |
|------------------------------------------------------|---------------------|------------------------------------------------|
| Native binary (Rust, ships in /data/local/tmp)       | `wart-<kebab>`      | `wart-host`, `wart-arbiter`                    |
| Warpkg / app (anything that ends up as a `.warpkg`)  | `war.<dot-id>`      | `war.ime.keyboard`, `war.lang.bg`, `war.markdown.renderer` |
| Vendored fork / external                             | upstream name as-is | `skiko`, `wasmtime-src`, `compose-multiplatform-core` |
| Reproducer                                           | `<thing>-repro`     | `wart-leak-repro`, `kt-memalloc-repro`         |
| Smoke artifact                                       | `<thing>-smoke`     | `md-smoke-rust`, `wart-app-md-smoke`           |
| Tooling subdir                                       | descriptive lowercase | `scripts`, `patches`, `triage`               |

**Why two patterns** (`wart-*` for native, `war.*` for warpkgs)?
- The native binaries (host, arbiter) are part of the runtime
  infrastructure. They use `wart-` because they're tightly
  coupled to the runtime name.
- Warpkgs use `war.` (dot-separated) as a reverse-DNS-style
  namespace, matching their on-device `app_id` in
  `package.toml`. The directory name == the app_id makes
  install/launch debugging much easier.
- The single user app (`wart-app`) is grandfathered — it's the
  reference demo, deeply wired into many scripts. Renaming it
  would churn for no operational benefit.

## How to add a new thing

| You want to add…                                       | Put it in…                                           | Naming                                                     |
|--------------------------------------------------------|------------------------------------------------------|------------------------------------------------------------|
| A new lang plugin (e.g. German)                        | `apps/system/lang/war.lang.de/`                      | Mirror `war.lang.bg/` exactly                              |
| A new system component (e.g. icon-set provider)        | `apps/system/war.icons.provider/`                    | Pick an `app_id` matching the dir name                     |
| A new IME (e.g. voice)                                 | `apps/system/war.ime.voice/`                         | Sibling of `war.ime.keyboard/`                             |
| A new first-party demo app                             | `apps/user/<demo-name>/`                             | dash-separated kebab-case OR `war.<id>` if "first-party but distributed" |
| A new native daemon (e.g. crash reporter)              | `runtime/wart-<name>/`                               | `wart-crash`, `wart-trace`, …                              |
| A new WIT contract                                     | `wit/<pkg>.wit` + mirror to each consumer            | Package as `war:<id>@<sv>` for warpkg-facing, `my:<id>` for runtime-internal |
| A bug reproducer                                       | `repros/<thing>-repro/`                              | `-repro` suffix                                            |
| A smoke harness (one-shot consumer of a new dep)       | `repros/<thing>-smoke/`                              | `-smoke` suffix                                            |
| An upstream patch we apply locally                     | `tools/patches/<upstream>-<n>.patch`                 | Numeric ordering preserved                                 |
| Vendoring a new upstream repo                          | `external/<upstream-name>/`                          | Keep upstream's tree shape                                 |
| A build/dev script                                     | `tools/scripts/<name>.sh`                            | dash-separated kebab-case                                  |

## Cross-references

- [`tasks/52-monorepo-reorg.md`](../tasks/52-monorepo-reorg.md)
  — the full migration plan + research bibliography.
- [`docs/architecture-runtime.md`](architecture-runtime.md) —
  where the native binaries fit + their socket protocol.
- [`docs/architecture-ime.md`](architecture-ime.md) — where the
  IME + lang plugins fit + their plugin contract.
- [CLAUDE.md](../CLAUDE.md) "Repository layout" section — the
  legacy flat-layout view, retired once task 52 executes.
