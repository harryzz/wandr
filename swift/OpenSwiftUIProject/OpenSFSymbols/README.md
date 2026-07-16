# OpenSFSymbols

Open, cross-platform counterpart to Apple's Darwin-only `SFSymbols` framework — the piece
OpenSwiftUI needs to render `Image(systemName:)` on **Linux / Windows / wasm**, where Apple's
CUICatalog and SF Pro font are unavailable.

Apple's SF Symbols glyphs live at private codepoints in Apple's own (non-redistributable) font,
so they can't be shipped off-Apple. Instead, OpenSFSymbols resolves the chain

```
SF-Symbol name  →  open icon font  →  unicode codepoint
   "text.justify"  →  tabler-icons.ttf  →  U+EC42  (glyph "menu-2")
```

using open-licensed icon fonts. `(font, codepoint)` is directly render-ready: layer 3 loads the
font file and draws the glyph with `draw_glyphs`.

## Layers

1. **name → font → codepoint (this module).** Pure Swift, no deps, no rendering. **← implemented.**
2. **OpenSwiftUI hook.** The non-Apple `Image(systemName:)` path asks OpenSFSymbols for the
   `IconRef` and emits a glyph-render DisplayList item. *(todo)*
3. **Renderer.** The host loads the font file and draws the glyph at the codepoint. *(todo)*

## Design

- **Font is a parameter** (`fontPriority`), not baked in. A symbol may resolve from whichever
  configured font has it — cross-font fallback.
- **Only fonts that ship a real webfont** (a `name → codepoint` table) can be used: Tabler,
  Material/MDI, Font Awesome, Lucide, Remix. (Feather/Heroicons are SVG-only — excluded from the
  font path unless a font is built from their SVGs.)
- **No silent gaps.** The full SF-name universe is known; a name with no substitution is surfaced
  via `missingNames` / `requireIconRef(...)` (throws), never rendered blank.

## Usage

```swift
let symbols = OpenSFSymbols(fontPriority: ["tabler"])
let ref = symbols.iconRef(for: "text.justify")
// IconRef(font: "tabler", fontFile: "tabler-icons.ttf", glyph: "menu-2", codepoint: 0xEC42)
try symbols.requireIconRef(for: "some.symbol")   // throws .noSubstitution for investigation
OpenSFSymbols.missingNames                         // coverage backlog
print(OpenSFSymbols.coverageSummary())
```

## Data & regeneration

```
Data/sf-symbols-7.json      SF-Symbol name universe (6984 names)
Data/fonts/<font>.json      an icon font's own table: {font, file, license, glyphs:{name:hex}}
Data/overrides.json         curated  SF-name -> [[font-id, glyph-name], ...]  (hand-verified)
Data/coverage-report.txt    generated: mapped / missing + invalid overrides
Tools/generate.py           regenerates Sources/OpenSFSymbols/Generated.swift + the report
```

Add a font: drop its `{font,file,glyphs}` table in `Data/fonts/`, and the actual `.ttf`/`.otf` in
`Resources/`. Map a symbol: add it to `overrides.json`. Then `python3 Tools/generate.py && swift
test`. The generator validates every override glyph against its font and reports invalid ones.

The `font` id in `Data/fonts/<font>.json` must equal the file name minus its extension — this is
what `IconRef.fontFamily` derives and what the host resolves **by name** via the platform's system
font manager (fontconfig / DirectWrite / `SkFontMgr_New_Android`). If two fonts share the same
*internal* name-table family (common for sibling styles like an "outline" + "filled" variant of the
same set — both may report `family="tabler-icons"` even though the files differ), the second one
will silently collide with/shadow the first at the OS level. Patch the internal name-table records
(`fontTools`: `TTFont(path)['name']`, rewrite every record whose value equals the shared name —
family, unique ID, full name, PostScript name, both Mac and Windows platform entries) to a distinct
name that matches the new `Data/fonts/<font>.json` id before vendoring it.

### Deploying the font file on-device — Android needs 2 places, do NOT dig for this again

Getting a webfont's actual bytes onto each target platform so `match_family_style` can find it by
name is a **separate step** from the SF-name→font→codepoint mapping above, and it's the one gotcha
that has cost real debugging time twice now. Once the `Data/fonts/<font>.json` entry + `Resources/`
`.ttf` exist:

- **Linux desktop**: install as a normal user fontconfig font — `~/.local/share/fonts/<font-id>/`,
  then `fc-cache -f <dir>`. Verify: `fc-scan --format "family=%{family}\n" <file>`.
- **Windows**: right-click → Install (or copy to `C:\Windows\Fonts\`, or
  `%LOCALAPPDATA%\Microsoft\Windows\Fonts\` for a no-admin per-user install).
- **Android (root, `--no-art`) — BOTH of these, not just one, or resolution silently returns `None`
  (falls back to a generic system font, no error) even after a full device reboot**:
  1. `/product/fonts/<font>.ttf` **+** a matching `<family customizationType="new-named-family"
     name="<font-id>">` entry in `/product/etc/fonts_customization.xml`.
  2. `/system/fonts/<font>.ttf` — the file Skia's Android font scanner actually reads. Missing
     this is the part that isn't obvious: fonts_customization.xml alone is not sufficient.

  Both paths are on the same read-only root filesystem on a system-as-root device (`/product` is a
  symlink to `/system/product`):
  ```
  adb push <font>.ttf /data/local/tmp/<font>.ttf
  adb shell "su -c '
    mount -o remount,rw /
    cp /data/local/tmp/<font>.ttf /product/fonts/<font>.ttf
    cp /data/local/tmp/<font>.ttf /system/fonts/<font>.ttf
    chown root:root /product/fonts/<font>.ttf /system/fonts/<font>.ttf
    chmod 644 /product/fonts/<font>.ttf /system/fonts/<font>.ttf
    mount -o remount,ro /
  '"
  ```
  Then add the `fonts_customization.xml` family entry the same way (pull, edit, push to
  `/data/local/tmp`, remount rw, copy into place, remount ro).

  Diagnose with the on-device `--font-probe <family1> <family2> ...` tool — output goes to
  **logcat**, not stdout:
  ```
  adb logcat -c
  adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wandr-host --font-probe <family>'"
  adb logcat -d | grep "match '"
  ```

## Coverage

With **Tabler** alone: **638 / 6,984 mapped** (27 curated + 611 auto-normalized). Adding the other
webfont sets (MDI ~7k, Font Awesome, Lucide, Remix) raises this substantially. Coverage of
available glyphs is not the limit — the `SF-name → glyph-name` matching is, and it grows via
curated overrides as apps need symbols.
