//! wandr-arbiter-media-session — the now-playing / transport module (task 108 M2).
//!
//! Owns the per-app media-session state (W3C Media Session API shape): metadata
//! (title/artist/album/artwork), playback state, and position. Apps publish
//! through the `wasi:media-session/session` host import, forwarded here as
//! `media-session-set-metadata` / `-set-art` / `-set-state` / `-set-position` /
//! `-clear`. The system chrome (lockscreen / status bar) reads the active
//! session (`media-session-now-playing`, `media-session-artwork` via the
//! host's `wandr:chrome/now-playing` impl) and issues transport intents
//! (`media-session-action <action> <seek|->`), which this module routes to the
//! active session's `session-handler.on-action` (delivered to the owner host).
//!
//! Active-session policy: publishing metadata or going `playing` makes a session
//! active (last writer wins — the player the user just started). On `clear` or
//! `SurfaceRemoved`, the active session falls back to a remaining one (preferring
//! a `playing` one), else none. Text fields arrive percent-encoded; artwork
//! arrives base64'd and is stored + served back verbatim (no decode here — the
//! host decodes for the surfacer), keeping the arbiter dependency-free.

use std::collections::HashMap;

use wandr_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

/// One app's published media session.
#[derive(Default, Clone)]
struct SessionState {
    app_id: String,
    title: String,
    artist: String,
    album: String,
    /// Wire token: "none" | "paused" | "playing".
    state: String,
    duration_s: f64,
    rate: f64,
    position_s: f64,
    /// Artwork as it arrived from the host: percent-encoded mime + base64 data.
    /// Empty when the track has no art.
    art_mime: String,
    art_b64: String,
    /// Monotonic recency stamp — the highest is the most-recently-touched
    /// session, used to pick a fallback when the active one goes away.
    seq: u64,
}

#[derive(Default)]
pub struct MediaSessionModule {
    sessions: HashMap<i32, SessionState>,
    active: Option<i32>,
    seq: u64,
}

impl MediaSessionModule {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Resolve a pid token to its app-id (the host self-reports `getpid()`).
    fn app_id_for(pid: i32, ctx: &Ctx) -> String {
        ctx.store
            .app_by_pid(pid)
            .map(|a| a.app_id.clone())
            .unwrap_or_else(|| pid.to_string())
    }

    /// Get-or-create the session for `pid`, refreshing its recency + app-id.
    fn touch(&mut self, pid: i32, ctx: &Ctx) -> &mut SessionState {
        let seq = self.next_seq();
        let app_id = Self::app_id_for(pid, ctx);
        let s = self.sessions.entry(pid).or_default();
        s.app_id = app_id;
        s.seq = seq;
        s
    }

    /// Drop sessions whose owning app is no longer running (killed / crashed), so
    /// the now-playing surface only ever shows a LIVE player. Backstops the
    /// `SurfaceRemoved` cleanup (covers a stale `active` left by rapid relaunches
    /// or a death notification we processed out of order). Called on every read /
    /// action so the lockscreen self-heals.
    fn prune_dead(&mut self, ctx: &Ctx) {
        self.sessions.retain(|pid, _| ctx.store.app_by_pid(*pid).is_some());
        if let Some(a) = self.active {
            if !self.sessions.contains_key(&a) {
                self.recompute_active();
            }
        }
    }

    /// Pick the active session after a removal: the most-recent remaining one,
    /// preferring a `playing` session over a paused/stopped one.
    fn recompute_active(&mut self) {
        self.active = self
            .sessions
            .iter()
            .max_by_key(|(_, s)| (s.state == "playing", s.seq))
            .map(|(pid, _)| *pid);
    }

