// Task 114 P1 host — runs the Swift custom-WIT component: provides WASI (Swift's
// stdlib needs it), implements wandr:swift-spike/host {log, draw-rect}, calls run.
use anyhow::Result;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::add_to_linker_sync;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({ world: "swift-spike", path: "wit" });
}
use bindings::wandr::swift_spike::host::Host;

struct State {
    table: ResourceTable,
    wasi: WasiCtx,
}
impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl Host for State {
    fn log(&mut self, msg: String) {
        println!("[swift→host] log: {msg}");
    }
    fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, argb: u32) {
        println!("[swift→host] draw-rect x={x} y={y} w={w} h={h} argb=0x{argb:08X}");
    }
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../spike.component.wasm".to_string());
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &path)?;

    let mut linker = Linker::<State>::new(&engine);
    add_to_linker_sync(&mut linker)?;
    bindings::SwiftSpike::add_to_linker::<_, HasSelf<State>>(&mut linker, |s| s)?;

    let wasi = WasiCtxBuilder::new().inherit_stdio().build();
    let mut store = Store::new(
        &engine,
        State { table: ResourceTable::new(), wasi },
    );
    let spike = bindings::SwiftSpike::instantiate(&mut store, &component, &linker)?;
    eprintln!("[host] calling Swift `run`…");
    spike.call_run(&mut store)?;
    eprintln!("[host] run returned OK");
    Ok(())
}
