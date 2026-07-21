//! wandr.video.player — task 117 M2 stage 2, ON-SCREEN proof.
//!
//! A UI reactor that plays an H.264 clip through `wandr:video`'s playback lane:
//!
//!   submit-timed(PTS) -> next-decoded -> present(at-ns) -> flush(EOS)
//!
//! WHY A UI APP: this began as a `wasi:cli` command, which proved the CONTROL
//! path but showed nothing — `--run-once` guests are surfaceless, so the host
//! stores decoded frames and never runs `composite_video_surfaces`. A reactor has
//! a render loop, which is what actually composites video to the display. The
//! guest itself never sees a pixel (`decoded-frame` is opaque by design), so
//! decode-to-surface is the only visual proof a guest can produce.
//!
//! PUMPED FROM `render-frame`, not a blocking loop: a reactor must return
//! promptly or the whole UI stalls. Each callback submits a bounded amount of
//! input and presents whatever is due — which is how a real player works anyway.
//!
//! Input: `/samples/bbb.h264` (Annex-B), via a read-only `[[mounts]]` entry in
//! package.toml. Demux is guest-side by design, so no sample path is baked into
//! the host.

wit_bindgen::generate!({
    world: "video-player-app",
    path: "wit",
    generate_all,
});

use crate::exports::my::skiko_gfx::frame_pacing::Guest as PacingGuest;
use crate::exports::my::skiko_gfx::renderer::{Guest as RendererGuest, KeyKind, PointerKind};
use crate::wandr::video::decoder::VideoDecoder;
use crate::wandr::video::types::{Codec, DecoderConfig, TimedFrame, VideoError, VideoRect, ZLayer};
use std::cell::RefCell;

const CLIP: &str = "/samples/bbb.h264";
const FPS: i64 = 30;
/// Decode-ahead cushion, in frames. Must exceed the stream's reorder depth or a
/// late earlier-PTS frame arrives after its slot has passed; unbounded would
/// decode the whole file into host memory before showing frame one.
const DECODE_AHEAD: usize = 8;
/// Cap the work done per render-frame so a reactor callback never runs long.
const MAX_SUBMITS_PER_FRAME: usize = 4;

fn pts_us(index: i64) -> i64 {
    index * 1_000_000 / FPS
}

