# Task 53 — Monorepo merge: subtree-import 13 sibling repos + 4 submodules

> Status: 🔲 scoped, not started — 2026-05-28
>
> Companion to [task 52](52-monorepo-reorg.md). Task 52 decides
> the target directory layout; task 53 carries out the git-side
> consolidation: subtree-merge all first-party sibling repos
> into the parent wart repo (history preserved) and wire the
> four upstream forks as submodules.

## Prerequisites

- Task 52 step 1 (decision + docs) landed.
- All sibling repos pushed to codeberg at their current state
  (`wart-host`, `wart-arbiter`, `wart-app`, `war.ime.keyboard`,
  `war.lang.bg`, `war.lang.fr`, `markdown-renderer`, `emoji-picker`,
  `system-fonts`, `md-smoke-rust`, `wart-app-md-smoke`,
  `wart-leak-repro`, `kt-memalloc-repro`).
- Codeberg accounts authorized to archive repos when done.
- A backup tag pushed on each sibling repo before the merge
  (`pre-monorepo-merge`).

## Step 0 — Push wasmtime fork to codeberg

`~/wart/wasmtime-src/` carries a local commit beyond upstream
(`058822330 wasi-preview1 adapter: pin State at a fixed linear-
memory address (KT-86415)` — the partition trick from task 34).
That commit needs to live on a codeberg fork before we can
submodule it.

Action:
```
cd ~/wart/wasmtime-src
git remote add codeberg https://codeberg.org/harryzz/wasmtime.git

# wasmtime-src may be a shallow clone — unshallow first, codeberg
# rejects shallow updates ("shallow update not allowed").
git fetch --unshallow origin
git config http.postBuffer 524288000           # 500 MB
git config http.lowSpeedLimit 0
git config http.lowSpeedTime 999999

# Codeberg 504s on the full 167 MB pack push in one shot; chunk it.
# Push older milestones first so each transaction is small enough.
for ref in v1.0.0 v10.0.0 v20.0.0 v30.0.0 v40.0.0 v44.0.0; do
    git push codeberg "$ref:refs/heads/main" --force
done
git push codeberg HEAD:main
```

User has to create the **empty** codeberg repo first at
`https://codeberg.org/harryzz/wasmtime` (Web UI → "+ Create
new" → "New Repository" — leave all "Initialize…" boxes
unchecked).

**Done 2026-05-28** — verified pushed; tip `058822330a` on
`codeberg.org/harryzz/wasmtime/main`.

**Note (decision 2026-05-28)**: `~/wart/kotlin-src/` is
**deferred**. It's a partial clone (`blob:none`) and has
**zero local commits** beyond upstream — pushing it would
backfill several GB of blobs from JetBrains and re-upload to
codeberg for no current benefit. Instead, step 5 submodules
kotlin against the upstream JetBrains URL directly. The empty
`codeberg.org/harryzz/kotlin` repo stays reserved for the
future moment when we land local patches; at that point, push
the local tree + swap the submodule URL in one commit.

## Step 1 — Backup tag on every sibling repo

Safety net for the irreversible-feeling steps below.

```
for repo in wart-host wart-arbiter wart-app war.ime.keyboard \
            war.lang.bg war.lang.fr markdown-renderer emoji-picker \
            system-fonts md-smoke-rust wart-app-md-smoke \
            wart-leak-repro kt-memalloc-repro; do
    ( cd "$HOME/wart/$repo" && \
        git tag -a pre-monorepo-merge -m "snapshot before task 53 monorepo merge" && \
        git push origin pre-monorepo-merge )
done
```

**Done 2026-05-28** — all 13 tags pushed.

(Aside: before the tag pass, push any unpushed branch commits so
the tag's snapshot stays useful. This session caught one
unpushed commit in `kt-memalloc-repro` (`4946435..eee981e` on
main) — pushed before tagging proceeded.)

## Step 2 — Move existing sibling repos out of ~/wart

