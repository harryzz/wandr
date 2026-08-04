# Release / CI notes

This folder documents the release process used by this repository.

## CI

The GitHub Actions workflow in `.github/workflows/ci.yml` runs:
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo nextest run`
- `cargo test --doc`

## Release

Releases are tag-driven: pushing a tag like `v0.1.2` triggers `.github/workflows/release.yml`.

The workflow will:
1. Validate `Cargo.toml` version matches the tag.
2. Re-run fmt/clippy/tests.
3. Run `cargo package`.
4. Publish to crates.io (requires `CARGO_REGISTRY_TOKEN` secret).
5. Create a GitHub Release using the matching section from `CHANGELOG.md`.

## Before tagging

- Update `CHANGELOG.md` (see `release/ci/changelog.md`).
- Ensure `Cargo.toml` metadata is correct (license, description, docs, repo URL).
- Run locally:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo nextest run`
  - `cargo package`
