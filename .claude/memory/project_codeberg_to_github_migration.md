---
name: project_codeberg_to_github_migration
description: All harryzz repos moved Codeberg→GitHub (Codeberg banned LLM-generated code); canonical remote is now github.com/harryzz
metadata: 
  node_type: memory
  type: project
  originSessionId: 8f923d2a-de3d-450d-8444-07ecb72775c5
  modified: 2026-07-28T07:46:51.861Z
---

**On 2026-07-28 all `harryzz` Codeberg repos were migrated to GitHub. The canonical
remote is now `github.com/harryzz`, NOT the `codeberg.org/harryzz/wandr` URL still
printed in CLAUDE.md and old memories.** Reason: Codeberg ToS §2(1)7 (added
2026-06-29, commit 71149c7) bans projects that "mostly consist of code written by
'generative AI'-tools (including … *Claude*)" — first violation = content removal +
warning, repeat = account suspension. wandr is Claude-Code-driven, so it was moved.

**Repos moved (Codeberg → GitHub, all public):** `wandr` (main), and forks `skiko`,
`wasmtime`, `compose-multiplatform-core`, `libsignal-service-rs`, `audioclient-rs`,
`rsbinder`. Already on GitHub before this (untouched): `wandr-host`, `wandr-wit`
(the `contracts` submodule), `wandr-sensors-client`, the 4 OpenSwiftUI forks.
Both `.gitmodules` (main + `runtime/wandr-host`) now point at GitHub.

**Gotchas worth remembering:**
- **compose-multiplatform-core history was REWRITTEN.** `jb-main` had
  `camera/gradle/wrapper/gradle-4.6-all.zip` = 101.8 MiB in ancestry, over GitHub's
  **100 MiB per-file hard limit**. Fixed with `git filter-repo --strip-blobs-bigger-than 100M`.
  Pin `73189e2f` → **`e4cd1bf`**; the pinned *tree* is unchanged (`6cbfea6d`) so the
  build is byte-identical. The two 34/39 MiB AndroidX test videos are legal (<100 MiB)
  and were kept. **GitHub free: no total-storage cap, but 100 MiB/file is a hard block,
  ~2 GB/push; Git LFS free = 1 GB.** Always scan fork history for >100 MiB blobs before
  mirroring (`git rev-list --objects --all | git cat-file --batch-check`).
- **libsignal pin `25400d1b` was local-only** — 2 task-115 commits (transport-backend
  selection, spawn/sleep executor seam) were never pushed to the fork remote, so the
  gitlink was dangling even on Codeberg. Pushed to GitHub `wandr-wasi-transport`.
- The Windows wandr-host clone (`C:\Users\harry\wandr-host-build`) needs
  `git pull && git submodule sync` to pick up the new GitHub submodule URLs.
- Mirror-clone backups of all 7 Codeberg repos are in `~/wandr-migration-backup/`.

Codeberg repos are being made **private** (not deleted) as the compliance step.
Related: [[feedback_no_new_branches]].
