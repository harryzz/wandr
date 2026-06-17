// Task 113 M5: instantiate the COMPONENT (zero imports) via wasmtime's component
// model and call the WIT export `packed-len` — proving Java → JS-free WasmGC →
// component → host-call-via-WIT works end to end.
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../target/wasm/spike.component.wasm").to_string()
    });
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_reference_types(true);
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &path)?;
    let mut store = Store::new(&engine, ());
    let linker = Linker::new(&engine); // component has ZERO imports
    let instance = linker.instantiate(&mut store, &component)?;
    let f = instance.get_typed_func::<(i32,), (i32,)>(&mut store, "packed-len")?;
    eprintln!("[comp-host] calling component WIT export packed-len…");
    for n in [1, 3, 5, 8, 10] {
        let (r,) = f.call(&mut store, (n,))?;
        f.post_return(&mut store)?;
        println!("  packed-len({n}) = {r}");
    }
    eprintln!("\n[comp-host] RESULT: a host called a Java function through a WIT \
               component interface — full Java→JS-free-WasmGC→component→WIT chain works.");
    Ok(())
}