/// Split Annex-B into ACCESS UNITS (one coded picture each): a new AU begins at a
/// VCL NAL when the current AU already holds one. Feeding NALs individually would
/// give several of them the same PTS and make presentation timing meaningless.
fn access_units(buf: &[u8]) -> Vec<(Vec<u8>, bool)> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            let sc = if i > 0 && buf[i - 1] == 0 { i - 1 } else { i };
            starts.push((sc, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let (mut aus, mut cur) = (Vec::new(), Vec::<u8>::new());
    let (mut has_vcl, mut idr) = (false, false);
    for (n, &(sc, payload)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map(|&(s, _)| s).unwrap_or(buf.len());
        let t = buf[payload] & 0x1f;
        let is_vcl = t == 1 || t == 5;
        if is_vcl && has_vcl {
            aus.push((std::mem::take(&mut cur), idr));
            has_vcl = false;
            idr = false;
        }
        cur.extend_from_slice(&buf[sc..end]);
        if is_vcl {
            has_vcl = true;
            idr |= t == 5;
        }
    }
    if !cur.is_empty() {
        aus.push((cur, idr));
    }
    aus
}

struct Player {
    dec: Option<VideoDecoder>,
    aus: Vec<(Vec<u8>, bool)>,
    next_au: usize,
    submitted: usize,
    presented: usize,
    /// Host clock (from `render-frame(nanos)`) at the moment playback began —
    /// our zero point for turning a PTS into "is this frame due yet?".
    origin_ns: u64,
    started: bool,
    flushed: bool,
    done: bool,
    last_pts: i64,
    out_of_order: usize,
    /// Surface geometry, learned from on-resize.
    surface: (u32, u32),
}

impl Player {
    const fn new() -> Self {
        Self {
            dec: None,
            aus: Vec::new(),
            next_au: 0,
            submitted: 0,
            presented: 0,
            origin_ns: 0,
            started: false,
            flushed: false,
            done: false,
            last_pts: -1,
            out_of_order: 0,
            surface: (1280, 720),
        }
    }

    fn start(&mut self, nanos: u64) {
        self.started = true;
        self.origin_ns = nanos;
        let buf = match std::fs::read(CLIP) {
            Ok(b) => b,
            Err(e) => {
                println!("player: cannot read {CLIP}: {e} (is the /samples mount resolving?)");
                self.done = true;
                return;
            }
        };
        self.aus = access_units(&buf);
        println!("player: {} bytes -> {} access units", buf.len(), self.aus.len());

        // Decode-to-surface: a REAL rect is what makes the host composite video.
        // above-ui = the video covers its rect and the UI lays out around it, so
        // we don't need the transparent-hole dance behind-ui requires.
        let (w, h) = self.surface;
        let vh = (w * 9 / 16).min(h);
        match VideoDecoder::open(DecoderConfig {
            codec: Codec::H264,
            width: 1280,
            height: 720,
            rect: VideoRect { x: 0, y: (h.saturating_sub(vh)) / 2, width: w, height: vh },
            rotation: 0,
            layer: ZLayer::AboveUi,
        }) {
            Ok(d) => {
                println!("player: decoder OPEN ✓ decode-to-surface {w}x{vh}");
                self.dec = Some(d);
            }
            Err(e) => {
                println!("player: decoder open FAILED: {e:?}");
                self.done = true;
            }
        }
    }

    /// One pump step. Submits a bounded amount, then presents everything due.
    fn pump(&mut self, nanos: u64) {
        let Some(dec) = self.dec.as_ref() else { return };

        // Feed, keeping only a bounded cushion ahead of what we've shown.
        let mut submits = 0;
        while self.next_au < self.aus.len()
            && submits < MAX_SUBMITS_PER_FRAME
            && self.submitted < self.presented + DECODE_AHEAD
        {
            let (data, keyframe) = &self.aus[self.next_au];
            let frame = TimedFrame {
                data: data.clone(),
                timestamp_us: pts_us(self.next_au as i64),
                keyframe: *keyframe,
            };
            match dec.submit_timed(&frame) {
                Ok(()) => {
                    self.submitted += 1;
                    self.next_au += 1;
                    submits += 1;
                }
                // Back-pressure, NOT loss: retry this same frame next pump. A file
                // player cannot resync at the next keyframe the way RTP can.
                Err(VideoError::QueueFull) => break,
                Err(e) => {
                    println!("player: submit-timed failed at AU {}: {e:?}", self.next_au);
                    self.done = true;
                    return;
                }
            }
        }

        // EOS: without flush, the frames a codec holds back for reordering are
        // never released and the tail of the clip silently disappears.
        if self.next_au >= self.aus.len() && !self.flushed {
            let _ = dec.flush();
            self.flushed = true;
        }

        // Present everything whose PTS is due against the HOST clock.
        let elapsed_us = (nanos.saturating_sub(self.origin_ns) / 1_000) as i64;
        while let Some(frame) = dec.next_decoded() {
            let pts = frame.timestamp_us();
            if pts <= self.last_pts {
                self.out_of_order += 1;
            }
            self.last_pts = pts;
            self.presented += 1;
            if pts > elapsed_us + 250_000 {
                // Too far in the future: hold it by NOT presenting. `present` is
                // the only way to hand it back, so drop the handle instead —
                // which the contract defines as equivalent to `discard`.
                //
                // ‼️ This is the shape of stage-2 finding #1: with a usable
                // `present(at-ns)` we would simply schedule it and let the host
                // show it on time, instead of discarding and re-deciding.
                continue;
            }
            frame.present(0);
        }

        if self.flushed && self.next_au >= self.aus.len() && !self.done {
            // Everything fed and drained.
            if self.presented >= self.aus.len() {
                self.done = true;
                println!(
                    "player: DONE — presented {}/{} frames, out-of-order {} (0 = display order OK)",
                    self.presented,
                    self.aus.len(),
                    self.out_of_order
                );
            }
        }
    }
}

thread_local! {
    static PLAYER: RefCell<Player> = const { RefCell::new(Player::new()) };
}

struct Component;

impl RendererGuest for Component {
    fn render_frame(nanos: u64) {
        PLAYER.with(|p| {
            let mut p = p.borrow_mut();
            if !p.started {
                p.start(nanos);
            }
            p.pump(nanos);
        });

        // No drawing: we import no canvas (see world.wit). The video surface is
        // composited by the host ABOVE our UI buffer, so an empty UI is exactly
        // what we want for a clean proof — anything on screen came from the
        // decoder, not from us.
    }

    fn on_pointer_event(_kind: PointerKind, _x: f32, _y: f32) {}
    fn on_key_event(_kind: KeyKind, _key_code: u32) {}
    /// The host tells us the surface size here — this is how we learn the
    /// geometry without importing a canvas interface.
    fn on_resize(w: u32, h: u32) {
        PLAYER.with(|p| p.borrow_mut().surface = (w.max(1), h.max(1)));
    }
    fn on_scheduled_callback(_id: u32) {}
    fn on_pointer_event_v2(_id: u32, _kind: PointerKind, _x: f32, _y: f32, _pressure: f32) {}
    fn on_key_event_v2(_kind: KeyKind, _code_point: u32, _key_id: u32) {}
    fn on_lifecycle_changed(_state: u32) {}
}

impl PacingGuest for Component {
    /// Drive at the clip's frame rate while playing; idle afterwards. On-demand
    /// pacing is the host's contract — asking for 0 would spin a core.
    fn next_frame_delay() -> u32 {
        PLAYER.with(|p| if p.borrow().done { 250 } else { (1000 / FPS) as u32 })
    }
}

export!(Component);
