//! Host half of the export-record spike: loads the componentized Kotlin
//! guest and hammers the two `wandr:spike/handler` exports with randomized
//! records-with-strings, verifying the returned checksum against a local
//! FNV-1a over the exact bytes sent. A mismatch (or trap) = the host-lowered
//! argument memory got corrupted before the guest finished lifting it.

use anyhow::{Context, Result};
use wasmtime::component::{bindgen, Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

bindgen!({
    path: "../wit",
    world: "spike-guest",
});

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

fn fnv1a(acc: &mut u32, bytes: &[u8]) {
    for b in bytes {
        *acc = (*acc ^ *b as u32).wrapping_mul(16777619);
    }
}

fn checksum(code: &str, text: &str, mods: u32, repeat: bool) -> u32 {
    let mut h: u32 = 2166136261;
    fnv1a(&mut h, code.as_bytes());
    fnv1a(&mut h, text.as_bytes());
    fnv1a(&mut h, &mods.to_le_bytes());
    fnv1a(&mut h, &[repeat as u8]);
    h
}

fn big_checksum(strings: &[&str; 8], mods: u32, ts: u64, repeat: bool) -> u32 {
    let mut h: u32 = 2166136261;
    for s in strings {
        fnv1a(&mut h, s.as_bytes());
    }
    fnv1a(&mut h, &mods.to_le_bytes());
    fnv1a(&mut h, &ts.to_le_bytes());
    fnv1a(&mut h, &[repeat as u8]);
    h
}

/// Deterministic LCG so failures are reproducible.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const CODES: &[&str] = &[
    "KeyA", "KeyQ", "Digit1", "Enter", "Backspace", "ArrowLeft", "Space",
    "ShiftLeft", "ControlRight", "MetaLeft", "F11", "NumpadAdd", "IntlBackslash",
];

