# Task 40 — emoji-picker system component

> **Status:** 🟡 in progress — second concrete system component;
> repeats the cross-app dep pattern with a non-markdown driver and
> forces the generic dep wiring refactor (task 39).

## Why this matters

Task 36 / 38 shipped the markdown cross-app dep end-to-end, but as a
single example. To prove the cross-app dep architecture is REPEATABLE
(not markdown-specific) and to force the generic dep wiring (task 39)
to land, we need a second concrete system component.

Emoji picker is the cleanest second choice:
- Static data — no I/O / state complications inside the component.
- Visual demo is obvious (grid of emojis in a Compose card).
- Different WIT shape than markdown (list of records vs structured
  tree) — exercises canonical-ABI patterns the markdown lift didn't.
- Single self-contained Rust component (~100 LoC + a curated emoji
  table). No host changes needed beyond task 39.

## WIT contract

`wit/emoji.wit`:

```wit
package war:emoji@0.1.0;

interface picker {
    /// One emoji with display metadata.
    record emoji {
        glyph: string,         // the emoji itself, e.g. "😀"
        name:  string,         // CLDR short name, e.g. "grinning face"
        category: string,      // e.g. "Smileys & Emotion"
    }

    /// Flat list of all emojis. Caller can group by `category` for UI.
    /// Curated subset (~60-80) — full Unicode emoji set is overkill
    /// for a first cut; revisit if real demand appears.
    list-all: func() -> list<emoji>;
}

world picker-world {
    export picker;
}
```

## Component implementation

`emoji-picker/` (own git repo + Codeberg sibling, same pattern as
`markdown-renderer/` and `md-smoke-rust/`).

- Rust cdylib, `wasm32-wasip2` target.
- `wit-bindgen::generate!{ world: "picker-world", path: "../wit/emoji.wit" }`.
- Curated `static EMOJI: &[...]` table embedded in the binary.
- ~60-80 emojis across Smileys/People/Animals/Food/Activities/Objects.

## Consumer changes (wart-app)

- `src/wasmWasiMain/kotlin/EmojiImports.kt` (new) — hand-written
  Kotlin/Wasm canonical-ABI lift for `list-all`. Each emoji is a
  record { glyph: string, name: string, category: string } — 24 bytes
  per record (three strings @ 8 bytes each, align 4).
- `src/wasmWasiMain/kotlin/EmojiCard.kt` (new) — Compose card that
  calls `listAllEmojis()` at composition, renders a grid of N emojis
  per row.
- `wit/wart-app.wit` adds `import war:emoji/picker@0.1.0;`.
- `wit/deps/emoji/emoji.wit` (new, copy of `~/wart/wit/emoji.wit`).
- `RealComposeApp.kt` adds `EmojiCard()` adjacent to `MarkdownCard()`.

## Package + install

- New warpkg: `emoji.warpkg` with `kind = "system"` + composition
  `same-store`.
- wart-app's `package.toml` gains a second `[dependencies.emoji]`
  entry → resolver picks up the new system bundle, wires it in
  alongside markdown via the (now-generic) `wire_dep_into_linker`.

## Verification

1. `cargo build` clean across all repos.
2. Install: `wart-host --install` succeeds for the new emoji bundle.
3. `cache-key.toml` for wart-app shows BOTH `markdown` and `emoji`
   under `[dependencies_resolved]`.
4. `wart-host --standalone --app com.example.wart-app` boots; logcat
   shows both deps loaded + instantiated + wired.
5. Screenshot: EmojiCard rendered alongside MarkdownCard.

## Out of scope (v1)

- Tap-to-insert into BasicTextField — saved for v2 if there's demand.
- Category tabs + search — overkill for the proof.
- Skin-tone variants — Unicode complexity not needed for first cut.
- Full Unicode emoji table — curated subset is fine.
- Persistent recently-used list — needs storage (separate-Store).

## Related

- `tasks/39-generic-dep-wiring.md` — co-shipping companion.
- `tasks/36-cross-app-deps.md` — the cross-app dep machinery this
  depends on.
- `tasks/41-system-fonts.md` — next system component after this.
