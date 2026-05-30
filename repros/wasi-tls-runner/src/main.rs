//! wasi-tls Signal-CA runner.
//!
//! Runs the `wasi-tls-probe` component on wasmtime 45 with a custom
//! `wasmtime_wasi_tls::TlsProvider` whose trust store is the webpki public roots
//! PLUS Signal's pinned self-signed CA (`certs/signal-messenger-ca.pem`). The
//! stock `wasmtime` CLI trusts only public roots, so `chat.signal.org` fails
//! `UnknownIssuer`; with Signal's CA added here, the handshake must succeed.
//!
//! This is the exact, minimal host-side change wart-host would make to let a
//! Signal guest reach the service over host-delegated wasi-sockets + wasi-tls.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{anyhow, ensure, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::bindings::Command;
use wasmtime_wasi::{
    DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};
use wasmtime_wasi_tls::{
    Error as TlsError, TlsProvider, TlsStream, TlsTransport, WasiTlsCtx, WasiTlsCtxBuilder,
    WasiTlsCtxView, WasiTlsView,
};

/// Signal's self-signed service CA (`O=Signal Messenger, LLC, CN=Signal
/// Messenger`), as served in the `chat.signal.org` chain. Production wart should
/// pin this from Signal's own source rather than the live server.
const SIGNAL_CA_PEM: &[u8] = include_bytes!("../certs/signal-messenger-ca.pem");

// ---- custom TLS provider: public roots + Signal's pinned CA ----------------

/// Newtype so we can impl wasi-tls's `TlsStream` on a foreign rustls stream
/// (orphan rule). Delegates all I/O to the inner stream, which is `Unpin`.
struct SignalTlsStream(tokio_rustls::client::TlsStream<Box<dyn TlsTransport>>);

impl AsyncRead for SignalTlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}
impl AsyncWrite for SignalTlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}
impl TlsStream for SignalTlsStream {}

struct SignalTlsProvider {
    config: Arc<rustls::ClientConfig>,
}

impl SignalTlsProvider {
    fn new() -> Result<Self> {
        let mut roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let public = roots.roots.len();
        let mut added = 0usize;
        let mut rd = std::io::BufReader::new(SIGNAL_CA_PEM);
        for cert in rustls_pemfile::certs(&mut rd) {
            roots.add(cert?)?;
            added += 1;
        }
        ensure!(added > 0, "no Signal CA cert parsed from PEM");
        eprintln!("[runner] trust store: {public} public roots + {added} Signal CA cert(s)");
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

impl TlsProvider for SignalTlsProvider {
    // NB: wasi-tls's `BoxFutureTlsStream` alias is pub(crate); write it expanded.
    fn connect(
        &self,
        server_name: String,
        transport: Box<dyn TlsTransport>,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn TlsStream>, TlsError>> + Send>> {
        let config = Arc::clone(&self.config);
        Box::pin(async move {
            let domain = rustls::pki_types::ServerName::try_from(server_name)
                .map_err(|_| TlsError::msg("invalid server name"))?;
            let stream = tokio_rustls::TlsConnector::from(config)
                .connect(domain, transport)
                .await
                .map_err(|e| TlsError::msg(e.to_string()))?;
            Ok(Box::new(SignalTlsStream(stream)) as Box<dyn TlsStream>)
        })
    }
}

// ---- wasi store context ----------------------------------------------------

struct Ctx {
    table: ResourceTable,
    wasi: WasiCtx,
    tls: WasiTlsCtx,
}
impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
impl WasiTlsView for Ctx {
    fn tls(&mut self) -> WasiTlsCtxView<'_> {
        WasiTlsCtxView {
            ctx: &mut self.tls,
            table: &mut self.table,
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let wasm = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: wasi-tls-runner <component.wasm>"))?;

    let mut config = Config::new();
    config.async_support(true);
    let engine = Engine::new(&config)?;

    // Persistence: preopen a host dir as guest `/state` (arg 2, default ./signal-state).
    let state_dir = std::env::args().nth(2).unwrap_or_else(|| "signal-state".into());
    std::fs::create_dir_all(&state_dir)?;
    eprintln!("[runner] state dir: {state_dir} -> guest /state");

    let ctx = Ctx {
        table: ResourceTable::new(),
        wasi: WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .inherit_network() // grant outbound network (default deny-all)
            .allow_ip_name_lookup(true) // grant DNS
            .preopened_dir(&state_dir, "/state", DirPerms::all(), FilePerms::all())?
            .build(),
        tls: WasiTlsCtxBuilder::new()
            .provider(Box::new(SignalTlsProvider::new()?))
            .build(),
    };

    let mut store = Store::new(&engine, ctx);
    let component = Component::from_file(&engine, &wasm)?;

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    let mut opts = wasmtime_wasi_tls::p2::LinkOptions::default();
    opts.tls(true);
    wasmtime_wasi_tls::p2::add_to_linker(&mut linker, &opts)?;

    let command = Command::instantiate_async(&mut store, &component, &linker).await?;
    command
        .wasi_cli_run()
        .call_run(&mut store)
        .await?
        .map_err(|()| anyhow!("guest command exited with non-zero status"))?;
    Ok(())
}