    /// `media-session-set-metadata <pid> <enc-title> <enc-artist> <enc-album> <has-art:0|1>`.
    /// Becomes the active session (the user just started/changed this track).
    fn cmd_set_metadata(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() < 4 {
            return Reply::err("media-session-set-metadata-args: <pid> <title> <artist> <album> [has-art]");
        }
        let Ok(pid) = t[0].parse::<i32>() else {
            return Reply::err(format!("media-session-bad-pid {:?}", t[0]));
        };
        let title = dec_field(t[1]);
        let artist = dec_field(t[2]);
        let album = dec_field(t[3]);
        {
            let s = self.touch(pid, ctx);
            s.title = title;
            s.artist = artist;
            s.album = album;
            // New metadata = (usually) a new track — drop stale art; a following
            // `set-art` refills it if the new track has any.
            s.art_mime.clear();
            s.art_b64.clear();
        }
        self.active = Some(pid);
        let app = self.sessions.get(&pid).map(|s| s.app_id.as_str()).unwrap_or("");
        log::info!("arbiter: media-session metadata pid={pid} app={app}");
        Reply::ok(format!("media-session metadata pid={pid}"))
    }

    /// `media-session-set-art <pid> <enc-mime> <b64>` — stored verbatim.
    fn cmd_set_art(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() < 3 {
            return Reply::err("media-session-set-art-args: <pid> <enc-mime> <b64>");
        }
        let Ok(pid) = t[0].parse::<i32>() else {
            return Reply::err(format!("media-session-bad-pid {:?}", t[0]));
        };
        match self.sessions.get_mut(&pid) {
            Some(s) => {
                s.art_mime = t[1].to_string();
                s.art_b64 = t[2].to_string();
                Reply::ok(format!("media-session art pid={pid} bytes={}", s.art_b64.len()))
            }
            None => Reply::err(format!("media-session-set-art-no-session {pid}")),
        }
    }

    /// `media-session-set-state <pid> <none|paused|playing>` — going `playing`
    /// makes this the active session.
    fn cmd_set_state(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("media-session-set-state-args: <pid> <state>");
        }
        let Ok(pid) = t[0].parse::<i32>() else {
            return Reply::err(format!("media-session-bad-pid {:?}", t[0]));
        };
        let state = match t[1] {
            "none" | "paused" | "playing" => t[1].to_string(),
            other => return Reply::err(format!("media-session-bad-state {other:?}")),
        };
        let playing = state == "playing";
        self.touch(pid, ctx).state = state;
        if playing {
            self.active = Some(pid);
        }
        Reply::ok(format!("media-session state pid={pid}"))
    }

    /// `media-session-set-position <pid> <duration-s> <rate> <position-s>`.
    fn cmd_set_position(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 4 {
            return Reply::err("media-session-set-position-args: <pid> <dur> <rate> <pos>");
        }
        let Ok(pid) = t[0].parse::<i32>() else {
            return Reply::err(format!("media-session-bad-pid {:?}", t[0]));
        };
        let dur = t[1].parse::<f64>().unwrap_or(0.0);
        let rate = t[2].parse::<f64>().unwrap_or(1.0);
        let pos = t[3].parse::<f64>().unwrap_or(0.0);
        let s = self.touch(pid, ctx);
        s.duration_s = dur;
        s.rate = rate;
        s.position_s = pos;
        Reply::ok(format!("media-session position pid={pid}"))
    }

    /// `media-session-clear <pid>` — playback ended / session torn down.
    fn cmd_clear(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        let Ok(pid) = args.trim().parse::<i32>() else {
            return Reply::err(format!("media-session-clear-bad-pid {args:?}"));
        };
        self.sessions.remove(&pid);
        if self.active == Some(pid) {
            self.recompute_active();
        }
        Reply::ok(format!("media-session cleared pid={pid}"))
    }

    /// `media-session-now-playing` — one line for the active session (the chrome
    /// feed), or empty when nothing is playing.
    ///   `app=<id> title=<pct> artist=<pct> album=<pct> state=<s> dur=<f> pos=<f> art=<0|1>`
    fn cmd_now_playing(&mut self, ctx: &Ctx) -> Reply {
        self.prune_dead(ctx);
        let Some(pid) = self.active else {
            return Reply::ok(String::new());
        };
        let Some(s) = self.sessions.get(&pid) else {
            return Reply::ok(String::new());
        };
        Reply::ok(format!(
            "app={} title={} artist={} album={} state={} dur={} pos={} art={}",
            s.app_id,
            pct_encode(&s.title),
            pct_encode(&s.artist),
            pct_encode(&s.album),
            if s.state.is_empty() { "none" } else { &s.state },
            s.duration_s,
            s.position_s,
            if s.art_b64.is_empty() { 0 } else { 1 },
        ))
    }

    /// `media-session-artwork` — `<enc-mime> <b64>` for the active session, or
    /// empty. Stored + served verbatim (the host base64-decodes for the surfacer).
    fn cmd_artwork(&mut self, ctx: &Ctx) -> Reply {
        self.prune_dead(ctx);
        let line = self
            .active
            .and_then(|pid| self.sessions.get(&pid))
            .filter(|s| !s.art_b64.is_empty())
            .map(|s| format!("{} {}", s.art_mime, s.art_b64))
            .unwrap_or_default();
        Reply::ok(line)
    }

    /// `media-session-action <action> <seek|->` — a transport tap (lockscreen /
    /// status bar / later headset). Route to the active session's owner host,
    /// which delivers `session-handler.on-action`.
    fn cmd_action(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        self.prune_dead(ctx);
        let Some(pid) = self.active else {
            return Reply::err("media-session-action-no-active-session");
        };
        let mut it = args.split_whitespace();
        let Some(action) = it.next() else {
            return Reply::err("media-session-action-args: <action> [seek]");
        };
        let seek = it.next().unwrap_or("-");
        ctx.deliver_to_host(pid, format!("media-session-action {action} {seek}\n"));
        log::info!("arbiter: media-session-action {action} -> pid={pid}");
        Reply::ok(format!("media-session action={action} pid={pid}"))
    }
}

