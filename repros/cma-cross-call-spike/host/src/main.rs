//! Task 115 M2a spike host — proves the production driving model:
//! a clone of wandr-host's `make_config()` + CM-async, dual-serve linkers
//! (p2 sync + p3), `block_on(call_async)` frames, and a `run_concurrent`
//! nap-pump between frames (the future standalone.rs nap replacement).
//!
//! Gates (see README):
//!   A. sync-lowered async `init` returns; background task advances BETWEEN
//!      `run-frame` calls (during pumped naps), no AsyncDeadlock.
//!   B. quiescence — with an UNPUMPED nap (plain thread::sleep) the ticker
//!      does NOT advance in the background.
//!   C. a pure-p2 component still instantiates + calls SYNC on the same engine.
use anyhow::Result;
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_tls::{WasiTlsCtx, WasiTlsCtxBuilder, WasiTlsCtxView, WasiTlsView};

struct HostState {
    ctx: WasiCtx,
    table: ResourceTable,
    tls: WasiTlsCtx,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}
impl WasiTlsView for HostState {
    fn tls(&mut self) -> WasiTlsCtxView<'_> {
        WasiTlsCtxView { ctx: &mut self.tls, table: &mut self.table }
    }
}

/// Clone of wandr-host `App::make_config()` (lib.rs) + the two p3-async flags.
fn make_config() -> Config {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    config.async_stack_size(8 * 1024 * 1024);
    config.max_wasm_stack(4 * 1024 * 1024);
    // the p3-async additions:
    config.async_support(true);
    config.wasm_component_model_async(true);
    config
}

fn new_state() -> HostState {
    HostState {
        ctx: WasiCtxBuilder::new()
            .inherit_stdio()
            .inherit_network()
            .allow_ip_name_lookup(true)
            .allow_tcp(true)
            .build(),
        table: ResourceTable::new(),
        tls: WasiTlsCtxBuilder::new().build(),
    }
}

