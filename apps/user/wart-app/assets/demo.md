# Cross-app dep + asset demo

This text lives in **`assets/demo.md`** — shipped inside the warpkg,
copied to the install dir, read at runtime by *the host* via the
**`my:skiko-gfx/assets.read`** WIT verb, then handed to
`war:markdown/renderer.render` to produce this *AnnotatedString*.

## What this proves

Three layers cooperating in one app load:

- The **installer** copied `assets/` from the bundle to the install dir.
- The **host** reads `demo.md` via the `assets` WIT interface
  (`wart-host/src/assets_impl.rs`), enforcing the path-safety guard.
- The **cross-app dep** `war:markdown/renderer` parses the source and
  returns the structured `document` tree the Compose card renders here.

## Iterating on this file

1. Edit `wart-app/assets/demo.md`.
2. Re-run the install pipeline (rebuild wart-app, repackage warpkg,
   reinstall — `scripts/smoke-markdown.sh` plus the standalone launch).
3. Re-launch `wart-host --standalone --app com.example.wart-app`.
4. This card re-reads the file at composition time.

> Wart's `[[assets]]` table doesn't exist yet — the installer
> auto-detects an `assets/` dir at the bundle root. Eventually we may
> want explicit declarations + per-asset metadata (mime type, locale,
> density variants for bitmaps, etc.) — out of scope for task 38's
> first cut.

```kotlin
val source = readAsset("demo.md")?.let { it.decodeToString() }
    ?: "<asset missing>"
val doc = renderDocument(source)
```

---

That `<hr>` came from a thematic break in the source.