impl ArbiterModule for MediaSessionModule {
    fn verbs(&self) -> &[&'static str] {
        &[
            "media-session-set-metadata",
            "media-session-set-art",
            "media-session-set-state",
            "media-session-set-position",
            "media-session-clear",
            "media-session-now-playing",
            "media-session-artwork",
            "media-session-action",
        ]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "media-session-set-metadata" => self.cmd_set_metadata(args, ctx),
            "media-session-set-art" => self.cmd_set_art(args, ctx),
            "media-session-set-state" => self.cmd_set_state(args, ctx),
            "media-session-set-position" => self.cmd_set_position(args, ctx),
            "media-session-clear" => self.cmd_clear(args, ctx),
            "media-session-now-playing" => self.cmd_now_playing(ctx),
            "media-session-artwork" => self.cmd_artwork(ctx),
            "media-session-action" => self.cmd_action(args, ctx),
            other => Reply::err(format!("media-session-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut Ctx) {
        if let Event::SurfaceRemoved { pid } = ev {
            if self.sessions.remove(pid).is_some() {
                log::info!("arbiter: media-session dropped (surface removed) pid={pid}");
                if self.active == Some(*pid) {
                    self.recompute_active();
                }
            }
        }
    }
}

/// Conservative percent-encoding: keep `A-Za-z0-9-._`, escape the rest as `%XX`.
/// Mirror of [`pct_decode`] and the host's encoder.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex(b >> 4));
            out.push(hex(b & 0xf));
        }
    }
    out
}

