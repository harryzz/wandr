---
name: reference_openswiftui_sfsymbols_rendering
description: How Image(systemName:) SF Symbols render off-Apple in the OpenSwiftUI fork — OpenSFSymbols module + 2 root-cause fixes (Button stub, resizable-symbol AttributeGraph cycle)
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

`Image(systemName:)` now renders on non-Apple platforms (wasm/Linux/Windows/Android) with **zero
app code** — verified visually on Linux (eleev 2048 hamburger `text.justify` + reset
`arrow.counterclockwise.circle`). See [[reference_swift_openswiftui_wandr]].

## Architecture (the clean seam)
- **OpenSFSymbols** standalone Swift package (`swift/OpenSwiftUIProject/OpenSFSymbols/`): maps
  `SF-name → IconRef(fontFamily, codepoint)`, data-driven from `Data/fonts/<font>.json` webfont
  tables + `overrides.json`. Bundles Tabler (`tabler-icons`, 5071 glyphs). Model is
  **SF-name → font → unicode** so cross-font fallback works. `fontFamily` = font file minus `.ttf`.
- Wired at the **OpenSwiftUICore** level (not per-app): `Image.Resolved._makeView` has
  `#if !OPENSWIFTUI_LINK_COREUI return wandrMakeSymbolView(...)`. `WandrSymbolLeaf` (a Rule) reads
  the resolved `.systemSymbol(name)` label, resolves via OpenSFSymbols, and emits a
  `StyledTextContentView` (host-shaped-text leaf) tagged `wasmFontFamily` + `wasmSymbolFill`.
- Host resolves the icon font **by name** (skia-safe 0.99/m150: fontconfig / DirectWrite /
  `SkFontMgr_New_Android`; Android needs the ttf in `/system/fonts` + `/product` customization —
  see [[feedback_android_fonts]]). Tint flows via `env.foregroundColor` (the leaf reads it).

## The 2 root causes that hid this for days (both in the OpenSwiftUI fork, off-Apple)
1. **`Button` was an empty STUB** — `body: some View { EmptyView() }`, init discarded its label.
   So `Button { Image(systemName:) }` rendered NOTHING; `Image._makeView` was never even reached.
   Fix: implement `Button` to render its label (`Button.swift`: store action+label,
   `body = label.onTapGesture(perform: action)`). The **tap action** rides the gesture path —
   render is confirmed; tap delivery off-Apple is NOT yet verified (separate gesture gap).
2. **Resizable-symbol layout formed an AttributeGraph cycle** (`IAG::Graph::print_cycle` at
   backtrace frame 0, via `UpdateStack::push_slow`, wasm `unreachable` trap on render_frame #0).
   Cause: the symbol's content Rule read the **resolved** `inputs.size` (`ViewSize`) to size the
   glyph — but that size is what layout is *computing* from this very leaf → cycle. **RULE: never
   read the resolved ViewSize inside a content-producing Rule that feeds layout.**
   Fix: `.resizable()` symbols FILL their proposed frame (`StyledTextContentView.wasmSymbolFill` →
   `sizeThatFits` returns the proposed size, square em-box for `scaledToFit`), and the glyph is
   sized to the laid-out rect **at draw time** in `WandrDisplayListRenderer` (`.text` case:
   `fontSize = min(frame.w, frame.h)` when fill). Fill flag is set from
   `resolved.image.resizingInfo != nil`.

## Gotcha: `Image.Style` stack is always empty
Nothing in OpenSwiftUI pushes onto the `Image.Style` ViewInput (defaults `.empty`, no
`ImageStyleProtocol` conformer). So `Image._makeView`'s `popLast(Style.self)` is **always nil** and
the styled path (`style._makeImageView` → unimplemented `render(style:)`) is DEAD CODE. Don't try to
route symbols by discarding that Style — it's a no-op. Image tinting comes through the environment
foreground style, not that stack.

Files: `OpenSwiftUICore/View/Image/{WandrSymbolGlyph.swift, Image.swift, ResolvedImage.swift}`,
`View/Text/Text/Text+View.swift` (StyledTextContentView `wasmSymbolFill`),
`Render/DisplayList/WandrDisplayListRenderer.swift`, `OpenSwiftUI/View/Control/Button/Button.swift`.
