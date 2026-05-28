# Task 52 — Monorepo reorganization & naming unification

> Status: 🔲 scoped, not started — 2026-05-28
>
> The ~/wart tree has grown organically over 50+ tasks. There are
> now 30+ top-level entries with no visible category boundary
> between native runtime, system warpkgs, user apps, vendored
> forks, reproducers, and meta directories. This task proposes a
> reorganization grounded in cross-industry precedent (AOSP,
> Cargo, wasmCloud, monorepo conventions) and a one-shot
> migration plan.

## Why now

Before the third lang plugin / fourth system component / fifth
demo app gets added we should pick conventions, because:

- **Categories are blurred.** `markdown-renderer/`, `wart-app/`,
  `wart-host/`, `wart-leak-repro/`, `wasmtime-src/` all sit at
  the same level — but they're a system component, a user app,
  a runtime binary, a one-shot reproducer, and a vendored fork
  respectively. A reader can't tell from `ls`.
- **Naming is inconsistent.** `wart-host` (dash) coexists with
  `war.lang.bg` (dot), with `wart-stack-magisk` (dash) — both for
  things shipped to the device. The session-7 user choice
  ("war.lang.xx for plugins, leave wart-host alone") is a working
  rule but isn't documented.
- **New things have nowhere obvious to land.** Where does a
  shared Kotlin helper library go? A non-Compose warpkg utility?
  An on-device sysprop daemon? Today: top-level. Tomorrow: not
  scalable.

## Research distilled

Four precedents that shaped the proposal — sources at the end.

### AOSP (the closest architectural cousin)

AOSP separates concerns into top-level categories that mirror
ours:

- **`frameworks/`** — core APIs (Java + native services).
  Parallel: our `runtime/wart-host/` (Rust host + canvas /
  paragraph / keyboard / etc. WIT impls).
- **`system/`** — low-level libs + init + sepolicy.
  Parallel: our zygote + arbiter + Magisk module.
- **`packages/`** — bundled-with-OS apps (Phone, Contacts,
  Settings). Parallel: our system warpkgs (markdown-renderer,
  emoji-picker, war.ime.keyboard, war.lang.*).
- **`hardware/`** — HALs. Parallel: our libgui shim (the only
  C++ piece). Could fold into runtime/.
- **`external/`** — third-party / vendored source.
  Parallel: our skiko, wasmtime-src, compose-multiplatform-core
  forks.

### Cargo workspaces (the multi-crate native pattern)

For repos with 10K–1M lines of code, rust-analyzer-style flat
layout is the consensus: one `crates/` directory at the root.
Published crates split off into `libs/` to enforce a "no
upward dep" boundary. ([matklad blog][1] is the canonical
write-up.) Our equivalent is: native binaries (wart-host,
wart-arbiter) under one umbrella; system warpkgs under another.

### wasmCloud (the multi-component WCM stack)

wasmCloud's host is a single binary; its "providers" (host
extensions) are separately built artifacts; its "components" are
the polyglot Wasm payloads. The host repo is single-purpose;
component templates live in separate repos. The cross-cutting
contract is in a `wit/` dir at the root of every artifact.

Takeaway: the **WIT contract gets its own top-level location**,
each consumer mirrors it under `wit/deps/`. We already do this.

### Polyglot monorepo conventions

