---
name: reference-wandrpkg-manifest-convention
description: "Each wandrpkg's package.toml lives in its app source dir (apps/.../package.toml); pack scripts copy it, no heredocs"
metadata: 
  node_type: memory
  type: reference
  originSessionId: d33723e5-8289-42cf-b090-a027d6a8e217
---

Every wandrpkg's manifest is a **`package.toml` at the root of its app source
dir** (`apps/system/<app>/package.toml`, `apps/user/<app>/package.toml`) —
the single source of truth. The pack scripts **copy** it into the `.wandrpkg`;
they do NOT generate it. (Refactor 2026-05-30 moved these out of heredocs
inside `build-system-wandrpkgs.sh` / `pack-ime-keyboard.sh` so they aren't
regenerated every pack.)

- `pack_wandrpkg "$PKG" "$WASM" "<comp-name>" "$REPO_ROOT/apps/.../package.toml" [assets_dir]`
  — 3rd arg is the component name and MUST match the toml's `[components]`
  entry; optional 5th arg is an `assets/` dir to bundle (task 38).
- To change an app's manifest, edit its `package.toml` directly — no
  script-diving.
- `orientation` field (`"auto" | "locked"`, default locked, NOT in the AOT
  cache-key — editing it doesn't invalidate the `.cwasm`): standing config
  is `wandr.launcher` = `locked` (home portrait + locks chrome), bars + IME +
  user apps = `auto`. See [[project-overlay-orientation]].
- 11 apps have one: wandr.markdown.renderer, wandr.emoji.picker, wandr.fonts.loader,
  wandr.launcher, wandr.statusbar, wandr.taskbar, wandr.ime.keyboard, lang/wandr.lang.bg,
  lang/wandr.lang.fr, wandr.dioxus.demo, wandr-app.

Documented in `docs/repository-layout.md` (`apps/` section + the
how-to-add-a-new-thing table).
