---
name: reference-warpkg-manifest-convention
description: "Each warpkg's package.toml lives in its app source dir (apps/.../package.toml); pack scripts copy it, no heredocs"
metadata: 
  node_type: memory
  type: reference
  originSessionId: d33723e5-8289-42cf-b090-a027d6a8e217
---

Every warpkg's manifest is a **`package.toml` at the root of its app source
dir** (`apps/system/<app>/package.toml`, `apps/user/<app>/package.toml`) —
the single source of truth. The pack scripts **copy** it into the `.warpkg`;
they do NOT generate it. (Refactor 2026-05-30 moved these out of heredocs
inside `build-system-warpkgs.sh` / `pack-ime-keyboard.sh` so they aren't
regenerated every pack.)

- `pack_warpkg "$PKG" "$WASM" "<comp-name>" "$REPO_ROOT/apps/.../package.toml" [assets_dir]`
  — 3rd arg is the component name and MUST match the toml's `[components]`
  entry; optional 5th arg is an `assets/` dir to bundle (task 38).
- To change an app's manifest, edit its `package.toml` directly — no
  script-diving.
- `orientation` field (`"auto" | "locked"`, default locked, NOT in the AOT
  cache-key — editing it doesn't invalidate the `.cwasm`): standing config
  is `war.launcher` = `locked` (home portrait + locks chrome), bars + IME +
  user apps = `auto`. See [[project-overlay-orientation]].
- 11 apps have one: war.markdown.renderer, war.emoji.picker, war.fonts.loader,
  war.launcher, war.statusbar, war.taskbar, war.ime.keyboard, lang/war.lang.bg,
  lang/war.lang.fr, war.dioxus.demo, wart-app.

Documented in `docs/repository-layout.md` (`apps/` section + the
how-to-add-a-new-thing table).
