# Task 14 — Paragraph Layout (deferred)

> **Status: ✅ complete (verified 2026-05-15).** Implementation shipped as part of the working end-to-end Compose-on-WASM PoC. WIT entries, Rust host impl, and Kotlin wasmWasi stubs all in place. This file is kept as historical reference for the architectural decisions made during implementation.

## Status: DEFERRED — read this before starting

Paragraph is the full text layout engine: bidirectional text, Unicode line
breaking, font fallback, per-span styling (TextStyle), and precise metrics
(line height, ascenders, descenders).

On native platforms it is backed by `libharfbuzz` + `libicu`. Neither of those
libraries compiles to WASM32 without significant porting work.

**Start this task only when** a real Compose widget (e.g., `BasicText`,
`Text`) fails to render correctly and the failure is traced to missing
Paragraph support — not before. For most Skiko-level drawing, `drawString` and
`TextBlobBuilder` (task 10) are sufficient.

---

## Goal (when this task is started)

`ParagraphBuilder` / `Paragraph` / `ParagraphStyle` / `TextStyle` work for
the Latin text path. BiDi and CJK text are out of scope unless specifically needed.

Done looks like: a `ParagraphBuilder` builds a paragraph with two TextStyles
(different sizes and colors), lays out to a fixed width, and renders correctly.

---

## Feasibility assessment (do this first)

Before writing any code, run:

```bash
# Check if kotlinx-serialization or another text shaping lib ships WASM bindings
# that we could delegate to instead of porting harfbuzz.
# Also check if the Kotlin stdlib wasmWasi target has any text layout support.
grep -r "paragraph\|harfbuzz\|icu" \
    /home/harry/skiko/skiko/src/wasmWasiMain/ 2>/dev/null
```

If nothing is found, the port must be done from scratch.

---

## Approach options

### Option A: Thin host implementation via skia-safe (recommended)

Add a `paragraph` resource to WIT. The host builds the paragraph using
skia-safe's paragraph module (which links libharfbuzz and libicu natively).
The WASM guest calls builder-style WIT functions and gets back metrics + a
render call.

```wit
interface paragraph {
    record text-style {
        font-size:   f32,
        font-weight: u32,
        italic:      bool,
        color:       u32,
        font-family: list<u8>,
    }

    create-paragraph-builder: func(width: f32) -> u32;
    push-text-style:          func(id: u32, style: text-style);
    add-text:                 func(id: u32, text: list<u8>);
    pop-text-style:           func(id: u32);
    build-paragraph:          func(id: u32) -> u32;  // returns paragraph ID
    drop-paragraph-builder:   func(id: u32);

    layout:                   func(id: u32, width: f32);
    paint:                    func(id: u32, x: f32, y: f32);
    get-height:               func(id: u32) -> f32;
    get-line-count:           func(id: u32) -> u32;
    drop-paragraph:           func(id: u32);
}
```

Kotlin side: `ParagraphBuilder` calls WIT functions.
Rust side: wraps `skia_safe::textlayout::ParagraphBuilder`.

**Requires**: `skia-safe` built with the `textlayout` feature:

```toml
[dependencies.skia-safe]
version = "0.93"
features = ["gl", "textlayout"]
```

Verify the feature compiles for Android:

```bash
cd /home/harry/wasm-android-runtime/host
cargo check --target aarch64-linux-android --features textlayout 2>&1 | tail -20
```

### Option B: Pure-Kotlin line-breaking approximation

Implement a simplified Latin-only line breaker in Kotlin that measures text
using per-character advance approximation and wraps at word boundaries.
No harfbuzz, no ICU, no WIT changes needed.

Accurate enough for single-language UI text. Breaks for emoji, CJK, BiDi.

---

## Steps (Option A — start here)

### 1. Enable textlayout in `host/Cargo.toml`

```toml
[dependencies.skia-safe]
version = "0.93"
features = ["gl", "textlayout", "embed-icudtl"]
```

`embed-icudtl` bundles the ICU data file so it doesn't need to be on the device.

### 2. Verify Android cross-compile still works

```bash
cd /home/harry/wasm-android-runtime/host
cargo build --target aarch64-linux-android 2>&1 | tail -20
```

Use the **cargo-triage** agent if build fails.

### 3. Add paragraph WIT interface

Add the `paragraph` interface to `wit/skiko-gfx.wit` and import it in the world:

```wit
world skiko-ui {
    import canvas;
    import paragraph;   // NEW
    export renderer;
}
```

### 4. Implement Rust paragraph host

In a new file `host/src/paragraph_impl.rs`, implement the WIT paragraph trait
using `skia_safe::textlayout`.

### 5. Add Kotlin stubs

In `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skia/paragraph/`, create:
- `ParagraphBuilder.kt`
- `Paragraph.kt`
- `ParagraphStyle.kt`
- `TextStyle.kt`
- `FontCollection.kt` (stub — delegates to host)

---

## Known issues (anticipated)

### `embed-icudtl` increases binary size by ~10 MB

Acceptable for a development APK. Use `--release` to enable LTO.

### `skia_safe::textlayout` API changes between versions

The `ParagraphBuilder` API changed between skia-safe 0.75 and 0.93.
Check: `grep -r "ParagraphBuilder\|FontCollection" ~/.cargo/registry/src/*/skia-safe-*/src/`

### Android font collection must be initialised with system fonts

```rust
let mut font_mgr = skia_safe::FontMgr::new();
// Load system fonts as we do for text blobs
let collection = Arc::new(skia_safe::textlayout::FontCollection::new());
collection.set_default_font_manager(Some(font_mgr), None);
```

---

## Do NOT

- Start this task to satisfy a hypothetical future need — wait for a concrete
  failing test case.
- Implement RTL/BiDi unless specifically required — it adds significant
  complexity to both the WIT interface and the Rust implementation.
