//! A synchronous `Read + Seek` over HTTP Range — the bridge that lets a sync
//! pull-demuxer (matroska-demuxer / the mp4 crate) run on our async `wasi:tls`
//! fetch.
//!
//! The cache is SHARED (`Rc<RefCell<..>>`) between the reader and a proactive,
//! ASYNC prefetch task: `drive_prefetch` (called from bg-tick) keeps the cache a
//! window ahead of the read position without blocking, so `read()` almost always
//! hits the cache. Only a true miss falls back to a synchronous `block_on` fetch
//! (waitable-set.wait) — legal because the demux runs in the async bg-tick.

use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom};
use std::rc::Rc;

/// Range window per fetch.
const WINDOW: u64 = 2 * 1024 * 1024;
/// Keep the cache at least this far ahead of the read cursor (prefetch target).
const PREFETCH_AHEAD: u64 = 8 * 1024 * 1024;
/// Max cached windows (FIFO). ~24 MiB — covers the prefetch-ahead region plus a
/// few behind, and the mp4 crate's interleaved video/audio read regions.
const MAX_CHUNKS: usize = 12;

/// Shared cache + cursors, reachable from both the reader and the prefetch task.
pub struct ReaderShared {
    /// (base, bytes) windows.
    chunks: Vec<(u64, Vec<u8>)>,
    /// Where the demuxer last read — the prefetch target origin.
    read_pos: u64,
    total_len: u64,
    /// A prefetch fetch is in flight (don't spawn another).
    inflight: bool,
    /// Diagnostics.
    pub blocking_fetches: u64,
    pub prefetches: u64,
}

pub type Shared = Rc<RefCell<ReaderShared>>;

/// Handle kept by the player so bg-tick can drive prefetch for this stream.
#[derive(Clone)]
pub struct PrefetchHandle {
    pub shared: Shared,
    pub url: String,
    pub client: reqwest::Client,
}

pub struct HttpRangeReader {
    shared: Shared,
    url: String,
    client: Option<reqwest::Client>,
    /// LOCAL-FILE mode (transport-exclusion test): when Some, `read` seeks+reads
    /// this file directly; the HTTP cache/prefetch path is bypassed entirely. This
    /// keeps the reader TYPE (and thus the whole demux/audio/clock pipeline)
    /// byte-for-byte identical — only where the bytes come from changes.
    local: Option<std::fs::File>,
    pos: u64,
}

impl HttpRangeReader {
    pub fn new(url: String, total_len: u64, client: reqwest::Client) -> Self {
        let shared = Rc::new(RefCell::new(ReaderShared {
            chunks: Vec::new(),
            read_pos: 0,
            total_len,
            inflight: false,
            blocking_fetches: 0,
            prefetches: 0,
        }));
        Self { shared, url, client: Some(client), local: None, pos: 0 }
    }

    /// Open a LOCAL file as the byte source (same interface; no network).
    pub fn new_local(path: &str, total_len: u64) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let shared = Rc::new(RefCell::new(ReaderShared {
            chunks: Vec::new(),
            read_pos: 0,
            total_len,
            inflight: false,
            blocking_fetches: 0,
            prefetches: 0,
        }));
        Ok(Self { shared, url: path.to_string(), client: None, local: Some(file), pos: 0 })
    }

    /// A handle for bg-tick to drive async prefetch — None in local-file mode.
    pub fn handle(&self) -> Option<PrefetchHandle> {
        let client = self.client.clone()?;
        Some(PrefetchHandle { shared: self.shared.clone(), url: self.url.clone(), client })
    }

    /// Fill the cache for `at` — from disk (local mode) or HTTP (network mode).
    fn block_fetch(&mut self, at: u64) -> io::Result<()> {
        let total = self.shared.borrow().total_len;
        let end = (at + WINDOW - 1).min(total.saturating_sub(1));
        // LOCAL-FILE mode: read a whole WINDOW from disk in one go.
        if let Some(f) = self.local.as_mut() {
            let len = (end - at + 1) as usize;
            let mut bytes = vec![0u8; len];
            f.seek(SeekFrom::Start(at))?;
            let mut filled = 0;
            while filled < len {
                match f.read(&mut bytes[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) => return Err(e),
                }
            }
            bytes.truncate(filled);
            let mut s = self.shared.borrow_mut();
            s.blocking_fetches += 1;
            insert_and_evict(&mut s, at, bytes);
            return Ok(());
        }
        let url = self.url.clone();
        let Some(client) = self.client.clone() else {
            return Err(io::Error::new(io::ErrorKind::Other, "no client (local mode)"));
        };
        let res = wit_bindgen::rt::async_support::block_on(async move {
            crate::net::fetch_range(&client, &url, at, Some(end)).await
        });
        let mut s = self.shared.borrow_mut();
        s.blocking_fetches += 1;
        match res {
            Ok(r) => { insert_and_evict(&mut s, at, r.bytes); Ok(()) }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }
}