/// Decode a positional set-metadata field — inverse of the host's `enc_field`
/// (the `~` sentinel is an empty field; see media_session_host_impl).
fn dec_field(s: &str) -> String {
    if s == "~" {
        String::new()
    } else {
        pct_decode(s)
    }
}

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (unhex(bytes[i + 1]), unhex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(n: u8) -> char {
    (if n < 10 { b'0' + n } else { b'A' + (n - 10) }) as char
}

fn unhex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wandr_arbiter_core::{AppState, Effect, Registry, Store};
    use std::time::{Instant, SystemTime};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(MediaSessionModule::new()));
        (r, Store::new())
    }
    fn add_app(s: &mut Store, id: &str, pid: i32) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
    }

    #[test]
    fn publish_then_now_playing_reflects_active() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "player", 42);
        r.dispatch_command(
            "media-session-set-metadata",
            &format!("42 {} {} {} 0", pct_encode("My Song"), pct_encode("The Artist"), pct_encode("Album")),
            &mut store,
        )
        .unwrap();
        r.dispatch_command("media-session-set-state", "42 playing", &mut store).unwrap();
        r.dispatch_command("media-session-set-position", "42 180.0 1.0 12.5", &mut store).unwrap();
        let (rep, _) = r.dispatch_command("media-session-now-playing", "", &mut store).unwrap();
        let line = rep.render();
        assert!(line.contains("app=player"), "{line}");
        assert!(line.contains("state=playing"), "{line}");
        assert!(line.contains("pos=12.5"), "{line}");
        assert!(line.contains(&format!("title={}", pct_encode("My Song"))), "{line}");
    }

    #[test]
    fn dead_player_clears_now_playing() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "player", 42);
        r.dispatch_command("media-session-set-metadata", "42 t a al 0", &mut store).unwrap();
        r.dispatch_command("media-session-set-state", "42 playing", &mut store).unwrap();
        // Live → shows.
        let (rep, _) = r.dispatch_command("media-session-now-playing", "", &mut store).unwrap();
        assert!(rep.render().contains("app=player"), "{}", rep.render());
        // The user kills the player (app leaves the registry) — no `clear` sent.
        store.remove_app("player");
        // now-playing self-heals: the dead session is pruned → empty.
        let (rep, _) = r.dispatch_command("media-session-now-playing", "", &mut store).unwrap();
        assert!(!rep.render().contains("app="), "stale now-playing: {}", rep.render());
        // And an action has no live target.
        let (rep, _) = r.dispatch_command("media-session-action", "play -", &mut store).unwrap();
        assert!(matches!(rep, Reply::Err(_)), "{}", rep.render());
    }

    #[test]
    fn action_routes_to_active_owner_host() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "player", 42);
        r.dispatch_command("media-session-set-metadata", "42 t a al 0", &mut store).unwrap();
        let (_, eff) = r.dispatch_command("media-session-action", "play -", &mut store).unwrap();
        assert!(eff.iter().any(
            |e| matches!(e, Effect::HostLine { pid: 42, line } if line == "media-session-action play -\n")
        ));
    }

    #[test]
    fn clear_drops_session_and_recomputes_active() {
        let (mut r, mut store) = reg();
        add_app(&mut store, "a", 1);
        add_app(&mut store, "b", 2);
        r.dispatch_command("media-session-set-metadata", "1 t a al 0", &mut store).unwrap();
        r.dispatch_command("media-session-set-state", "1 paused", &mut store).unwrap();
        r.dispatch_command("media-session-set-metadata", "2 t a al 0", &mut store).unwrap();
        r.dispatch_command("media-session-set-state", "2 playing", &mut store).unwrap();
        // active is 2 (playing). Clear 2 → falls back to 1.
        r.dispatch_command("media-session-clear", "2", &mut store).unwrap();
        let (rep, _) = r.dispatch_command("media-session-now-playing", "", &mut store).unwrap();
        assert!(rep.render().contains("app=a"), "{}", rep.render());
        // Clear 1 too → no active session (empty body, no `app=`).
        r.dispatch_command("media-session-clear", "1", &mut store).unwrap();
        let (rep, _) = r.dispatch_command("media-session-now-playing", "", &mut store).unwrap();
        assert!(!rep.render().contains("app="), "{:?}", rep.render());
    }
}
