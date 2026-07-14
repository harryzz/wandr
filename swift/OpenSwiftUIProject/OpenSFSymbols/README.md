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

Add a font: drop its `{font,file,glyphs}` table in `Data/fonts/`. Map a symbol: add it to
`overrides.json`. Then `python3 Tools/generate.py && swift test`. The generator validates every
override glyph against its font and reports invalid ones.

## Coverage

With **Tabler** alone: **638 / 6,984 mapped** (27 curated + 611 auto-normalized). Adding the other
webfont sets (MDI ~7k, Font Awesome, Lucide, Remix) raises this substantially. Coverage of
available glyphs is not the limit — the `SF-name → glyph-name` matching is, and it grows via
curated overrides as apps need symbols.
