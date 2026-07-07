//! M1 spike — async TLS transport over p3 wasi:tls + wasi:sockets, NO step-executor.
//! The whole flow is native async (resolve.await / connect.await / handshake.await)
//! driven by the host's shared event loop. VERIFIED end-to-end: a live HTTPS GET to
//! example.com returns "HTTP/1.1 200 OK" through the async TLS receive pipe.
//! See README.md.
wit_bindgen::generate!({ world: "spike", generate_all });

use crate::wasi::sockets::types::{IpAddress, IpSocketAddress, Ipv4SocketAddress,
    IpAddressFamily, TcpSocket};
use crate::wasi::sockets::ip_name_lookup::resolve_addresses;
use crate::wasi::tls::client::Connector;
use wit_bindgen::rt::async_support::spawn;

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

        // 3. Wire the TLS connector's stream transforms to the socket's byte streams.
        //    connector.send:   app cleartext  -> wire ciphertext  (-> socket.send)
        //    connector.receive: wire ciphertext (<- socket.receive) -> app cleartext
        let (net_rx, net_recv_done) = sock.receive();
        let conn = Connector::new();
        let (mut app_tx, app_reader) = wit_stream::new();
        let (wire_ciphertext, enc_done) = conn.send(app_reader);
        let (clear_in, dec_done) = conn.receive(net_rx);
        let net_send_done = sock.send(wire_ciphertext);

        // drive the host-side pipe subtasks concurrently with our read/write
        spawn(async move { let _ = net_recv_done.await; });
        spawn(async move { let _ = enc_done.await; });
        spawn(async move { let _ = dec_done.await; });
        spawn(async move { let _ = net_send_done.await; });

        // 4. Handshake — async (consumes the connector; uses the streams set up above).
        Connector::connect(conn, host.clone()).await.map_err(|e| format!("handshake: {e:?}"))?;

        // 5. Send an HTTP/1.0 GET over cleartext. Keep `app_tx` OPEN — dropping it
        //    tears the connection down before the response arrives.
        let req = format!("GET / HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let _ = app_tx.write_all(req.into_bytes()).await;

        // 6. Read the decrypted response; return the first line (e.g. "HTTP/1.1 200 OK").
        let mut r = clear_in;
        let mut buf = Vec::new();
        while let Some(b) = r.next().await { buf.push(b); if buf.len() >= 256 { break; } }
        let _keep = &app_tx;
        Ok(String::from_utf8_lossy(&buf).lines().next().unwrap_or("<empty>").to_string())
    }
}

export!(Spike);
