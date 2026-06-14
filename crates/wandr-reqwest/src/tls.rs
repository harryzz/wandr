//! Async TLS byte stream over task-66 `wasi:tls` + the `wandr-step-executor`
//! `wasi:io/poll` reactor (persistent + frame-stepped, so the Signal engine's
//! background tasks survive across `chat.poll-events` calls). Proven in
//! `repros/wstd-wasitls-spike`. Shared by the HTTP/1.1 client (`http1`) and the
//! websocket shim.

use std::cell::RefCell;

use wandr_step_executor::AsyncPollable;

use crate::wasi::io::poll::Pollable;
use crate::wasi::io::streams::{InputStream, OutputStream, StreamError};
use crate::wasi::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress,
    Ipv6SocketAddress, Network,
};
use crate::wasi::sockets::tcp::TcpSocket;
use crate::wasi::sockets::{instance_network, ip_name_lookup, tcp_create_socket};
use crate::wasi::tls::types::{ClientConnection, ClientHandshake};

/// Bridge our wit-bindgen `wasi:io/poll.pollable` into the step-executor's
/// reactor: the two crates wrap the same component resource, so we move the raw
/// handle.
fn schedule(p: Pollable) -> AsyncPollable {
    let handle = p.take_handle();
    let wp = unsafe { wasip2::io::poll::Pollable::from_handle(handle) };
    AsyncPollable::new(wp)
}

fn family_of(ip: &IpAddress) -> IpAddressFamily {
    match ip {
        IpAddress::Ipv4(_) => IpAddressFamily::Ipv4,
        IpAddress::Ipv6(_) => IpAddressFamily::Ipv6,
    }
}

fn sock_addr(ip: IpAddress, port: u16) -> IpSocketAddress {
    match ip {
        IpAddress::Ipv4(address) => {
            IpSocketAddress::Ipv4(Ipv4SocketAddress { port, address })
        },
        IpAddress::Ipv6(address) => IpSocketAddress::Ipv6(Ipv6SocketAddress {
            port,
            address,
            flow_info: 0,
            scope_id: 0,
        }),
    }
}

async fn resolve_first(net: &Network, name: &str) -> Result<IpAddress, String> {
    let stream = ip_name_lookup::resolve_addresses(net, name)
        .map_err(|e| format!("resolve_addresses: {e:?}"))?;
    let pollable = schedule(stream.subscribe());
    loop {
        match stream.resolve_next_address() {
            Ok(Some(addr)) => return Ok(addr),
            Ok(None) => return Err("DNS returned no addresses".into()),
            Err(ErrorCode::WouldBlock) => pollable.wait_for().await,
            Err(e) => return Err(format!("resolve_next_address: {e:?}")),
        }
    }
}

/// An established TLS connection with an internal read-pushback buffer.
///
/// Field order is the drop order (component-model: a parent resource must outlive
/// its children): pollables (children of the streams) → tls streams (children of
/// the connection) → connection (owns the tcp streams, children of the socket) →
/// socket.
pub struct TlsStream {
    in_pollable: AsyncPollable,
    out_pollable: AsyncPollable,
    input: InputStream,
    output: OutputStream,
    _conn: ClientConnection,
    _socket: TcpSocket,
    buf: RefCell<Vec<u8>>,
}

impl TlsStream {
    pub async fn connect(host: &str, port: u16) -> Result<TlsStream, String> {
        let net = instance_network::instance_network();
        let ip = resolve_first(&net, host).await?;

        let socket = tcp_create_socket::create_tcp_socket(family_of(&ip))
            .map_err(|e| format!("create_tcp_socket: {e:?}"))?;
        socket
            .start_connect(&net, sock_addr(ip, port))
            .map_err(|e| format!("start_connect: {e:?}"))?;
        let (tcp_in, tcp_out) = loop {
            match socket.finish_connect() {
                Ok(io) => break io,
                Err(ErrorCode::WouldBlock) => {
                    schedule(socket.subscribe()).wait_for().await
                },
                Err(e) => return Err(format!("finish_connect: {e:?}")),
            }
        };

        let future =
            ClientHandshake::finish(ClientHandshake::new(host, tcp_in, tcp_out));
        let (conn, tls_in, tls_out) = loop {
            match future.get() {
                None => schedule(future.subscribe()).wait_for().await,
                Some(Ok(Ok(triple))) => break triple,
                Some(Ok(Err(e))) => {
                    return Err(format!(
                        "tls handshake: {}",
                        e.to_debug_string()
                    ))
                },
                Some(Err(())) => {
                    return Err("tls handshake future consumed".into())
                },
            }
        };

        let in_pollable = schedule(tls_in.subscribe());
        let out_pollable = schedule(tls_out.subscribe());
        Ok(TlsStream {
            in_pollable,
            out_pollable,
            input: tls_in,
            output: tls_out,
            _conn: conn,
            _socket: socket,
            buf: RefCell::new(Vec::new()),
        })
    }

    pub async fn write_all(&self, mut data: &[u8]) -> Result<(), String> {
        while !data.is_empty() {
            self.out_pollable.wait_for().await;
            let permit = self
                .output
                .check_write()
                .map_err(|e| format!("check_write: {e:?}"))?
                as usize;
            if permit == 0 {
                continue;
            }
            let n = data.len().min(permit);
            self.output
                .write(&data[..n])
                .map_err(|e| format!("write: {e:?}"))?;
            data = &data[n..];
        }
        self.output
            .blocking_flush()
            .map_err(|e| format!("flush: {e:?}"))?;
        Ok(())
    }

    /// Pull one chunk from the socket into `buf`. Returns false at EOF.
    async fn fill(&self) -> Result<bool, String> {
        loop {
            match self.input.read(64 * 1024) {
                Ok(chunk) if chunk.is_empty() => {
                    self.in_pollable.wait_for().await
                },
                Ok(chunk) => {
                    self.buf.borrow_mut().extend_from_slice(&chunk);
                    return Ok(true);
                },
                Err(StreamError::Closed) => return Ok(false),
                Err(StreamError::LastOperationFailed(e)) => {
                    return Err(format!("read: {}", e.to_debug_string()))
                },
            }
        }
    }

    fn take_from_buf(&self, n: usize) -> Vec<u8> {
        let mut buf = self.buf.borrow_mut();
        let rest = buf.split_off(n);
        std::mem::replace(&mut *buf, rest)
    }

    /// Read until `needle` appears; return everything up to and including it,
    /// leaving the remainder buffered.
    pub async fn read_until(&self, needle: &[u8]) -> Result<Vec<u8>, String> {
        loop {
            // Bind to a local so the `Ref` is dropped before `take_from_buf`
            // borrows the cell mutably (else: "RefCell already borrowed").
            let found = find_subslice(&self.buf.borrow(), needle);
            if let Some(pos) = found {
                return Ok(self.take_from_buf(pos + needle.len()));
            }
            if !self.fill().await? {
                return Err("connection closed before delimiter".into());
            }
        }
    }

    pub async fn read_exact(&self, n: usize) -> Result<Vec<u8>, String> {
        while self.buf.borrow().len() < n {
            if !self.fill().await? {
                return Err("connection closed before reading n bytes".into());
            }
        }
        Ok(self.take_from_buf(n))
    }

    /// Drain whatever is buffered without blocking (≤ buffered bytes).
    pub fn take_buffered(&self) -> Vec<u8> {
        let n = self.buf.borrow().len();
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