fn chunk_covering(chunks: &[(u64, Vec<u8>)], off: u64) -> Option<usize> {
    chunks.iter().position(|(b, d)| off >= *b && off < *b + d.len() as u64)
}

/// The furthest contiguous byte covered by the cache starting from `pos`.
fn contiguous_end(chunks: &[(u64, Vec<u8>)], pos: u64) -> u64 {
    let mut end = pos;
    loop {
        match chunks.iter().find(|(b, d)| end >= *b && end < *b + d.len() as u64) {
            Some((b, d)) => end = *b + d.len() as u64,
            None => return end,
        }
    }
}

fn insert_and_evict(s: &mut ReaderShared, base: u64, bytes: Vec<u8>) {
    if bytes.is_empty() {
        return;
    }
    // Skip if we already fully have this region (prefetch vs fallback race).
    if chunk_covering(&s.chunks, base).is_some()
        && chunk_covering(&s.chunks, base + bytes.len() as u64 - 1).is_some()
    {
        return;
    }
    s.chunks.push((base, bytes));
    // FIFO cap — drop the oldest windows. (Never position-based, so a just-
    // inserted window can't be evicted out from under a backward seek.)
    while s.chunks.len() > MAX_CHUNKS {
        s.chunks.remove(0);
    }
}

/// Proactive, NON-BLOCKING prefetch: if the cache isn't `PREFETCH_AHEAD` beyond
/// the read cursor, spawn an async fetch of the next window. Called from bg-tick.
pub fn drive_prefetch(h: &PrefetchHandle) {
    let start = {
        let s = h.shared.borrow();
        if s.inflight {
            return;
        }
        let end = contiguous_end(&s.chunks, s.read_pos);
        if end >= s.total_len || end.saturating_sub(s.read_pos) >= PREFETCH_AHEAD {
            return; // far enough ahead already
        }
        end
    };
    h.shared.borrow_mut().inflight = true;
    let shared = h.shared.clone();
    let url = h.url.clone();
    let client = h.client.clone();
    reqwest::task::spawn(async move {
        let total = shared.borrow().total_len;
        let end = (start + WINDOW - 1).min(total.saturating_sub(1));
        let res = crate::net::fetch_range(&client, &url, start, Some(end)).await;
        let mut s = shared.borrow_mut();
        s.prefetches += 1;
        if let Ok(r) = res {
            insert_and_evict(&mut s, start, r.bytes);
        }
        s.inflight = false;
    });
}

impl Read for HttpRangeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // NOTE: LOCAL-FILE mode still goes through the WINDOW cache below (via
        // block_fetch) — reading disk in 2 MiB windows, not per-`read` syscalls.
        // The demuxer issues thousands of tiny reads; direct seek+read per call
        // is catastrophic over slow drvfs (/mnt/c).
        {
            let total = self.shared.borrow().total_len;
            if self.pos >= total || buf.is_empty() {
                return Ok(0);
            }
        }
        // Record the read position BEFORE fetching — eviction is keyed on it, so
        // on a backward seek this keeps the window we're about to fetch from being
        // evicted the instant it's inserted (which panicked as "just fetched").
        self.shared.borrow_mut().read_pos = self.pos;
        if chunk_covering(&self.shared.borrow().chunks, self.pos).is_none() {
            self.block_fetch(self.pos)?; // miss → synchronous fallback
        }
        let n = {
            let s = self.shared.borrow();
            let idx = match chunk_covering(&s.chunks, self.pos) {
                Some(i) => i,
                // Defensive: never panic (a guest panic traps the instance).
                None => return Err(io::Error::new(io::ErrorKind::Other, "cache miss after fetch")),
            };
            let (base, data) = &s.chunks[idx];
            let off = (self.pos - base) as usize;
            let n = (data.len() - off).min(buf.len());
            buf[..n].copy_from_slice(&data[off..off + n]);
            n
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpRangeReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let total = self.shared.borrow().total_len;
        let np = match from {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => total as i64 + d,
        };
        if np < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "negative seek"));
        }
        self.pos = np as u64;
        Ok(self.pos)
    }
}
