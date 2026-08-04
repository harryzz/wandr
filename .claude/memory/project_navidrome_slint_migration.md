---
name: project_navidrome_slint_migration
description: "NEXT-SESSION plan — rebuild wandr.navidrome (then jellyfin/dash) UI on Slint to fix tiny-font density generally, reusing the media-engine headless audio core."
metadata: 
  node_type: memory
  type: project
  originSessionId: c6bfd2e3-58ed-44e9-8de8-85655ad45867
  modified: 2026-08-04T07:18:17.717Z
---

# Navidrome (+ jellyfin/dash) UI → Slint — fresh-session build plan

**Why (the real problem):** the media-engine apps render a HAND-ROLLED wasi:canvas UI
at fixed px, ignoring display density → text is unreadable on device (Pixel 2 XL is
**1440×2880 @ dpi 560 ≈ 3.5×**). The fix is NOT a scale-factor patch on the canvas
draws — it's replacing the UI with a real framework (**Slint**) that gets density for
free via `wandr:ui-shell/metrics.get-density` (slint-wandr dispatches
`ScaleFactorChanged`). User explicitly chose this over a canvas patch.

**Architecture (keep the split):** `wandr-media-engine` stays a HEADLESS core
(demux/decode/A-V clock/present/audio/HTTP-streaming — all correct, incl. the
device-audio fix `95c1e588`). Only the UI layer changes. **Video is framework-agnostic**:
the engine presents decoded video to a HOST-composited surface via `wandr:video`
`DecoderConfig{ layer: ZLayer::BehindUi }` + `set_rect` ("pixels never enter the guest")
— Signal proves this works under a **dioxus** UI. So Slint works for jellyfin/dash too:
Slint draws chrome, host composites video behind it. My earlier "video-in-Slint unsolved"
was WRONG.

## Order: navidrome first (audio-only, cleanest), then jellyfin/dash (video)

## The Slint-on-wandr skeleton — reference app: `apps/user/wandr.audio.player`
audio.player is a WORKING Slint music player (density-correct on device+desktop, audio
plays). Copy its structure. It has its OWN symphonia+local-file audio, but navidrome must
reuse the ENGINE's NETWORK-streaming audio (audio.player's local-file path won't stream a
Subsonic server; the engine has the async-fetch device fix).

**Cargo.toml** (mirror audio.player):
- `slint = { git = "https://github.com/slint-ui/slint", rev = "46cfde659f21de52bb0fa3693826ca99a6466d88", default-features = false, features = ["compat-1-2","std"] }`
  ‼️ pin the SAME git rev as slint-wandr's i-slint-core (one crate instance = one platform global).
- `slint-wandr = { path = "../../../crates/slint-wandr" }`
- `wandr-media-engine = { path = "../../../crates/wandr-media-engine", default-features = false }` (audio-only profile; `video` OFF)
- `opensubsonic = { path = "../../../crates/opensubsonic-rs" }` + `wandr-reqwest` (package rename) + `serde`/`serde_json`
- `image = { version="0.25", default-features=false, features=["jpeg","png"] }` for cover art → Slint Image
- features: `default=["p3-async"]`, `p3-async=["reqwest/p3-async","wandr-media-engine/p3-async"]`

