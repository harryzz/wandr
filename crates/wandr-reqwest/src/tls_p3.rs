//! Task 115 — p3 twin of `tls.rs`: async TLS byte stream over **WASI 0.3**
//! `wasi:tls` + `wasi:sockets` native Component-Model streams. Zero
//! step-executor: every await is suspended/resumed by the host's event loop.
//! Same public API as `tls.rs` (`connect` / `write_all` / `read_until` /
//! `read_exact` / `take_buffered` / `read_to_end`), so `http1`, `api`, and the
//! websocket shim compile unchanged. Transport recipe proven live in
//! `repros/wasi-tls-p3-spike` (HTTP 200 over real TLS 1.3).
//!
//! **Cancellation safety by construction:** a dedicated reader task owns the
//! decrypted `StreamReader` and pushes into a shared pushback buffer; `read_*`
//! futures only wait on that buffer. Dropping a pending `read_*` mid-
//! `select!` (the engine's receive loop does this every iteration) can never
//! lose bytes, because the in-flight stream read lives in the reader task,
//! not in the dropped future.

use std::cell::RefCell;
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Poll, Waker};

use wit_bindgen::rt::async_support::{spawn, StreamResult, StreamWriter};

use crate::p3::wasi::sockets::ip_name_lookup::resolve_addresses;
use crate::p3::wasi::sockets::types::{
    IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress, TcpSocket,
};
use crate::p3::wasi::tls::client::Connector;

fn family_of(ip: &IpAddress) -> IpAddressFamily {
    match ip {
        IpAddress::Ipv4(_) => IpAddressFamily::Ipv4,
        IpAddress::Ipv6(_) => IpAddressFamily::Ipv6,
    }
}

fn sock_addr(ip: IpAddress, port: u16) -> IpSocketAddress {
    match ip {
        IpAddress::Ipv4(address) => IpSocketAddress::Ipv4(Ipv4SocketAddress { port, address }),
        IpAddress::Ipv6(address) => IpSocketAddress::Ipv6(Ipv6SocketAddress {
            port,
            address,
            flow_info: 0,
            scope_id: 0,
        }),
    }
}

/// Reader-task ↔ readers shared state. Single-threaded (wasm), so a plain
/// RefCell + manual waker list is sound; borrows are never held across awaits.
#[derive(Default)]
struct Shared {
    buf: Vec<u8>,
    eof: bool,
    wakers: Vec<Waker>,
}

impl Shared {
    fn wake_all(&mut self) {
        for w in self.wakers.drain(..) {
            w.wake();
        }
    }
}

/// An established TLS connection with an internal read-pushback buffer.
pub struct TlsStream {
    shared: Rc<RefCell<Shared>>,
    /// Taken out around the `write_all` await so no RefCell borrow spans a
    /// suspension point. Writes are serialized by the callers (the websocket
    /// shim has one writer loop), matching the p2 impl's implicit contract.
    tx: RefCell<Option<StreamWriter<u8>>>,
}

