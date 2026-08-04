# jellyfin-sdk

Async Jellyfin API client SDK for Rust (reqwest-based).

This crate focuses on a high-quality core calling experience (auth, retries, pagination, streaming),
then expands endpoint coverage based on real playback/library workflows.

## Status

Early-stage. Public APIs may change quickly until `1.0`.

## Features

- Async client built on `reqwest` (Rustls) + Jellyfin `Authorization: MediaBrowser ...`
- Runtime token updates (`set_token` / `clear_token`) shared across clones
- Configurable retry/backoff + timeouts
- Pagination helpers for `startIndex`/`limit` + `QueryResult<T>`
- Streaming download helpers (download to file)
- Raw escape hatch (`request/execute/send_json`) for unwrapped endpoints

## Coverage / alignment

See `docs/ALIGNMENT.md` for:
- implemented endpoints by OpenAPI tag
- a user-facing playback UX checklist

Build-time metadata is extracted from `docs/jellyfin-openapi-stable.json` (see `jellyfin_sdk::openapi::*`).

## Alignment

See `docs/ALIGNMENT.md` for a high-level view of what is implemented vs. the OpenAPI spec.

## Quick start

Add the dependency:

```bash
cargo add jellyfin-sdk
```

Basic usage:

```rust
use jellyfin_sdk::JellyfinClient;

# async fn demo() -> jellyfin_sdk::Result<()> {
let client = JellyfinClient::builder("http://localhost:8096")?
    .client_name("my-app")
    .device_name("rust")
    .build()?;

client.set_token("your_access_token");

let info = client.system().get_public_info().await?;
println!("server={:?} version={:?}", info.server_name, info.version);
# Ok(())
# }
```

Run the quickstart example (see `--help` for the full flag list):

```bash
cargo run --example quickstart -- --url http://localhost:8096 --token <token> --list-views
```

## Low-level access

When you need an endpoint that isn't wrapped yet, use the raw request helpers:
- `client.request(Method::GET, "/Some/Path")?`
- `client.send_json(...)` for typed JSON
- `client.execute(...)` for streaming downloads

## Pagination

Jellyfin commonly uses `startIndex` + `limit`. For endpoints that return a `QueryResult<T>` payload,
use `pagination::QueryPager` (or the per-API `pager(...)` helpers).

## MSRV

Rust `1.85` (edition 2024).

## License

Dual-licensed under either:
- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)
