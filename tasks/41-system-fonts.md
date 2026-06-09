# Task 41 — system fonts loader system component

> **Status:** 🔲 scoped, not started — third system component, picks
> up after task 40 (emoji picker) ships.

## Why

The MarkdownCard demo today renders headings + code-block + body all
in the default font. With Compose's `FontFamily.Default` /
`FontFamily.Monospace` only, we can't show off custom typography. A
system-fonts loader system component lets the markdown UI pull TTF
bytes from the host's `/system/fonts/` and use them via
`androidx.compose.ui.text.font.Font(byteArray)`.

This is the natural Compose-extension follow-up to task 40 (emoji),
exercising a different WIT pattern (host-side I/O inside the dep
instead of static data) and producing the most visible typography
upgrade with one small new component.

## WIT contract

`wit/system-fonts.wit`:

```wit
package wandr:fonts@0.1.0;

interface loader {
    /// Lightweight metadata for one installed font.
    record font-info {
        family:    string,  // e.g. "Roboto", "NotoSerif"
        style:     string,  // "Regular", "Bold", "Italic", "BoldItalic"
        path:      string,  // host-side path (informational)
    }

    /// List all fonts discoverable under the host's font search path
    /// (Android: /system/fonts/, /system/font_fallback/, plus Compose's
    /// known subdirs). Cached after first call.
    list: func() -> list<font-info>;

    /// Load the TTF/OTF bytes for one family+style. Returns `none`
    /// when no exact match — caller can list() to find available
    /// alternatives.
    load: func(family: string, style: string) -> option<list<u8>>;
}

world loader-world {
    export loader;
}
```

## Component implementation

`system-fonts/` (own git repo, sibling pattern):

- Rust cdylib, `wasm32-wasip2` target.
- Reads `/system/fonts/` via WASI preopen.
  - **Open question:** wasm32-wasip2 modules need a `/system/fonts/`
    preopen to read it. wandr-host's WasiCtxBuilder for the DEP would
    need to preopen this dir specifically. Two implementation paths:
    - **(a)** Hard-code the preopen for any dep whose WIT exports
      `wandr:fonts/*` — special-case in app_loader's dep instantiation.
    - **(b)** Add a manifest declaration in the dep's `package.toml`
      saying "I need read access to /system/fonts/" — installer copies
      perms into cache-key; loader honors it at dep-instantiate time.
    - **(c)** Implement via a NEW host WIT verb the dep imports
      (`my:skiko-gfx/sysio.read-system-file(path)`), keeping the
      filesystem access on the host. Same pattern as
      `my:skiko-gfx/assets.read`.

    (c) is the most consistent with the host-driven model and avoids
    deps having arbitrary filesystem access; the dep just calls into
    the host. Recommended for v1.

## Consumer changes (wandr-app)

- `wit/deps/system-fonts/system-fonts.wit` (new).
- `wit/wandr-app.wit` adds `import wandr:fonts/loader@0.1.0;`.
- `src/wasmWasiMain/kotlin/FontsImports.kt` (new) — hand-written
  bindings for `list` + `load`.
- `MarkdownCard.kt` extension: at composition time, `load("NotoSerif",
  "Regular")` for body / headings, `load("RobotoMono", "Regular")` for
  code-block; pass the bytes to
  `androidx.compose.ui.text.font.Font(bytes)` and apply via
  `FontFamily(Font(...))`.

## Verification

1. Install all three deps + wandr-app.
2. Screenshot: MarkdownCard with serif headings + serif body +
   monospace code — visibly different typography from today's default.
3. Logcat: `system-fonts: list() → N families`, `system-fonts:
   load(NotoSerif, Regular) → M bytes`.

## Out of scope (v1)

- Variable fonts (font-variation-axes). Stick to static fonts.
- Custom font formats beyond TTF/OTF.
- Font hot-reload (user installs a font → live re-render). Restart on
  font change.
- Per-app embedded fonts in the wandrpkg — the wandrpkg already supports
  `assets/`; that's where bundled fonts go. This task is specifically
  for SYSTEM fonts on the device.

## Related

- `tasks/40-emoji-picker.md` — predecessor; same shipping pattern.
- `tasks/39-generic-dep-wiring.md` — must be in place; no new wandr-host
  wiring code needed for system-fonts itself.
- `tasks/36-cross-app-deps.md`, `tasks/38-wandrpkg-assets.md` — the
  architecture this rides on.
