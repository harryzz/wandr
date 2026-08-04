# Changelog conventions

GitHub Releases are generated from `CHANGELOG.md` via `ffurrer2/extract-release-notes`.

## Format

Use headings like:

```md
## Unreleased

## 0.1.0
...
```

## Rules

- Keep entries user-facing (what changed, why it matters).
- Prefer grouping by area (e.g. "Playback", "Subtitles", "Images", "Admin").
- Avoid including tokens, URLs, or private server details.
- When cutting a release:
  1. Move items from `Unreleased` into a new `## X.Y.Z` section.
  2. Commit the changelog update.
  3. Tag `vX.Y.Z` and push the tag.
