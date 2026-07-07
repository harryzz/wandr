//! M1 host — drives the p3 wasi:tls transport guest with native async, no step-executor.
//! Links wasmtime_wasi::p3 (sockets/io) + wasmtime_wasi_tls::p3 (tls), runs the Store
//! async, and invokes the guest's `run` async export → a LIVE TLS handshake.
use anyhow::Result;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_tls::{WasiTlsCtx, WasiTlsCtxBuilder, WasiTlsCtxView, WasiTlsView};

struct Host { ctx: WasiCtx, table: ResourceTable, tls: WasiTlsCtx }

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}
impl WasiTlsView for Host {
    fn tls(&mut self) -> WasiTlsCtxView<'_> {
        WasiTlsCtxView { ctx: &mut self.tls, table: &mut self.table }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let wasm = std::env::args().nth(1).expect("usage: host <guest.wasm> [host]");
    let target = std::env::args().nth(2).unwrap_or_else(|| "example.com".into());

    let mut config = Config::new();
    config.async_support(true);
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;

    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;  // std stdio/io @0.2
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;           // sockets/io @0.3
    wasmtime_wasi_tls::p3::add_to_linker(&mut linker)?;

    let ctx = WasiCtxBuilder::new()
        .inherit_stdio()
        .inherit_network()
        .allow_ip_name_lookup(true)
        .allow_tcp(true)
        .build();
    let state = Host { ctx, table: ResourceTable::new(), tls: WasiTlsCtxBuilder::new().build() };
    let mut store = Store::new(&engine, state);

    let component = Component::from_file(&engine, &wasm)?;
    let instance = linker.instantiate_async(&mut store, &component).await?;

    let run = instance
        .get_typed_func::<(String,), (Result<String, String>,)>(&mut store, "run")?;
    let (res,) = run.call_async(&mut store, (target.clone(),)).await?;
    run.post_return_async(&mut store).await?;

    match res {
        Ok(line) => { println!("LIVE TLS OK ({target}) -> {line}"); Ok(()) }
        Err(e) => anyhow::bail!("guest returned error: {e}"),
    }
}