**wit-p3/world.wit** (the app's second generate!, NOT the Slint one):
```
world navidrome-extras { export wandr:background/background@0.1.0; }
```
NO wasi:audio import (the engine's own imports-only generate! provides it). Copy
`wit-p3/deps/background/` from audio.player. (media-session optional — add later for
lockscreen transport.)

**lib.rs structure:**
```
slint::slint!{ ... UI markup ... }          // density-correct automatically
mod bindings {
    slint_wandr::__wit_bindgen::generate!({ path:"wit-p3", world:"navidrome-extras",
        generate_all, runtime_path:"::slint_wandr::__wit_bindgen::rt" });
    struct Extras;
    impl exports::wandr::background::background::Guest for Extras {
        async fn bg_tick() -> u32 { crate::engine_tick() }
    }
    export!(Extras);
}
slint_wandr::launch!(|| { let ui = MainWindow::new()...; ui.on_row_tap(...); ...; });
slint_wandr::on_lifecycle(|fg| ...);         // optional bg audio-ring switch
```
slint-wandr's `launch!` provides renderer/input/frame-pacing/DENSITY exports; the app
adds only `background/bg_tick`.

## Composition — already de-risked
audio.player composes slint-wandr + **wandr-reqwest** (own imports-only generate!) fine.
`wandr-media-engine` is the same pattern (imports-only `generate!(world:"media-engine-imports")`,
no cabi_realloc export — slint-wandr owns cabi_realloc). So slint-wandr + media-engine +
opensubsonic + Extras compose in one cdylib. If a wit-bindgen runtime clash appears,
that's the thing to debug first, but it shouldn't.

## Engine integration — drive headless audio from bg_tick
`pump_stream(nanos)` needs a monotonic timestamp; bg_tick has none. Use
`std::time::Instant` (wasip2 std maps it to wasi:clocks monotonic — no WIT import):
```
static ORIGIN: OnceCell<Instant> = ...;   // set on first tick
fn engine_tick() -> u32 {
    // 1. spawn opensubsonic driver once; drain pending_nav / pending_play / pending_open
    // 2. if STREAM.is_some(): let nanos = ORIGIN.elapsed().as_nanos() as u64;
    //      engine::pump_stream(nanos);  // audio-only => NO render, pure clock+audio-write
    //      engine::fill_queues(p); engine::decode_audio(p); engine::drive_prefetch(&h);
    //    handle seek_request/stop_requested via engine::CONTROLS; is_ended → auto-advance queue
    // 3. push engine state → Slint props (clock_us, duration_us, title, playing, is_ended)
    8 // delay ms
}
```
Slint UI callbacks set engine::CONTROLS (paused/muted/volume/seek_request/stop_requested)
and app queue state. **`pump_stream` audio-only does NOT render** (render_playing is a
separate fn we DON'T call) — so no canvas conflict with Slint.

## Reuse verbatim from the CURRENT canvas navidrome (git `95c1e588`: apps/user/wandr.navidrome/src/lib.rs)
- `/state/navidrome/config.json` load (serde Config{server,user,pass}); `Auth::plain` (server rejects token, error 41)
- opensubsonic driver: `Client::new(server, Auth::plain).with_client_name("wandr")`, `ping`,
  `get_random_songs` / `get_album_list2(AlphabeticalByName)` / `get_artists` / `get_artist` /
  `get_album` / `get_playlists` / `get_playlist` / `search3` / `get_cover_art`
- `play(idx)`: `client.stream_url(&song.id, None, Some("raw"))` → `engine::net::fetch_range(&hc,&url,0,Some(0))` size probe → `engine::open_audio_sync(url,total,title,dur_us,surface)`; dur_us from `song.duration`
- queue Vec<Child> + qidx, skip(±1), is_ended → auto-advance
- Cover art: fetch `get_cover_art(id, Some(400))` bytes → decode with `image` crate → `SharedPixelBuffer<Rgba8Pixel>` → Slint `Image` (audio.player does exactly this; NOT engine::decode_image under Slint)

## Engine public API (headless audio) — all `pub` in wandr-media-engine
`open_audio_sync`, `pump_stream(nanos)`, `fill_queues(p)`, `decode_audio(p)`,
`drive_prefetch(&h)`, `STREAM` (`RefCell<Option<StreamPlayer>>`), `CONTROLS`,
`StreamPlayer::{clock_us,duration_us,is_ended,prefetch_handle}`, `with_audio`,
`seek_from_clock`, `net::fetch_range`. (`set_surface`/`render_*`/`draw_*`/`decode_image`
are canvas-UI — NOT used under Slint.)

## First increment (de-risk before full UI)
connect → **random songs** ListView (density-correct) → tap plays via engine → now-playing
(title, progress, play/pause, prev/next, seek). Prove density + engine streaming audio
under Slint ON DEVICE. THEN layer back: menu (Albums/Artists/Playlists/Search), cover art,
queue view. (Open question deferred: start-on-albums vs random — pick random for the slice.)

## Deploy/test (this session's proven flow)
- Build: `cargo build --release --target wasm32-wasip2` → `cp target/.../release/*.wasm components/ui.wasm`
- Device: `adb push <stage> /data/local/tmp/<app>` then
  `adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=/data/local/tmp/wandr-apps /data/local/tmp/wandr-host --install /data/local/tmp/<app>'"`,
  launch `wandr-arbiter launch wandr.navidrome`. Guest **stderr**→logcat (engine::log now uses eprintln); stdout→/dev/null.
- Desktop: host `runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host`,
  `WANDR_APPS_ROOT=~/wandr-desktop-apps ... --install`, run via `tools/scripts/run-app-linux.sh wandr.navidrome`.
- Overwrite the canvas navidrome IN PLACE (working version stays in git at `95c1e588`).

## After navidrome: jellyfin/dash on Slint
Same pattern + video: open the engine VideoDecoder (ZLayer::BehindUi, set_rect the video
area), Slint draws transport chrome on top, host composites. Also: **dash Fmp4 SegStream
still block_on's** (its own per-segment reader, separate from the HttpRangeReader fix in
`95c1e588`) — needs the same async-native treatment. See [[reference_media_engine_device_audio_fix]].

Related: [[reference_slint_wasip2]] · [[reference_jellyfin_container_demux_and_mkv_seek]]
· [[feedback_no_hardcoding]] (density = derive from get-density, never hardcode).