impl TlsStream {
    pub async fn connect(host: &str, port: u16) -> Result<TlsStream, String> {
        // 1. DNS — native async.
        let addrs = resolve_addresses(host.to_string())
            .await
            .map_err(|e| format!("resolve_addresses: {e:?}"))?;
        let ip = addrs.into_iter().next().ok_or("DNS returned no addresses")?;

        // 2. TCP connect — async.
        let sock = TcpSocket::create(family_of(&ip)).map_err(|e| format!("create: {e:?}"))?;
        sock.connect(sock_addr(ip, port))
            .await
            .map_err(|e| format!("connect: {e:?}"))?;

        // 3. Wire the TLS connector's stream transforms to the socket streams:
        //    app cleartext -> [conn.send] -> wire ciphertext -> socket, and
        //    socket -> [conn.receive] -> app cleartext.
        let (net_rx, net_recv_done) = sock.receive();
        let conn = Connector::new();
        let (app_tx, app_reader) = crate::p3::wit_stream::new();
        let (wire_ciphertext, enc_done) = conn.send(app_reader);
        let (clear_in, dec_done) = conn.receive(net_rx);
        let net_send_done = sock.send(wire_ciphertext);

        // Drive the host-side pipe subtasks concurrently with our I/O.
        spawn(async move {
            let _ = net_recv_done.await;
        });
        spawn(async move {
            let _ = enc_done.await;
        });
        spawn(async move {
            let _ = dec_done.await;
        });
        spawn(async move {
            let _ = net_send_done.await;
        });

        // 4. Handshake (consumes the connector; streams stay live after).
        Connector::connect(conn, host.to_string())
            .await
            .map_err(|e| format!("tls handshake: {}", e.to_debug_string()))?;

        // 5. The reader task: sole owner of the decrypted stream (and of the
        //    socket, which must outlive its child streams). Pushes chunks into
        //    the shared buffer and wakes waiting `read_*` futures.
        let shared = Rc::new(RefCell::new(Shared::default()));
        let sh = Rc::clone(&shared);
        spawn(async move {
            let mut rx = clear_in;
            let mut scratch: Vec<u8> = Vec::with_capacity(64 * 1024);
            loop {
                let (status, buf) = rx.read(scratch).await;
                match status {
                    StreamResult::Complete(_) => {
                        let mut s = sh.borrow_mut();
                        s.buf.extend_from_slice(&buf);
                        s.wake_all();
                        scratch = buf;
                        scratch.clear();
                    }
                    StreamResult::Dropped => {
                        let mut s = sh.borrow_mut();
                        s.eof = true;
                        s.wake_all();
                        break;
                    }
                    StreamResult::Cancelled => unreachable!("read never cancelled"),
                }
            }
            drop(sock); // keep the socket alive for the connection's lifetime
        });

        Ok(TlsStream {
            shared,
            tx: RefCell::new(Some(app_tx)),
        })
    }

    pub async fn write_all(&self, data: &[u8]) -> Result<(), String> {
        let mut tx = self
            .tx
            .borrow_mut()
            .take()
            .ok_or_else(|| "tls write: writer busy or closed".to_string())?;
        let leftover = tx.write_all(data.to_vec()).await;
        let ok = leftover.is_empty();
        *self.tx.borrow_mut() = Some(tx);
        if ok {
            Ok(())
        } else {
            Err("tls stream closed while writing".into())
        }
    }

    /// Wait until the reader task delivers more bytes. Returns false at EOF
    /// (with nothing new buffered). Drop-safe: cancelling just abandons a
    /// stale waker; the underlying stream read lives in the reader task.
    async fn fill(&self) -> Result<bool, String> {
        let start_len = self.shared.borrow().buf.len();
        poll_fn(|cx| {
            let mut s = self.shared.borrow_mut();
            if s.buf.len() > start_len {
                return Poll::Ready(Ok(true));
            }
            if s.eof {
                return Poll::Ready(Ok(false));
            }
            s.wakers.push(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    fn take_from_buf(&self, n: usize) -> Vec<u8> {
        let mut s = self.shared.borrow_mut();
        let rest = s.buf.split_off(n);
        std::mem::replace(&mut s.buf, rest)
    }

    /// Read until `needle` appears; return everything up to and including it,
    /// leaving the remainder buffered.
    pub async fn read_until(&self, needle: &[u8]) -> Result<Vec<u8>, String> {
        loop {
            let found = find_subslice(&self.shared.borrow().buf, needle);
            if let Some(pos) = found {
                return Ok(self.take_from_buf(pos + needle.len()));
            }
            if !self.fill().await? {
                return Err("connection closed before delimiter".into());
            }
        }
    }

    pub async fn read_exact(&self, n: usize) -> Result<Vec<u8>, String> {
        while self.shared.borrow().buf.len() < n {
            if !self.fill().await? {
                return Err("connection closed before reading n bytes".into());
            }
        }
        Ok(self.take_from_buf(n))
    }

    /// Drain whatever is buffered without blocking (≤ buffered bytes).
    pub fn take_buffered(&self) -> Vec<u8> {
        let n = self.shared.borrow().buf.len();
        self.take_from_buf(n)
    }

    pub async fn read_to_end(&self) -> Result<Vec<u8>, String> {
        while self.fill().await? {}
        Ok(self.take_buffered())
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