Industry advice converges on: **apps/** (user-facing units),
**packages/** or **libs/** (shared building blocks),
**tools/** (build & dev infrastructure), **external/** or
**vendor/** (third-party). Avoid mixing unrelated artifacts in
one directory; be explicit about ownership boundaries.

## Proposed layout

```
~/wart/
├── apps/                           # warpkgs shipped to device
│   ├── system/                     # bundled with the runtime
│   │   ├── markdown-renderer/
│   │   ├── emoji-picker/
│   │   ├── system-fonts/
│   │   ├── war.ime.keyboard/       # the IME (system but Compose-shaped)
│   │   └── lang/                   # IME language plugins
│   │       ├── war.lang.bg/
│   │       └── war.lang.fr/
│   └── user/                       # first-party reference apps
│       └── wart-app/               # the Compose demo
│
├── runtime/                        # native Rust binaries
│   ├── wart-host/
│   ├── wart-arbiter/
│   └── magisk-module/              # wart-stack-magisk/, renamed
│
├── wit/                            # canonical WIT — single source of truth
│   ├── skiko-gfx.wit
│   ├── ime.wit
│   ├── keyboard-lang.wit
│   ├── markdown.wit
│   ├── emoji.wit
│   └── system-fonts.wit
│
├── external/                       # vendored / forked upstreams (huge)
│   ├── skiko/                      # was ~/wart/skiko/
│   ├── wasmtime/                   # was ~/wart/wasmtime-src/, suffix dropped
│   ├── compose-multiplatform-core/
│   └── kotlin/                     # was ~/wart/kotlin-src/, suffix dropped
│
├── tools/                          # build helpers + diagnostic harnesses
│   ├── scripts/                    # existing scripts/
│   ├── patches/                    # existing patches/
│   └── triage/                     # wasmtime-issue-artifacts/, renamed
│
├── repros/                         # focused reproducers
│   ├── wart-leak-repro/
│   ├── kt-memalloc-repro/
│   ├── md-smoke-rust/
│   └── wart-app-md-smoke/
│
├── tasks/                          # task narrative (unchanged)
├── docs/                           # architecture docs (unchanged)
├── CLAUDE.md
├── README.md                       # NEW — top-level orientation page
├── post-art-roadmap.md
├── .gitignore                      # updated for new paths
└── .claude/, memory/, .git/        # meta
```

### Naming convention (locked in by this task)

| Class                                    | Pattern             | Example                |
|------------------------------------------|---------------------|------------------------|
| Native binary (Rust, ship in /data/local/tmp) | `wart-<kebab>` | `wart-host`, `wart-arbiter` |
| Warpkg / app (Compose or cdylib, ships as `.warpkg`) | `war.<dot-id>` | `war.ime.keyboard`, `war.lang.bg`, `war.markdown.renderer` |
| Vendored fork / external                 | upstream name, no `-src` suffix | `skiko`, `wasmtime`, `compose-multiplatform-core`, `kotlin` |
| Repro / dev artifact                     | `<thing>-repro` or `<thing>-smoke` | `wart-leak-repro`, `md-smoke-rust` |
| Tooling subdir                           | descriptive         | `scripts`, `patches`, `triage` |

Note: the existing `markdown-renderer/`, `emoji-picker/`, and
`system-fonts/` directories use dash-separated descriptive names
even though they're warpkgs. They predate the war.* convention.
**Migration: rename to `war.markdown.renderer/`, `war.emoji.picker/`,
`war.fonts.loader/`** so the directory matches the app_id. Saves
the next reader a head-scratch about why one warpkg has a `war.`
prefix and another doesn't.

### Aliases / shortcuts to keep working

- `wart-app` (the demo) stays — it's a Cargo/Gradle project name
  that's deeply wired. Move under `apps/user/wart-app/` but keep
  the directory name.
- The wart repo itself is the **outer** monorepo containing apps,
  runtime, docs, etc. — keep `~/wart` as the parent name.

## Migration plan

Eight steps, each small + reversible.

### Step 1 — Decide & document conventions (~30 min)

This task doc + a new `docs/repository-layout.md` that becomes
the canonical layout reference. Cross-link from CLAUDE.md.

### Step 2 — Create the new top-level dirs (~5 min)

```
mkdir -p apps/system apps/system/lang apps/user runtime external tools repros
```

Empty; no moves yet.

### Step 3 — Move dirs (~30 min)

`git mv` (for tracked dirs) and plain `mv` (for the gitignored
sibling repos). Each sibling repo retains its own `.git/` and
codeberg origin. Specifically:

| From                          | To                                  |
|-------------------------------|-------------------------------------|
| `wart-host/`                  | `runtime/wart-host/`                |
| `wart-arbiter/`               | `runtime/wart-arbiter/`             |
| `wart-stack-magisk/`          | `runtime/magisk-module/`            |
| `wart-app/`                   | `apps/user/wart-app/`               |
| `war.ime.keyboard/`           | `apps/system/war.ime.keyboard/`     |
| `markdown-renderer/`          | `apps/system/war.markdown.renderer/`|
| `emoji-picker/`               | `apps/system/war.emoji.picker/`     |
| `system-fonts/`               | `apps/system/war.fonts.loader/`     |
| `war.lang.bg/`                | `apps/system/lang/war.lang.bg/`     |
| `war.lang.fr/`                | `apps/system/lang/war.lang.fr/`     |
| `skiko/`                      | `external/skiko/` (still a symlink) |
| `wasmtime-src/`               | `external/wasmtime/`                |
| `compose-multiplatform-core/` | `external/compose-multiplatform-core/`|
| `kotlin-src/`                 | `external/kotlin/`                  |
| `scripts/`                    | `tools/scripts/`                    |
| `patches/`                    | `tools/patches/`                    |
| `wasmtime-issue-artifacts/`   | `tools/triage/wasmtime-issues/`     |
| `wart-leak-repro/`            | `repros/wart-leak-repro/`           |
| `kt-memalloc-repro/`          | `repros/kt-memalloc-repro/`         |
| `md-smoke-rust/`              | `repros/md-smoke-rust/`             |
| `wart-app-md-smoke/`          | `repros/wart-app-md-smoke/`         |

Dir renames at the same time (in steps 3 + 4 batched):
- `markdown-renderer` → `war.markdown.renderer`
- `emoji-picker` → `war.emoji.picker`
- `system-fonts` → `war.fonts.loader`
- `wart-stack-magisk` → `magisk-module`

### Step 4 — Update `.gitignore` (~5 min)

Replace the flat `wart-*/`, `compose-*/`, etc. globs with paths
matching the new layout (`runtime/wart-*/`, `external/skiko/`,
…). The exception for the now-renamed `magisk-module` stays
tracked from the top.

### Step 5 — Update scripts + path constants (~1-2 h, mechanical)

Grep + sed pass for every old-path reference:

```
grep -rEn '(wart-host|wart-arbiter|wart-app|war\.ime\.keyboard|war\.lang\.bg|war\.lang\.fr|markdown-renderer|emoji-picker|system-fonts|wart-stack-magisk|compose-multiplatform-core|wasmtime-src|skiko|scripts/|patches/|wart-app-md-smoke|md-smoke-rust|wart-leak-repro|kt-memalloc-repro)' --include=*.{sh,toml,kts,gradle,rs,md} | …
```

Files most affected:
- `tools/scripts/*.sh` (every script that references the old
  paths)
- `runtime/wart-host/Cargo.toml` (any `path = "../wasmtime-src/…"` → `path = "../../external/wasmtime/…"`)
- `runtime/wart-host/build.rs` (vendor paths)
- `apps/system/war.markdown.renderer/src/lib.rs` (`wit_bindgen::generate!({ path: "../wit/markdown.wit", … })` → `path: "../../../wit/markdown.wit"`)
- `apps/*/build.gradle.kts` (skiko + compose-multiplatform-core paths)
- Every `wit/deps/` symlink or copy of the canonical WIT
- `CLAUDE.md` (the Repository layout section)

### Step 6 — Update wit/deps mirrors (~30 min)

Each warpkg has `wit/deps/<dep-name>/<dep>.wit` mirrors of the
canonical `wart/wit/<dep>.wit`. Re-derive each based on the new
relative paths (or convert to symlinks pointing at the canonical
file — task 53 candidate).

### Step 7 — Rebuild everything from scratch + smoke (~2 h)

- `cd runtime/wart-host && cargo build --target aarch64-linux-android --release` — Rust host rebuild.
- `cd runtime/wart-arbiter && cargo build --target aarch64-linux-android --release` — arbiter rebuild.
- `cd apps/user/wart-app && ./gradlew compileProductionExecutableKotlinWasmWasi` — Kotlin/Wasm rebuild.
- `cd apps/system/war.ime.keyboard && ./gradlew compileProductionExecutableKotlinWasmWasi` — IME rebuild.
- Each `apps/system/{war.markdown.renderer,war.emoji.picker,war.fonts.loader,lang/war.lang.bg,lang/war.lang.fr} && cargo build --target wasm32-wasip2 --release`.
- `bash tools/scripts/build-system-warpkgs.sh` — packs + installs all warpkgs.
- `bash tools/scripts/pack-ime-keyboard.sh` — IME warpkg.
- Hybrid stack relaunch + IME 🌐 cycle smoke (the task-49 step-6
  scenario).

### Step 8 — Commit + push + close-out (~30 min)

Single commit per repo (the parent `wart` repo, each sibling
repo's path-constant updates if any). Update CLAUDE.md status
row. Memory `feedback_repo_layout.md` captures the convention.

## Trade-offs

### Wins

- **Navigability.** `ls apps/system/` shows everything bundled
  with the runtime; `ls apps/user/` shows user-facing demos. No
  more "is `markdown-renderer/` a system thing or a smoke?"
- **Onboarding.** A new contributor can read `docs/repository-layout.md`
  and `apps/<x>/<y>/README.md` and know where to add a new
  thing. Today: read CLAUDE.md, read 5 tasks, infer.
- **Naming consistency.** Every warpkg matches its app_id. Every
  native binary uses `wart-`.
- **Future-proofing.** `apps/system/lang/` already groups
  language plugins; task-49 / task-51 follow-ups slot in
  obviously.

### Costs

- **One-time mechanical churn.** ~30 path updates in scripts +
  Cargo.toml / build.gradle.kts. Mostly grep+sed.
- **External git history disruption.** Each sibling repo's
  codeberg origin is unchanged, but the working-tree LOCATION
  shifts. Any open branches / WIP / scripts that hard-coded
  `~/wart/wart-host/` need updating.
- **Doc churn.** All `tasks/*.md` references like `wart-host/cpp/sf_surface.cpp`
  point to old paths. Either bulk-update or leave as historical;
  the latter is fine (tasks are point-in-time).
- **Re-push of renamed sibling repos.** war.markdown.renderer etc.
  need a new codeberg URL OR a rename on codeberg (the latter is
  free + keeps history). Same for emoji-picker, system-fonts,
  wart-stack-magisk if they keep their old remote names.

### What this doesn't address

- WIT mirroring. Each warpkg copies the canonical WIT into
  `wit/deps/`. Could replace with symlinks (task 53 candidate)
  but not in scope here.
- `tasks/` history. Old task docs reference old paths. Leave
  as-is; they're frozen snapshots.

## Repository organization (single repo + submodules)

Task 52 covers the **directory** reorganization. The **git
organization** — should everything be one repo, separate repos,
or hybrid — is bundled in here because the answer drives a
companion task (53) that has to land before / alongside 52.

### Decision: hybrid

- **First-party = single monorepo.** Everything in
  `apps/`, `runtime/`, `wit/`, `tools/`, `repros/`, `docs/`,
  `tasks/` lives in **one** git repo (the wart repo itself).
- **External forks = submodules.** Everything in `external/`
  is a git submodule pointing at our fork (or upstream).

Why hybrid:
- Layer-1 code co-evolves at WIT boundaries. A change to
  `wit/skiko-gfx.wit` requires touching the host impl + every
  guest binding + every smoke. **One commit beats N.** Today's
  N-repo arrangement made the task-49-step-5 plugin shipment
  take 4 separate commits across 4 repos that couldn't be
  bisected together.
- Shared `Cargo.lock` dedups Rust deps across host, arbiter,
  and every system warpkg.
- Discoverability — `git clone` gets all first-party code.
- External forks are slow-moving + huge — submodules amortize
  the size cost and make upstream-rebase a single explicit
  operation (`git submodule update --remote`).
- Submodule fatigue is real with many small modules — not real
  with a handful of large rare-update ones.

### Repos to merge (subtree import, history preserved)

| Sibling repo (codeberg.org/harryzz/…)   | Target prefix in monorepo            |
|----------------------------------------|--------------------------------------|
| `wart-host.git`                        | `runtime/wart-host/`                 |
| `wart-arbiter.git`                     | `runtime/wart-arbiter/`              |
| `wart-app.git`                         | `apps/user/wart-app/`                |
| `war.ime.keyboard.git`                 | `apps/system/war.ime.keyboard/`      |
| `war.lang.bg.git`                      | `apps/system/lang/war.lang.bg/`      |
| `war.lang.fr.git`                      | `apps/system/lang/war.lang.fr/`      |
| `markdown-renderer.git`                | `apps/system/war.markdown.renderer/` |
| `emoji-picker.git`                     | `apps/system/war.emoji.picker/`      |
| `system-fonts.git`                     | `apps/system/war.fonts.loader/`      |
| `md-smoke-rust.git`                    | `repros/md-smoke-rust/`              |
| `wart-app-md-smoke.git`                | `repros/wart-app-md-smoke/`          |
| `wart-leak-repro.git`                  | `repros/wart-leak-repro/`            |
| `kt-memalloc-repro.git`                | `repros/kt-memalloc-repro/`          |

Subtree-import preserves each repo's full history under the
target prefix. After merge, the codeberg repos are **archived**
(read-only, marker note in their README pointing at the
monorepo). No data loss.

### Repos to keep as submodules

| Upstream / fork                                        | Submodule target              |
|--------------------------------------------------------|-------------------------------|
| `codeberg.org/harryzz/skiko.git`                       | `external/skiko/`             |
| `codeberg.org/harryzz/wasmtime.git` *(needs push)*     | `external/wasmtime/`          |
| `codeberg.org/harryzz/compose-multiplatform-core.git`  | `external/compose-multiplatform-core/` |
| `github.com/JetBrains/kotlin.git` *(upstream — fork deferred until we have local patches)* | `external/kotlin/` |

The `wasmtime-src/` tree currently tracks upstream
`github.com/bytecodealliance/wasmtime` but carries a local
commit (KT-86415 adapter fix `058822330`). It needs its own
fork URL on codeberg first — that's step 0 of task 53.

`kotlin-src/` has zero local commits beyond upstream + is a
partial clone (`blob:none`), so pushing it would re-upload
several GB of upstream blobs for no current gain. Submodule
against upstream `github.com/JetBrains/kotlin` directly; swap
to the codeberg fork (already created, kept empty) when local
patches arrive.

### What stays multi-repo (the parent + submodules)

Just two layers of git tracking:
1. `~/wart/.git` — the parent monorepo (Layer 1 + submodule
   pointers).
2. `~/wart/external/<fork>/.git` — each submodule's working
   tree, points at its own remote.

No git in `apps/`, `runtime/`, `repros/`, etc. — those are
just directories in the parent monorepo now.

### Migration carried out by **task 53** (separate task)

The mechanical subtree-merge + archive flow is split into its
own task because it touches every codeberg repo and benefits
from being landed in one focused session. Task 52 (this doc)
covers the directory shape + naming; task 53 covers the git
history shuffle.

See `tasks/53-monorepo-merge.md`.

## Effort estimate

~5 hours focused work, mostly mechanical. Risk: scripts that
hard-code paths in ways grep won't catch (string-interpolated
paths, `${REPO_ROOT}/${SOMETHING}`). Mitigation: full smoke at
the end (step 7) is the safety net.

## When to do this

Before tasks 51 (dynamic lang loading) or 53 (anything that adds
new top-level entries). Doing the reorg first means the new
tasks slot into the right place from day one.

## Open questions

- **Q1: Should `apps/system/war.ime.keyboard/` move into a
  per-class subdir like `apps/system/ime/war.ime.keyboard/`?**
  Probably not — there's only one IME today. If voice IME +
  emoji IME ship later, then yes.
- **Q2: Should `runtime/magisk-module/` stay at runtime/ level or
  graduate to `tools/`?** It's part of the production deployment,
  not a dev tool. Keep at runtime/.
- **Q3: Should the canonical `wit/` live under `runtime/wit/`
  (since the runtime is the primary implementor)?** No — it's
  the cross-cutting contract; top-level is right.
- **Q4: Do we rename `wart-app` to `war.example.demo` (matching
  the codeberg+device app_id `com.example.wart-app`)?** Tempting
  but disruptive. Defer to a follow-up rename pass.

## Sources / precedents

- [WebAssembly Component Model 2026 cheat sheet](https://techbytes.app/posts/wasm-component-model-cheat-sheet/) — current state of WCM polyglot composition.
- [The Ultimate Guide to Building a Monorepo (Medium)](https://medium.com/@sanjaytomar717/the-ultimate-guide-to-building-a-monorepo-in-2025-sharing-code-like-the-pros-ee4d6d56abaa) — apps/ + packages/ pattern, language-agnostic monorepo conventions.
- [Decoding AOSP Folder Structure (Embien)](https://www.embien.com/blog/decoding-aosp-folder-structure-for-developers) — frameworks/ vs system/ vs packages/ vs hardware/ vs vendor/ separation that wart parallels.
- [Large Rust Workspaces (matklad)](https://matklad.github.io/2021/08/22/large-rust-workspaces.html) — flat `crates/` vs `crates/`+`libs/` boundary at 10K–1M LoC scale.
- [Managing multiple languages in a monorepo (Graphite)](https://graphite.com/guides/managing-multiple-languages-in-a-monorepo) — polyglot organization, avoid mixing unrelated services per directory.
- [wasmCloud Runtime](https://wasmcloud.com/docs/runtime/) — single-binary host + extensions pattern; WIT contract location.

[1]: https://matklad.github.io/2021/08/22/large-rust-workspaces.html
