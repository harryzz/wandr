//! wasi-tls reachability probe.
//!
//! Proves that a `wasm32-wasip2` guest, given only host-delegated
//! `wasi:sockets` + `wasi:tls@0.2.0-draft` (no crypto/TLS compiled into the
//! guest), can DNS-resolve, TCP-connect, complete a TLS handshake, and
//! exchange HTTP bytes with a remote host — the transport layer a Signal
//! client would need.
//!
//! Run (desktop, wasmtime 45 CLI):
//!   cargo build --target wasm32-wasip2 --release
//!   wasmtime run -S inherit-network -S allow-ip-name-lookup -S tls \
//!       target/wasm32-wasip2/release/wasi-tls-probe.wasm
//!
//! The driving code mirrors wasmtime's own `p2_tls_sample_application`
//! test program; helpers are inlined so the probe is self-contained.

wit_bindgen::generate!({
    world: "probe",
    path: "wit",
    features: ["tls"],
    generate_all,
});

use crate::wasi::clocks::monotonic_clock;
use crate::wasi::io::poll::{self, Pollable};
use crate::wasi::io::streams::{InputStream, OutputStream, StreamError};
use crate::wasi::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress,
    Network,
};
use crate::wasi::sockets::tcp::{ShutdownType, TcpSocket};
use crate::wasi::sockets::{instance_network, ip_name_lookup, tcp_create_socket};
use crate::wasi::tls::types::ClientHandshake;

const TIMEOUT_NS: u64 = 30_000_000_000; // 30s — generous for a remote handshake
const PORT: u16 = 443;

// ---- inlined blocking helpers (from wasmtime test-programs) ----

fn block_until(p: &Pollable, timeout: &Pollable) -> Result<(), ErrorCode> {
    let ready = poll::poll(&[p, timeout]);
    assert!(!ready.is_empty());
    match ready[0] {
        0 => Ok(()),
        1 => Err(ErrorCode::Timeout),
        _ => unreachable!(),
    }
}

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

fn resolve_first(net: &Network, name: &str) -> Result<IpAddress, String> {
    let stream = ip_name_lookup::resolve_addresses(net, name)
        .map_err(|e| format!("resolve_addresses: {e:?}"))?;
    let timeout = monotonic_clock::subscribe_duration(TIMEOUT_NS);
    let pollable = stream.subscribe();
    loop {
        match stream.resolve_next_address() {
            Ok(Some(addr)) => return Ok(addr),
            Ok(None) => return Err("DNS returned no addresses".into()),
            Err(ErrorCode::WouldBlock) => {
                block_until(&pollable, &timeout).map_err(|_| "DNS timed out".to_string())?
            }
            Err(e) => return Err(format!("resolve_next_address: {e:?}")),
        }
    }
}

fn tcp_connect(
    net: &Network,
    ip: IpAddress,
) -> Result<(TcpSocket, InputStream, OutputStream), String> {
    let socket = tcp_create_socket::create_tcp_socket(family_of(&ip))
        .map_err(|e| format!("create_tcp_socket: {e:?}"))?;
    let timeout = monotonic_clock::subscribe_duration(TIMEOUT_NS);
    let sub = socket.subscribe();
    socket
        .start_connect(net, sock_addr(ip, PORT))
        .map_err(|e| format!("start_connect: {e:?}"))?;
    loop {
        match socket.finish_connect() {
            Ok((i, o)) => return Ok((socket, i, o)),
            Err(ErrorCode::WouldBlock) => {
                block_until(&sub, &timeout).map_err(|_| "connect timed out".to_string())?
            }
            Err(e) => return Err(format!("finish_connect: {e:?}")),
        }
    }
}

fn tls_handshake(
    server_name: &str,
    tcp_in: InputStream,
    tcp_out: OutputStream,
) -> Result<(crate::wasi::tls::types::ClientConnection, InputStream, OutputStream), String> {
    let future = ClientHandshake::finish(ClientHandshake::new(server_name, tcp_in, tcp_out));
    let timeout = monotonic_clock::subscribe_duration(TIMEOUT_NS);
    let pollable = future.subscribe();
    loop {
        match future.get() {
            None => block_until(&pollable, &timeout).map_err(|_| "handshake timed out".to_string())?,
            Some(Ok(Ok(triple))) => return Ok(triple),
            Some(Ok(Err(e))) => {
                return Err(format!("tls handshake io-error: {}", e.to_debug_string()))
            }
            Some(Err(())) => return Err("tls handshake future already consumed".into()),
        }
    }
}

fn write_all(out: &OutputStream, mut bytes: &[u8]) -> Result<(), String> {
    let timeout = monotonic_clock::subscribe_duration(TIMEOUT_NS);
    let pollable = out.subscribe();
    while !bytes.is_empty() {
        block_until(&pollable, &timeout).map_err(|_| "write timed out".to_string())?;
        let permit = out.check_write().map_err(|e| format!("check_write: {e:?}"))? as usize;
        let len = bytes.len().min(permit);
        let (chunk, rest) = bytes.split_at(len);
        out.write(chunk).map_err(|e| format!("write: {e:?}"))?;
        out.blocking_flush().map_err(|e| format!("flush: {e:?}"))?;
        bytes = rest;
    }
    Ok(())
}

fn read_to_end(input: &InputStream) -> Result<Vec<u8>, String> {
    let mut data = vec![];
    loop {
        match input.blocking_read(64 * 1024) {
            Ok(chunk) => data.extend(chunk),
            Err(StreamError::Closed) => return Ok(data),
            Err(StreamError::LastOperationFailed(e)) => {
                // a TLS close_notify-less shutdown still gives us what we read
                if data.is_empty() {
                    return Err(format!("read: {e:?}"));
                }
                return Ok(data);
            }
        }
    }
}

// ---- the probe ----

fn probe(domain: &str) -> Result<String, String> {
    let net = instance_network::instance_network();
    let ip = resolve_first(&net, domain)?;
    let (socket, tcp_in, tcp_out) = tcp_connect(&net, ip)?;
    let (_conn, tls_in, tls_out) = tls_handshake(domain, tcp_in, tcp_out)?;

    let request = format!(
        "GET / HTTP/1.1\r\nHost: {domain}\r\nUser-Agent: wart-wasi-tls-probe\r\nConnection: close\r\n\r\n"
    );
    write_all(&tls_out, request.as_bytes())?;
    let response = read_to_end(&tls_in)?;
    let _ = socket.shutdown(ShutdownType::Both);

    let text = String::from_utf8_lossy(&response);
    let status = text.lines().next().unwrap_or("<no status line>").to_string();
    Ok(format!(
        "resolved {ip:?} · {} bytes · status: {status}",
        response.len()
    ))
}

fn main() {
    // example.com is the control (known-good public TLS endpoint); chat.signal.org
    // is the real target (Signal's service edge).
    let targets = ["example.com", "chat.signal.org"];
    let mut all_ok = true;
    for domain in targets {
        match probe(domain) {
            Ok(info) => println!("[OK]   {domain} — {info}"),
            Err(e) => {
                all_ok = false;
                println!("[FAIL] {domain} — {e}");
            }
        }
    }
    if all_ok {
        println!("\nTRANSPORT PROVEN: wasi-sockets + wasi-tls handshake + HTTP exchange OK");
    } else {
        println!("\nTRANSPORT INCOMPLETE — see [FAIL] lines above");
        std::process::exit(1);
    }
}