fn main() -> Result<()> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config)?;

    let arg1 = std::env::args().nth(1).context("usage: runner <component.wasm|.cwasm> [iters] | runner --precompile <in.wasm> <out.cwasm>")?;

    // AOT mode, mirroring wandr's on-device installer: precompile natively
    // on the target, then run the .cwasm via deserialize.
    if arg1 == "--precompile" {
        let input = std::env::args().nth(2).context("--precompile <in.wasm> <out.cwasm>")?;
        let output = std::env::args().nth(3).context("--precompile <in.wasm> <out.cwasm>")?;
        let bytes = std::fs::read(&input)?;
        let cwasm = engine.precompile_component(&bytes)?;
        std::fs::write(&output, cwasm)?;
        println!("precompiled {input} -> {output}");
        return Ok(());
    }

    let component_path = arg1;
    let iters: u64 = std::env::args().nth(2).map(|s| s.parse()).transpose()?.unwrap_or(100_000);

    let component = if component_path.ends_with(".cwasm") {
        // SAFETY: the .cwasm was produced by this same binary's --precompile
        // (same wasmtime version + config) on this same machine.
        unsafe { Component::deserialize_file(&engine, &component_path)? }
    } else {
        Component::from_file(&engine, &component_path)?
    };
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let wasi = WasiCtxBuilder::new().inherit_stderr().build();
    let mut store = Store::new(&engine, Ctx { wasi, table: ResourceTable::new() });

    let guest = SpikeGuest::instantiate(&mut store, &component, &linker)?;
    let handler = guest.wandr_spike_handler();

    let mut rng = Lcg(0x77A2_D5EE_Du64 ^ 0x1234_5678_9ABC_DEF0);
    let mut ok_strict = 0u64;
    let mut bad_strict = 0u64;
    let mut ok_late = 0u64;
    let mut bad_late = 0u64;
    let mut ok_big = 0u64;
    let mut bad_big = 0u64;
    let mut ok_big_late = 0u64;
    let mut bad_big_late = 0u64;

    for i in 0..iters {
        let code = CODES[rng.below(CODES.len() as u64) as usize].to_string();
        // text length distribution: mostly small, sometimes a few KB,
        // occasionally huge (forces cabi_realloc growth paths).
        let len = match rng.below(100) {
            0..=79 => rng.below(64),
            80..=97 => 64 + rng.below(4096),
            _ => 4096 + rng.below(65536),
        } as usize;
        let mut text = String::with_capacity(len);
        while text.len() < len {
            // mix ASCII + multi-byte so UTF-8 lowering size ≠ char count
            let c = match rng.below(16) {
                0 => 'é',
                1 => '猫',
                2 => '🌍',
                _ => (b'a' + rng.below(26) as u8) as char,
            };
            text.push(c);
        }
        let mods = rng.next() as u32;
        let repeat = rng.below(2) == 1;
        let expect = checksum(&code, &text, mods, repeat);

        let ev = exports::wandr::spike::handler::KeyEvent {
            code: code.clone(),
            text: text.clone(),
            mods,
            repeat,
        };

        let got = handler.call_on_key(&mut store, &ev).map_err(|e| anyhow::anyhow!("on-key trapped: {e:#}"))?;
        if got == expect { ok_strict += 1 } else {
            bad_strict += 1;
            if bad_strict <= 3 {
                eprintln!("[strict] MISMATCH iter={i} code={code} text_len={} expect={expect:08x} got={got:08x}", text.len());
            }
        }

        let got = handler.call_on_key_late_lift(&mut store, &ev).map_err(|e| anyhow::anyhow!("on-key-late-lift trapped: {e:#}"))?;
        if got == expect { ok_late += 1 } else {
            bad_late += 1;
            if bad_late <= 3 {
                eprintln!("[late ] MISMATCH iter={i} code={code} text_len={} expect={expect:08x} got={got:08x}", text.len());
            }
        }

        // ── indirect-args spill leg: 8 strings + u32 + u64 + bool = 19 flat ──
        let mut smalls: Vec<String> = Vec::with_capacity(7);
        for _ in 0..7 {
            let n = rng.below(48) as usize;
            let mut s = String::with_capacity(n);
            while s.len() < n {
                s.push((b'a' + rng.below(26) as u8) as char);
            }
            smalls.push(s);
        }
        let ts = rng.next();
        let strs: [&str; 8] = [
            &code, &smalls[0], &smalls[1], &smalls[2], &smalls[3], &smalls[4], &smalls[5], &smalls[6],
        ];
        let expect_big = big_checksum(&strs, mods, ts, repeat);
        let big = exports::wandr::spike::handler::BigEvent {
            code: code.clone(),
            text: smalls[0].clone(),
            key: smalls[1].clone(),
            locale: smalls[2].clone(),
            layout: smalls[3].clone(),
            compose: smalls[4].clone(),
            dead: smalls[5].clone(),
            ime: smalls[6].clone(),
            mods,
            ts,
            repeat,
        };

        let got = handler.call_on_big(&mut store, &big).map_err(|e| anyhow::anyhow!("on-big trapped: {e:#}"))?;
        if got == expect_big { ok_big += 1 } else {
            bad_big += 1;
            if bad_big <= 3 {
                eprintln!("[big   ] MISMATCH iter={i} expect={expect_big:08x} got={got:08x}");
            }
        }

        let got = handler.call_on_big_late_lift(&mut store, &big).map_err(|e| anyhow::anyhow!("on-big-late-lift trapped: {e:#}"))?;
        if got == expect_big { ok_big_late += 1 } else {
            bad_big_late += 1;
            if bad_big_late <= 3 {
                eprintln!("[bigL  ] MISMATCH iter={i} expect={expect_big:08x} got={got:08x}");
            }
        }

        if (i + 1) % 10_000 == 0 {
            println!(
                "… {}/{iters}  strict ok={ok_strict} bad={bad_strict}  late ok={ok_late} bad={bad_late}  big ok={ok_big} bad={bad_big}  bigL ok={ok_big_late} bad={bad_big_late}",
                i + 1
            );
        }
    }

    println!();
    println!("flat   strict (freeAll→lift→scoped):  ok={ok_strict} bad={bad_strict}");
    println!("flat   late   (freeAll→scoped→lift):  ok={ok_late} bad={bad_late}");
    println!("spill  strict (freeAll→lift→scoped):  ok={ok_big} bad={bad_big}");
    println!("spill  late   (table→scoped→bytes):   ok={ok_big_late} bad={bad_big_late}");
    println!();
    if bad_strict == 0 && bad_big == 0 {
        println!("RESULT: STRICT ORDERING PASSES on BOTH paths — flat AND indirect-args spill");
        if bad_late > 0 || bad_big_late > 0 {
            println!("        (positive controls corrupted: flat {bad_late}x, spill {bad_big_late}x — lift-before-alloc is the contract)");
        }
        Ok(())
    } else {
        println!("RESULT: FAIL — strict ordering corrupted (flat bad={bad_strict}, spill bad={bad_big})");
        std::process::exit(1);
    }
}
