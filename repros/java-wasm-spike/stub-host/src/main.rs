// Task 112 live-vs-dead test: instantiate the TeaVM WasmGC spike with a real
// memory + heap globals + real putcharStdout + stringBuiltinsSupported()->false,
// and TRAPPING STUBS for every other import (teavmJso.*, wasm:js-string.*,
// teavmDate.*, teavm.*). The wasm `start` section runs main at instantiation.
//  - completes (prints) with no trap  => JSO imports are DEAD for pure compute
//  - traps naming a teavmJso/js-string import => that import is LIVE
use std::io::Write;
use wasmtime::*;

fn main() -> anyhow::Result<()> {
    let wasm = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../target/wasm/spike.wasm").to_string()
    });

    let mut config = Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_reference_types(true);
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config)?;
    let module = Module::from_file(&engine, &wasm)?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::<()>::new(&engine);

    // env::memory — provide 64 initial pages (>= imported min 33) for heap headroom.
    let mem = Memory::new(&mut store, MemoryType::new(64, Some(32768)))?;
    linker.define(&mut store, "env", "memory", mem)?;

    // teavmMemory globals: heapOffset past static data, maxSize = i32 max (loader convention).
    let g_heap = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Const),
        Val::I32(33 * 65536))?;
    let g_max = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Const),
        Val::I32(0x7FFF_FFFF))?;
    linker.define(&mut store, "teavmMemory", "heapOffset", g_heap)?;
    linker.define(&mut store, "teavmMemory", "maxSize", g_max)?;
    linker.func_wrap("teavmMemory", "notifyHeapResized", || {})?;

    // real stdout so we SEE main's output
    linker.func_wrap("teavmConsole", "putcharStdout", |c: i32| {
        let _ = std::io::stdout().write_all(&[c as u8]);
        let _ = std::io::stdout().flush();
    })?;
    // string builtins NOT supported -> TeaVM routes strings to the char-array fallback
    linker.func_wrap("teavmJso", "stringBuiltinsSupported", || -> i32 { 0 })?;

    // EVERYTHING else -> trapping stubs (the detector).
    linker.define_unknown_imports_as_traps(&module)?;

    eprintln!("[harness] instantiating (start section runs main)…");
    match linker.instantiate(&mut store, &module) {
        Ok(instance) => {
            eprintln!("[harness] instantiated (JS-free). Calling exported Java fn packed_len…");
            let f = instance.get_typed_func::<i32, i32>(&mut store, "packed_len")?;
            for n in [1, 3, 5, 8, 10] {
                let r = f.call(&mut store, n)?;
                println!("  packed_len({n}) = {r}");
            }
            eprintln!("\n[harness] RESULT: a host (wasmtime) CALLED a Java function and got \
                       results — Java compute runs JS-free under WASI-class wasmtime.");
            Ok(())
        }
        Err(e) => {
            eprintln!("\n[harness] RESULT: trapped during start/main:\n{e:?}");
            eprintln!("[harness] => if the trap names a teavmJso/js-string import, it is LIVE.");
            Ok(())
        }
    }
}
