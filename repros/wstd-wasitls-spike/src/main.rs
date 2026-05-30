//! Task-67 transport-foundation spike.
//!
//! Proves the one unproven piece before the libsignal-service-rs transport swap:
//! that `wstd`'s `wasi:io/poll` async executor can drive an **async** HTTP/1.1
//! GET over task-66 `wasi:tls`. The raw reachability (sync) is already proven by
//! `repros/wasi-tls-probe`; what's new here is running it under a real async
//! reactor (the thing libsignal-service's `futures::select!` websocket loop + the
//! `RequestBuilder::send().await` shim will need).
//!
//! Run (desktop, wasmtime 45 CLI):
//!   cargo build --target wasm32-wasip2 --release
//!   wasmtime run -S inherit-network -S allow-ip-name-lookup -S tls \
//!       target/wasm32-wasip2/release/wstd-wasitls-spike.wasm

wit_bindgen::generate!({
    world: "spike",
    path: "wit",
    features: ["tls"],
    generate_all,
});

use crate::wasi::io::poll::Pollable;
use crate::wasi::io::streams::{InputStream, OutputStream, StreamError};
use crate::wasi::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress,
    Ipv6SocketAddress, Network,
};
use crate::wasi::sockets::tcp::{ShutdownType, TcpSocket};
use crate::wasi::sockets::{instance_network, ip_name_lookup, tcp_create_socket};
use crate::wasi::tls::types::{ClientConnection, ClientHandshake};

use wstd::runtime::AsyncPollable;

const PORT: u16 = 443;

/// Bridge: our wit-bindgen `wasi:io/poll.pollable` and wstd's
/// `wasip2::io::poll::Pollable` wrap the *same* component-model resource, so we
/// move the raw handle from ours into wstd's and let its reactor await it.
async fn wait_for(p: Pollable) {
    let handle = p.take_handle();
    let wp = unsafe { wasip2::io::poll::Pollable::from_handle(handle) };
    AsyncPollable::new(wp).wait_for().await;
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
    loop {
        match stream.resolve_next_address() {
            Ok(Some(addr)) => return Ok(addr),
            Ok(None) => return Err("DNS returned no addresses".into()),
            Err(ErrorCode::WouldBlock) => wait_for(stream.subscribe()).await,
            Err(e) => return Err(format!("resolve_next_address: {e:?}")),
        }
    }
}

async fn tcp_connect(
    net: &Network,
    ip: IpAddress,
) -> Result<(TcpSocket, InputStream, OutputStream), String> {
    let socket = tcp_create_socket::create_tcp_socket(family_of(&ip))
        .map_err(|e| format!("create_tcp_socket: {e:?}"))?;
    socket
        .start_connect(net, sock_addr(ip, PORT))
        .map_err(|e| format!("start_connect: {e:?}"))?;
    loop {
        match socket.finish_connect() {
            Ok((i, o)) => return Ok((socket, i, o)),
            Err(ErrorCode::WouldBlock) => wait_for(socket.subscribe()).await,
            Err(e) => return Err(format!("finish_connect: {e:?}")),
        }
    }
}

async fn tls_handshake(
    server_name: &str,
    tcp_in: InputStream,
    tcp_out: OutputStream,
) -> Result<(ClientConnection, InputStream, OutputStream), String> {
    let future =
        ClientHandshake::finish(ClientHandshake::new(server_name, tcp_in, tcp_out));
    loop {
        match future.get() {
            None => wait_for(future.subscribe()).await,
            Some(Ok(Ok((conn, tls_in, tls_out)))) => {
                // Keep `conn` alive and return it: the tls streams are its
                // children, so the connection must outlive them (drop order).
                return Ok((conn, tls_in, tls_out))
            },
            Some(Ok(Err(e))) => {
                return Err(format!("tls handshake io-error: {}", e.to_debug_string()))
            },
            Some(Err(())) => {
                return Err("tls handshake future already consumed".into())
            },
        }
    }
}

async fn write_all(out: &OutputStream, mut bytes: &[u8]) -> Result<(), String> {
    while !bytes.is_empty() {
        wait_for(out.subscribe()).await;
        let permit =
            out.check_write().map_err(|e| format!("check_write: {e:?}"))? as usize;
        if permit == 0 {
            continue;
        }
        let len = bytes.len().min(permit);
        let (chunk, rest) = bytes.split_at(len);
        out.write(chunk).map_err(|e| format!("write: {e:?}"))?;
        bytes = rest;
    }
    out.blocking_flush().map_err(|e| format!("flush: {e:?}"))?;
    Ok(())
}

async fn read_to_end(input: &InputStream) -> Result<Vec<u8>, String> {
    let mut data = vec![];
    loop {
        match input.read(64 * 1024) {
            Ok(chunk) if chunk.is_empty() => wait_for(input.subscribe()).await,
            Ok(chunk) => data.extend(chunk),
            Err(StreamError::Closed) => return Ok(data),
            Err(StreamError::LastOperationFailed(e)) => {
                if data.is_empty() {
                    return Err(format!("read: {}", e.to_debug_string()));
                }
                return Ok(data);
            },
        }
    }
}

async fn probe(domain: &str) -> Result<String, String> {
    let net = instance_network::instance_network();
    let ip = resolve_first(&net, domain).await?;
    let (socket, tcp_in, tcp_out) = tcp_connect(&net, ip).await?;
    // `conn` must outlive tls_in/tls_out (its children); `socket` must outlive
    // `conn` (which owns the tcp streams). Declaration order gives reverse-order
    // drop: tls_out, tls_in, conn, socket — correct.
    let (_conn, tls_in, tls_out) =
        tls_handshake(domain, tcp_in, tcp_out).await?;

    let request = format!(
        "GET / HTTP/1.1\r\nHost: {domain}\r\nUser-Agent: wart-wstd-wasitls-spike\r\nConnection: close\r\n\r\n"
    );
    write_all(&tls_out, request.as_bytes()).await?;
    let response = read_to_end(&tls_in).await?;
    let _ = socket.shutdown(ShutdownType::Both);

    let text = String::from_utf8_lossy(&response);
    let status = text.lines().next().unwrap_or("<no status line>").trim();
    Ok(format!("resolved {ip:?} | {} bytes | status: {status}", response.len()))
}

#[wstd::main]
async fn main() {
    let targets = ["example.com", "chat.signal.org"];
    let mut all_ok = true;
    for domain in targets {
        let line = match probe(domain).await {
            Ok(info) => format!("[wstd-wasitls-spike] [OK]   {domain} - {info}\n"),
            Err(e) => {
                all_ok = false;
                format!("[wstd-wasitls-spike] [FAIL] {domain} - {e}\n")
            },
        };
        eprint!("{line}");
    }
    if all_ok {
        eprintln!("[wstd-wasitls-spike] ASYNC TRANSPORT PROVEN: wstd reactor drove wasi:tls HTTP/1.1");
    } else {
        eprintln!("[wstd-wasitls-spike] INCOMPLETE — see [FAIL] lines");
    }
}
