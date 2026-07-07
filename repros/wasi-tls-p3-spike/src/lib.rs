//! M1 spike — async TLS transport over p3 wasi:tls + wasi:sockets, NO step-executor.
//! The whole flow is native async (resolve.await / connect.await / handshake.await)
//! driven by the host's shared event loop. Proves the p3 async contracts generate
//! usable bindings and the transport flow type-checks + compiles. See README.md.
wit_bindgen::generate!({ world: "spike", generate_all });

use crate::wasi::sockets::types::{IpAddress, IpSocketAddress, Ipv4SocketAddress,
    IpAddressFamily, TcpSocket};
use crate::wasi::sockets::ip_name_lookup::resolve_addresses;
use crate::wasi::tls::client::Connector;

struct Spike;

impl Guest for Spike {
    async fn run(host: String) -> Result<String, String> {
        // 1. DNS — native async, no poll loop.
        let addrs = resolve_addresses(host.clone()).await.map_err(|e| format!("dns: {e:?}"))?;
        let ip = addrs.into_iter().next().ok_or("no address")?;
        let v4 = match ip { IpAddress::Ipv4(a) => a, _ => return Err("want ipv4".into()) };
        let remote = IpSocketAddress::Ipv4(Ipv4SocketAddress { port: 443, address: v4 });

        // 2. TCP connect — async.
        let sock = TcpSocket::create(IpAddressFamily::Ipv4).map_err(|e| format!("create: {e:?}"))?;
        sock.connect(remote).await.map_err(|e| format!("connect: {e:?}"))?;

        // 3. Network byte streams.
        let (net_rx, _net_recv_done) = sock.receive();          // ciphertext FROM network

        // 4. TLS connector: app cleartext <-> wire ciphertext (stream transforms).
        let conn = Connector::new();
        let (mut app_tx, app_reader) = wit_stream::new();       // we write cleartext here
        let (wire_ciphertext, _enc_done) = conn.send(app_reader);
        let (clear_in, _dec_done) = conn.receive(net_rx);
        let _net_send_done = sock.send(wire_ciphertext);        // ciphertext TO network

        // 5. Handshake — async (consumes the connector).
        Connector::connect(conn, host.clone()).await.map_err(|e| format!("handshake: {e:?}"))?;

        // 6. Send a minimal HTTP/1.0 GET over cleartext, then close it.
        let req = format!("GET / HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let _leftover = app_tx.write_all(req.into_bytes()).await;
        drop(app_tx);                                           // close -> TLS close_notify

        // 7. Read the decrypted response to end-of-stream; return the first line.
        let buf = clear_in.collect().await;
        let text = String::from_utf8_lossy(&buf);
        Ok(text.lines().next().unwrap_or("<empty>").to_string())
    }
}

export!(Spike);
