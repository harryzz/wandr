# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API may change rapidly until `1.0`.

## Unreleased

- TBD

## 0.1.0

Initial release.

Highlights:
- Async client (`reqwest` + Rustls) with Jellyfin-style `Authorization: MediaBrowser ...`.
- Configurable retry/backoff, timeouts, pagination, and streaming download helpers.
- Core library browsing and playback building blocks (views/items/playback info/playstate).
- Subtitles: stream/HLS, remote search/download/attach, upload/delete, and fallback fonts.
- Images: item images and `GET/HEAD /UserImage`.
