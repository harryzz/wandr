---
name: compose-row-weight-fillmaxwidth-pattern
description: When putting a Row inside a Column with a child that needs the remaining horizontal space, the Row MUST have `Modifier.fillMaxWidth()` or `Modifier.weight(1f)` on the inner child gets the wrong width — produces word-per-line wrap artifacts
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

When a Compose UI puts a Row inside a Column-shaped parent and wants ONE child of the Row to claim the remaining horizontal space, **both** modifiers are needed:

```kotlin
Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
    Text("•  ", fontSize = 13.sp)            // intrinsic-sized prefix
    Column(modifier = Modifier.weight(1f)) { // claims the rest
        renderTextThatNeedsToWrap()
    }
}
```

**Without `Modifier.fillMaxWidth()` on the Row**, the Row defaults to intrinsic-content width (sum of its children's natural sizes). `Modifier.weight(1f)` on the Column then divides "the remaining width" which is *zero* — the Column gets only what the un-weighted siblings didn't claim, which is effectively the Text's intrinsic width. The Text inside wraps every word.

**Without `Modifier.weight(1f)` on the inner Column**, the Column takes its intrinsic content width and ignores the Row's available space. Same visual symptom.

**Why:** `Why:` Compose Row's measurement is two-pass — first measure non-weighted children at intrinsic width, then distribute remaining space across weighted children. If the Row itself isn't told to fill its parent, "remaining space" is computed against the intrinsic total, which often equals zero or near-zero.

**How to apply:** Any time you build a list/quote/indented-content pattern (bullet, numbered, block-quote with vertical bar, key-value with label/value, …) where the prefix is a fixed text and the content needs to wrap, the prefix-and-content Row needs *both* modifiers — `fillMaxWidth()` on the outer Row, `weight(1f)` on the inner Column.

**Where this bit:** task 38 — MarkdownCard's `RenderBulletList` / `RenderOrderedList` / `RenderBlockQuote` initially showed every word of a bullet on its own line. Diagnosis went from "scroll didn't take" to "build cache?" to "is markdown-renderer breaking paragraphs into per-word blocks?" before the root cause (the Row pattern) clicked. The markdown-renderer ALSO had a real bug (tight-list items: inline children weren't coalesced into one Paragraph) that masked the layout bug — once that was fixed, the layout bug became visible and resisted the `weight(1f)`-alone fix until the Row gained `fillMaxWidth()`.

**Adjacent gotcha:** if the parent of your Row is a `Column { ... }` inside a `Card { ... }` that already has padding/width constraints, the Row's `fillMaxWidth()` is bounded by that — fine, that's what you want.

Related: this is wandr-app/`src/wasmWasiMain/kotlin/MarkdownCard.kt`'s `RenderBulletList`/`RenderOrderedList`/`RenderBlockQuote`.
