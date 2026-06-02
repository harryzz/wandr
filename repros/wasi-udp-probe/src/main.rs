//! UDP transport de-risk for the WebRTC call engine (wasm32-wasip2 guest).
//!
//! wart-host wires `wasi:sockets` (wasmtime-wasi p2) + grants the network
//! (`inherit_network`), so a guest should be able to use `std::net::UdpSocket`
//! directly — the sans-IO `rtc` engine would drive its IO this way. This probe
//! confirms it:
//!   1. LOOPBACK — bind two sockets on 127.0.0.1, send + recv. Proves the
//!      wasi-sockets UDP API (bind/send_to/recv_from) works for the guest.
//!   2. STUN (best-effort) — send a Binding Request to a public STUN server and
//!      parse XOR-MAPPED-ADDRESS. Proves outbound internet UDP + yields our
//!      public (server-reflexive) address — the first thing ICE needs.
//!
//! Run on-device: `wart-host --run-once <app-id>` (or any wasi:cli host).

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

const MAGIC_COOKIE: u32 = 0x2112_A442;

fn main() {
    if let Err(e) = loopback() {
        println!("UDP LOOPBACK FAILED: {e}");
        std::process::exit(1);
    }
    // Outbound is best-effort (needs internet + DNS); never fails the probe.
    match stun_reflexive() {
        Ok(Some(addr)) => println!("UDP OUTBOUND OK — STUN server-reflexive address = {addr}"),
        Ok(None) => println!("UDP OUTBOUND: no STUN response (timeout) — loopback already proved the API"),
        Err(e) => println!("UDP OUTBOUND skipped: {e} — loopback already proved the API"),
    }
}

/// (1) localhost loopback — the core proof.
fn loopback() -> std::io::Result<()> {
    let a = UdpSocket::bind("127.0.0.1:0")?;
    let b = UdpSocket::bind("127.0.0.1:0")?;
    let (a_addr, b_addr) = (a.local_addr()?, b.local_addr()?);
    println!("[udp-probe] bound a={a_addr} b={b_addr}");

    let payload = b"WART-UDP-PROBE";
    let n = a.send_to(payload, b_addr)?;
    println!("[udp-probe] sent {n} bytes a->b");

    let mut buf = [0u8; 64];
    let (rn, from) = b.recv_from(&mut buf)?;
    println!("[udp-probe] recv {rn} bytes from {from}: {:?}", std::str::from_utf8(&buf[..rn]));
    if &buf[..rn] != payload {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "payload mismatch"));
    }
    println!("UDP LOOPBACK OK — wasi-sockets UDP works for the guest");
    Ok(())
}

/// (2) best-effort STUN binding request → server-reflexive address.
fn stun_reflexive() -> std::io::Result<Option<SocketAddr>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    // Non-blocking + poll (what a sans-IO engine uses) so a missing response
    // can't hang. wasip2 std lacks SO_RCVTIMEO, but set_nonblocking maps to the
    // wasi-sockets non-blocking flag; the `?` surfaces it if unsupported.
    sock.set_nonblocking(true)?;

    // Google's public STUN (resolved via wasi ip-name-lookup; needs DNS grant).
    let server = "stun.l.google.com:19302";

    // 20-byte STUN Binding Request: type 0x0001, len 0, magic cookie, txid.
    let txid: [u8; 12] = *b"wart-probe!!";
    let mut req = Vec::with_capacity(20);
    req.extend_from_slice(&0x0001u16.to_be_bytes());
    req.extend_from_slice(&0x0000u16.to_be_bytes());
    req.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    req.extend_from_slice(&txid);
    sock.send_to(&req, server)?;
    println!("[udp-probe] STUN Binding Request sent to {server}");

    // Poll for the response up to ~3s.
    let mut buf = [0u8; 512];
    for _ in 0..60 {
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                println!("[udp-probe] STUN response: {n} bytes from {from}");
                return Ok(parse_xor_mapped_address(&buf[..n]));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Parse XOR-MAPPED-ADDRESS (attr 0x0020) from a STUN success response.
fn parse_xor_mapped_address(msg: &[u8]) -> Option<SocketAddr> {
    if msg.len() < 20 || u16::from_be_bytes([msg[0], msg[1]]) != 0x0101 {
        return None; // not a Binding Success Response
    }
    let mut i = 20; // skip header
    while i + 4 <= msg.len() {
        let atype = u16::from_be_bytes([msg[i], msg[i + 1]]);
        let alen = u16::from_be_bytes([msg[i + 2], msg[i + 3]]) as usize;
        let val = msg.get(i + 4..i + 4 + alen)?;
        if atype == 0x0020 && alen >= 8 && val[1] == 0x01 {
            // IPv4: family(0x01), xport (^ cookie hi16), xip (^ cookie)
            let port = u16::from_be_bytes([val[2], val[3]]) ^ (MAGIC_COOKIE >> 16) as u16;
            let raw = u32::from_be_bytes([val[4], val[5], val[6], val[7]]) ^ MAGIC_COOKIE;
            return Some(SocketAddr::from((Ipv4Addr::from(raw), port)));
        }
        i += 4 + alen + ((4 - (alen % 4)) % 4); // 4-byte aligned
    }
    None
}