fn main() -> Result<()> {
    let composite = std::env::args().nth(1).expect("usage: host <composite.wasm> <p2sync.wasm>");
    let p2sync = std::env::args().nth(2).expect("usage: host <composite.wasm> <p2sync.wasm>");

    // Models AsyncApp: one current-thread runtime owns every store op.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()?;

    let engine = Engine::new(&make_config())?;
    let mut linker = Linker::<HostState>::new(&engine);
    // Production dual-serve: p2 stays SYNC for every existing guest; p3 is additive.
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;
    wasmtime_wasi_tls::p3::add_to_linker(&mut linker)?;

    // ---- async-flavored composite (UI ◁ engine) ----
    let component = Component::from_file(&engine, &composite)?;
    let mut store = Store::new(&engine, new_state());
    let instance = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), linker.instantiate_async(&mut store, &component)).await
    })??;
    let run_frame = instance.get_typed_func::<(), (u32,)>(&mut store, "run-frame")?;

    // Host starts the engine: call the async-lifted `start` through the
    // composite's re-exported chat interface (the production probe pattern).
    let chat = instance
        .get_export_index(&mut store, None, "demo:cma/chat")
        .ok_or_else(|| anyhow::anyhow!("composite does not re-export demo:cma/chat"))?;
    let start_idx = instance
        .get_export_index(&mut store, Some(&chat), "start")
        .ok_or_else(|| anyhow::anyhow!("chat export has no `start`"))?;
    let start = instance.get_typed_func::<(), ()>(&mut store, start_idx)?;
    rt.block_on(async {
        match tokio::time::timeout(Duration::from_secs(5), async {
            start.call_async(&mut store, ()).await?;
            start.post_return_async(&mut store).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => Err(anyhow::anyhow!(
                "start() timed out — spawned task may be blocking task.return"
            )),
        }
    })?;
    println!("start() returned — engine task spawned");

    let mut call_frame = |store: &mut Store<HostState>| -> Result<u32> {
        rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let (v,) = run_frame.call_async(&mut *store, ()).await?;
                run_frame.post_return_async(&mut *store).await?;
                Ok::<u32, anyhow::Error>(v)
            })
            .await
            .map_err(|_| anyhow::anyhow!("run-frame timed out — sync-lowered init likely blocked until task EXIT (gate A FAIL)"))?
        })
    };

    // ---- Gate A: pumped frames — ticker must advance BETWEEN calls ----
    println!("--- gate A: 10 frames, 200ms PUMPED naps (run_concurrent) ---");
    let t0 = Instant::now();
    let mut last = 0u32;
    let mut ticks_in_naps = 0i64;
    for frame in 0..10 {
        let v = call_frame(&mut store)?;
        let delta = v as i64 - last as i64;
        if frame > 0 {
            ticks_in_naps += delta;
        }
        println!("frame {frame:2}  t={:6.0}ms  ticks={v:3}  (+{delta})", t0.elapsed().as_secs_f64() * 1000.0);
        last = v;
        // the future standalone.rs nap: pump the store's event loop while napping
        rt.block_on(store.run_concurrent(async |_| {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }))?;
    }
    // 9 pumped naps x 200ms @ 100ms/tick => ~18 expected
    println!("gate A ticks accrued across pumped naps: {ticks_in_naps} (expect ~18)");
    let gate_a = ticks_in_naps >= 12;
    println!("gate A (task survives + advances between calls): {}", if gate_a { "PASS" } else { "FAIL" });

    // ---- Gate B: quiescence — UNPUMPED naps must not advance the ticker ----
    println!("--- gate B: 3 frames, 400ms UNPUMPED naps (thread::sleep) ---");
    let mut quiesced = true;
    let mut last_b = call_frame(&mut store)?;
    for frame in 0..3 {
        std::thread::sleep(Duration::from_millis(400));
        let v = call_frame(&mut store)?;
        // one expired timer may complete DURING the call itself; >1 means the
        // task ran in the background of an unpumped nap.
        let delta = v as i64 - last_b as i64;
        println!("frame {frame:2}  ticks={v:3}  (+{delta})");
        if delta > 1 {
            quiesced = false;
        }
        last_b = v;
    }
    println!("gate B (quiescent when host does not pump): {}", if quiesced { "PASS" } else { "FAIL" });

    // ---- Gate C: pure-p2 component stays SYNC on the same engine ----
    println!("--- gate C: p2 sync coexistence ---");
    let comp2 = Component::from_file(&engine, &p2sync)?;
    let mut store2 = Store::new(&engine, new_state());
    let gate_c = (|| -> Result<u32> {
        let inst2 = linker.instantiate(&mut store2, &comp2)?; // SYNC instantiate
        let run2 = inst2.get_typed_func::<(), (u32,)>(&mut store2, "run")?;
        let (v,) = run2.call(&mut store2, ())?; // SYNC call
        run2.post_return(&mut store2)?;
        Ok(v)
    })();
    match &gate_c {
        Ok(v) => println!("gate C (sync instantiate+call on same engine): PASS (run() = {v})"),
        Err(e) => println!("gate C: FAIL — {e:#}"),
    }

    // ---- Gate D (phase 3): live HTTPS over the real wandr-reqwest p3 stack ----
    println!("--- gate D: wandr-reqwest p3 backend (live HTTPS + select-drop torture) ---");
    let target = std::env::args().nth(3).unwrap_or_else(|| "example.com".into());
    let mut call_fetch = |name: &str, store: &mut Store<HostState>| -> Result<String> {
        let idx = instance
            .get_export_index(&mut *store, Some(&chat), name)
            .ok_or_else(|| anyhow::anyhow!("chat export has no `{name}`"))?;
        let f = instance.get_typed_func::<(String,), (Result<String, String>,)>(&mut *store, idx)?;
        rt.block_on(async {
            match tokio::time::timeout(Duration::from_secs(30), async {
                let (r,) = f.call_async(&mut *store, (target.clone(),)).await?;
                Ok::<Result<String, String>, anyhow::Error>(r)
            })
            .await
            {
                Ok(inner) => inner?.map_err(|e| anyhow::anyhow!("guest error: {e}")),
                Err(_) => Err(anyhow::anyhow!("{name} timed out")),
            }
        })
    };
    let gate_d1 = call_fetch("fetch", &mut store);
    match &gate_d1 {
        Ok(line) => println!("gate D1 fetch (Client→http1→tls_p3): PASS — {line}"),
        Err(e) => println!("gate D1 fetch: FAIL — {e:#}"),
    }
    let gate_d2 = call_fetch("fetch-chopped", &mut store);
    match &gate_d2 {
        Ok(line) => println!("gate D2 select-drop torture: PASS — {line}"),
        Err(e) => println!("gate D2 select-drop torture: FAIL — {e:#}"),
    }

    if gate_a && quiesced && gate_c.is_ok() && gate_d1.is_ok() && gate_d2.is_ok() {
        println!("ALL GATES PASS");
        Ok(())
    } else {
        anyhow::bail!("one or more gates failed — see output above")
    }
}
