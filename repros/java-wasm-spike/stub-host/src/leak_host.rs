// Minimal WASI-floor host for TeaVM WEBASSEMBLY_GC_WASI core modules (task 113).
//
// The core module imports only `wasi_snapshot_preview1.{fd_write, clock_time_get}`
// and exports its linear `memory`. This host implements exactly those two
// functions against the exported memory and calls an exported entry function
// (default `run`). No component model, no adapter, no wasmtime-wasi crate — so it
// cross-compiles to aarch64-linux-android with nothing but wasmtime, and runs the
// same wasm on the device.
//
//   leak-host <module.wasm> [export_name]   (export defaults to `run`)
use std::io::Write;
use wasmtime::*;

fn read_u32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: leak-host <module.wasm> [export]");
    let entry = args.next().unwrap_or_else(|| "run".to_string());

    let mut config = Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_reference_types(true);
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config)?;
    let module = Module::from_file(&engine, &path)?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    // wasi_snapshot_preview1.fd_write(fd, iovs, iovs_len, nwritten) -> errno
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, ()>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return 8, // EBADF-ish: no memory export
            };
            let mut out = Vec::new();
            {
                let data = mem.data(&caller);
                for i in 0..iovs_len as usize {
                    let base = iovs as usize + i * 8;
                    let ptr = read_u32(data, base) as usize;
                    let len = read_u32(data, base + 4) as usize;
                    out.extend_from_slice(&data[ptr..ptr + len]);
                }
            }
            let total = out.len() as u32;
            if fd == 2 {
                let _ = std::io::stderr().write_all(&out);
            } else {
                let mut so = std::io::stdout();
                let _ = so.write_all(&out);
                let _ = so.flush();
            }
            let nw = nwritten as usize;
            mem.data_mut(&mut caller)[nw..nw + 4].copy_from_slice(&total.to_le_bytes());
            0
        },
    )?;

    // wasi_snapshot_preview1.clock_time_get(clock_id, precision, result_ptr) -> errno
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "clock_time_get",
        |mut caller: Caller<'_, ()>, _clock: i32, _prec: i64, result_ptr: i32| -> i32 {
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return 8,
            };
            let rp = result_ptr as usize;
            mem.data_mut(&mut caller)[rp..rp + 8].copy_from_slice(&ns.to_le_bytes());
            0
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let run = instance.get_typed_func::<(), ()>(&mut store, &entry)?;
    eprintln!("[leak-host] calling {entry}() on {path}");
    run.call(&mut store, ())?;
    Ok(())
}