Subtree-merge **adds** the sibling repo's content under a
prefix; the existing working-tree at the sibling location has
to be out of the way first. Move (don't delete) so we can
roll back if needed.

```
mkdir -p ~/wart-premerge-backup
for repo in wart-host wart-arbiter wart-app war.ime.keyboard \
            war.lang.bg war.lang.fr markdown-renderer emoji-picker \
            system-fonts md-smoke-rust wart-app-md-smoke \
            wart-leak-repro kt-memalloc-repro; do
    mv "$HOME/wart/$repo" "$HOME/wart-premerge-backup/"
done

# Forks too — about to become submodules
mv ~/wart/skiko                       ~/wart-premerge-backup/
mv ~/wart/wasmtime-src                ~/wart-premerge-backup/  # → external/wasmtime
mv ~/wart/kotlin-src                  ~/wart-premerge-backup/  # → external/kotlin
mv ~/wart/compose-multiplatform-core  ~/wart-premerge-backup/
```

## Step 3 — Create the new directory shape

```
cd ~/wart
mkdir -p apps/system/lang apps/user runtime external tools repros
```

Move existing tracked directories into the new tree:
```
git mv scripts                        tools/scripts
git mv patches                        tools/patches
git mv wasmtime-issue-artifacts       tools/triage/wasmtime-issues
git mv wart-stack-magisk              runtime/magisk-module
git commit -m "task 52: lay out apps/ runtime/ external/ tools/ repros/"
```

## Step 4 — Subtree-import the sibling repos

Each import is one command. Use `--squash` if you don't want
the full sibling history (smaller monorepo, history available
in archived codeberg repo) or omit `--squash` to preserve every
commit (bigger, full bisect across repos). Default: **omit
`--squash`** — full history wins for the project's debugging
patterns.

Branch caveat: the 13 sibling repos use a mix of `main` and
`task-33-boot-model` for their dev-tip. Subtree-add the
correct ref per repo:

| Repo                | Branch                  |
|---------------------|-------------------------|
| wart-host           | `task-33-boot-model`    |
| wart-arbiter        | `task-33-boot-model`    |
| markdown-renderer   | `task-33-boot-model`    |
| wart-app-md-smoke   | `task-33-boot-model`    |
| (everything else)   | `main`                  |

```
cd ~/wart

# Native runtime — both on task-33-boot-model
git remote add -f tmp-host    https://codeberg.org/harryzz/wart-host.git
git subtree add --prefix=runtime/wart-host    tmp-host    task-33-boot-model
git remote remove tmp-host

git remote add -f tmp-arb     https://codeberg.org/harryzz/wart-arbiter.git
git subtree add --prefix=runtime/wart-arbiter tmp-arb     task-33-boot-model
git remote remove tmp-arb

# User apps
git remote add -f tmp-app     https://codeberg.org/harryzz/wart-app.git
git subtree add --prefix=apps/user/wart-app   tmp-app     main
git remote remove tmp-app

# System apps
git remote add -f tmp-ime     https://codeberg.org/harryzz/war.ime.keyboard.git
git subtree add --prefix=apps/system/war.ime.keyboard tmp-ime main
git remote remove tmp-ime

git remote add -f tmp-md      https://codeberg.org/harryzz/markdown-renderer.git
git subtree add --prefix=apps/system/war.markdown.renderer tmp-md task-33-boot-model
git remote remove tmp-md

git remote add -f tmp-em      https://codeberg.org/harryzz/emoji-picker.git
git subtree add --prefix=apps/system/war.emoji.picker tmp-em main
git remote remove tmp-em

git remote add -f tmp-ft      https://codeberg.org/harryzz/system-fonts.git
git subtree add --prefix=apps/system/war.fonts.loader tmp-ft main
git remote remove tmp-ft

# Language plugins
git remote add -f tmp-bg      https://codeberg.org/harryzz/war.lang.bg.git
git subtree add --prefix=apps/system/lang/war.lang.bg tmp-bg main
git remote remove tmp-bg

git remote add -f tmp-fr      https://codeberg.org/harryzz/war.lang.fr.git
git subtree add --prefix=apps/system/lang/war.lang.fr tmp-fr main
git remote remove tmp-fr

# Repros + smokes
git remote add -f tmp-leak    https://codeberg.org/harryzz/wart-leak-repro.git
git subtree add --prefix=repros/wart-leak-repro tmp-leak main
git remote remove tmp-leak

git remote add -f tmp-mem     https://codeberg.org/harryzz/kt-memalloc-repro.git
git subtree add --prefix=repros/kt-memalloc-repro tmp-mem main
git remote remove tmp-mem

git remote add -f tmp-mdr     https://codeberg.org/harryzz/md-smoke-rust.git
git subtree add --prefix=repros/md-smoke-rust tmp-mdr main
git remote remove tmp-mdr

git remote add -f tmp-mdk     https://codeberg.org/harryzz/wart-app-md-smoke.git
git subtree add --prefix=repros/wart-app-md-smoke tmp-mdk task-33-boot-model
git remote remove tmp-mdk
```

13 subtree-add commits land. Verify with `git log --oneline -20`
and `ls apps/system/`.

## Step 5 — Wire the four external forks as submodules

```
cd ~/wart
git submodule add https://codeberg.org/harryzz/skiko.git                        external/skiko
git submodule add https://codeberg.org/harryzz/wasmtime.git                     external/wasmtime
git submodule add https://codeberg.org/harryzz/compose-multiplatform-core.git   external/compose-multiplatform-core
# Kotlin tracks upstream directly — no local patches today; codeberg
# fork repo reserved for future use. Swap URL when patches land.
git submodule add https://github.com/JetBrains/kotlin.git                       external/kotlin

git commit -m "task 53: wire upstream forks as submodules"
```

`.gitmodules` ends up at the root:
```
[submodule "external/skiko"]
    path = external/skiko
    url = https://codeberg.org/harryzz/skiko.git
[submodule "external/wasmtime"]
    path = external/wasmtime
    url = https://codeberg.org/harryzz/wasmtime.git
[submodule "external/compose-multiplatform-core"]
    path = external/compose-multiplatform-core
    url = https://codeberg.org/harryzz/compose-multiplatform-core.git
[submodule "external/kotlin"]
    path = external/kotlin
    url = https://github.com/JetBrains/kotlin.git
```

## Step 6 — Update `.gitignore`

Old globs (`wart-*/`, `compose-*/`, `skiko/`, `markdown-renderer/`,
`emoji-picker/`, `system-fonts/`, `md-smoke-rust/`, `kotlin-src/`,
`wasmtime-src/`, `kt-memalloc-repro/`) all become obsolete.
Replace with:

```gitignore
# Build artifacts everywhere
**/build/
**/target/
**/.gradle/

# Submodules: tracked at the parent level, but their working
# trees have their own gitignores
```

Submodule contents are governed by their own `.gitignore` files
(inside `external/<fork>/.gitignore`).

## Step 7 — Update path references in tools/scripts + Cargo.toml

This is the mechanical bulk of task 52 step 5 — runs after the
merge so the grep targets are in their final locations.

```
grep -rEn \
    '(wart-host|wart-arbiter|wart-app|war\.ime\.keyboard|war\.lang\.bg|war\.lang\.fr|markdown-renderer|emoji-picker|system-fonts|wart-stack-magisk|compose-multiplatform-core|wasmtime-src|skiko/|kotlin-src|wart-app-md-smoke|md-smoke-rust|wart-leak-repro|kt-memalloc-repro|scripts/|patches/)' \
    --include='*.sh' --include='*.toml' --include='*.kts' --include='*.gradle' \
    --include='*.rs' --include='*.md' \
    tools/ runtime/ apps/ repros/ docs/ wit/ CLAUDE.md \
    | tee /tmp/path-refs.txt
```

Update each match. Most common patterns:
- `path = "../wit/<x>.wit"` → relative paths shift by the depth change
- `~/wart/wart-host/...` → `~/wart/runtime/wart-host/...`
- `markdown-renderer` → `apps/system/war.markdown.renderer` (or app_id `war.markdown.renderer` unchanged — only the build dir changes)

`build.gradle.kts` in apps/ likely have hard-coded relative
paths to `../../../skiko` or `../../../compose-multiplatform-core`
— those go through `external/`.

Commit per cluster (one for runtime/, one for apps/system/, one
for tools/, etc.) so the diff stays bisectable.

## Step 8 — Reconfigure on-device installer paths

If the build-system-warpkgs.sh script sourcing the
`markdown-renderer/` dir is changed to source
`apps/system/war.markdown.renderer/`, the dev pipeline rebuilds.
On-device `app_id` strings stay identical — `war.markdown.renderer`
is the app_id whether the source dir is `markdown-renderer/` or
`war.markdown.renderer/`. Same for emoji-picker → `war.emoji.picker`,
system-fonts → `war.fonts.loader`.

## Step 9 — Full rebuild + smoke

Same as task 52 step 7 — every Cargo crate, every Gradle
project, every warpkg packs + installs, then run the IME 🌐
cycle smoke (task 49 step 6).

## Step 10 — Archive sibling repos on codeberg

For each of the 13 subtree-imported repos:
1. Push a final commit to its main branch noting the move:
   ```
   git checkout --orphan archived
   echo "# Archived 2026-XX-XX" > README.md
   echo "" >> README.md
   echo "This repo's history is now part of the wart monorepo at" >> README.md
   echo "<https://codeberg.org/harryzz/wart> under the prefix" >> README.md
   echo "\`apps/system/war.ime.keyboard/\` (or equivalent — see" >> README.md
   echo "\`docs/repository-layout.md\` in the monorepo)." >> README.md
   git add README.md && git commit -m "archive: moved to wart monorepo"
   git push -f origin archived:main
   ```
2. Mark the repo "Archived" via codeberg web UI (Repository
   Settings → Archive).

The `pre-monorepo-merge` tag from step 1 stays as the last
non-archive commit on each repo, in case anyone needs to
recover the pre-merge state.

## Step 11 — Delete `~/wart-premerge-backup` (only when smoke passes)

Don't `rm -rf` until everything builds, the device-side smoke
passes, and one full clone-from-scratch test confirms the
monorepo + submodules clone cleanly. Then remove:

```
rm -rf ~/wart-premerge-backup
```

## Effort estimate

~6-8 hours focused, mostly mechanical. The risky parts are:
- step 0 (push fork URLs — needs codeberg repo creation by user)
- step 4 (13 subtree adds — slow but no surprises if all repos
  are pushed)
- step 7 (path-reference updates — grep coverage is the safety
  net; full rebuild in step 9 catches anything grep missed)
- step 10 (archives — irreversible by codeberg's archive
  semantics; do last)

## Rollback plan

Every step has a clear undo:
- Steps 1-3: just `mv` directories back, nothing pushed.
- Steps 4-5: `git reset --hard HEAD~N` removes the merge
  commits. Sibling repos still exist with all history.
- Step 6: revert .gitignore.
- Step 7: revert path-reference commits.
- Step 9: device-side undo is `bash tools/scripts/build-system-warpkgs.sh`
  again from the pre-merge tree.
- Step 10: codeberg "Unarchive" works for archived repos.
- Step 11: don't delete the backup until everything's verified.

If any step fails halfway, the rule is: bail, restore from
`~/wart-premerge-backup`, file a follow-up. Don't try to fix
mid-merge.

## After: clone instructions

A fresh contributor will:
```
git clone --recurse-submodules https://codeberg.org/harryzz/wart.git
cd wart
# All first-party code is here. Submodules under external/
# point at the four forks; they're cloned automatically.
```

Without `--recurse-submodules`, `external/<x>/` will be empty
shells until `git submodule update --init --recursive` runs.

The runtime + apps + tools + repros are all immediately ready
for build. External-fork rebuilds (e.g. updating skiko after a
patch) follow the existing per-fork `BUILD.md`.

## Related

- [`tasks/52-monorepo-reorg.md`](52-monorepo-reorg.md) —
  directory shape + naming conventions (the *why*).
- [`docs/repository-layout.md`](../docs/repository-layout.md) —
  canonical reference once both tasks land.
- [Monorepo vs Polyrepo vs Submodule vs Subtree (Mammadzada)](https://raminmammadzada.medium.com/monorepo-vs-multirepo-vs-git-submodule-vs-git-subtree-3fde1af15b76) — option comparison the decision is based on.
- [GitHub Well-Architected: repository architecture strategy](https://wellarchitected.github.com/library/architecture/recommendations/scaling-git-repositories/repository-architecture-strategy/) — official-feeling justification for the hybrid approach.
