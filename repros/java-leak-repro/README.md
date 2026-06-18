# java-leak-repro

The **Java** analog of [`repros/wandr-leak-repro`](../wandr-leak-repro) (which is
Kotlin/Wasm). Same self-driving reproducer for the wasmtime DRC sweep-cost issue
([bytecodealliance/wasmtime#13403](https://github.com/bytecodealliance/wasmtime/issues/13403)),
but written in pure Java and compiled with the TeaVM **`WEBASSEMBLY_GC_WASI`**
target (task 113) — a JS-free WasmGC module on the real `wasi_snapshot_preview1`
floor.

It exists to (a) give the leak repro a second language front-end, and (b) exercise
the task-113 WASI floor with a real, non-trivial guest: continuous WasmGC
allocation + `System.out.println` over the `fd_write` floor, no host imports
beyond WASI.

## What it does

`run()` drives a tight unbounded loop. Each iteration allocates one short-lived
`Frame` (the Java stand-in for the Kotlin reproducer's per-`suspendCoroutine`
continuation / state-machine instance), reads it, then drops it — so it becomes
unreachable WasmGC garbage. With wasmtime's DRC collector nothing sweeps until a
GC-heap grow fails, so garbage piles up and each successive sweep walks an
ever-larger over-approximated-roots list. Progress prints every 1,000,000 ticks.

## Imports

Only the WASI floor — verify with `wasm-tools print`:

```
(import "wasi_snapshot_preview1" "fd_write" ...)        ; System.out.println
(import "wasi_snapshot_preview1" "clock_time_get" ...)  ; via TeaVM's EventQueue scheduler
```

No `teavmJso`, no `wasm:js-string`, no `teavmConsole`/`teavmDate`/`teavmMemory` —
the JS-embedding runtime is gone. (`clock_time_get` is pulled in by TeaVM's
always-present `EventQueue`, which reads the wall clock; it is a genuine runtime
dependency, not this repro calling the clock.)

## Build

Needs the task-113 TeaVM fork (`harryzz/teavm:wasmgc-wasi-poc`, commit `fd17b58`)
published to the local Maven repo — same prerequisite as
[`repros/java-wasm-spike`](../java-wasm-spike). Then:

```bash
mvn -q clean package
# -> target/wasm/java-leak-repro.wasm
```

## Run

TeaVM's WASI target emits no `_start`, so (unlike the Kotlin repro's plain
`wasmtime run`) drive it through the exported `run`:

```bash
wasmtime run -W all-proposals=y --invoke run target/wasm/java-leak-repro.wasm
```

Let it run a few minutes and watch sweep cost climb, then Ctrl-C. Expected output:

```
leak-repro(java): self-driving repro starting -- pure allocation loop, WASI stdout only
leak-repro(java): tick #1000000 (sink=499999500000)
leak-repro(java): tick #2000000 (sink=1999999000000)
...
```

(`sink` accumulates each `Frame.value` so the allocation can't be optimized away.)

### On device (Pixel 2 XL, aarch64)

The core module needs only `fd_write` + `clock_time_get`, so a tiny wasmtime host
that implements just those two against the guest's exported memory runs it on the
phone — no component model, no `wasmtime-wasi`. That host is
[`../java-wasm-spike/stub-host`](../java-wasm-spike/stub-host)'s `leak-host` bin.

```bash
# cross-compile the host for the device
cd ../java-wasm-spike/stub-host
source ../../../tools/scripts/env-android.sh
cargo build --release --bin leak-host --target aarch64-linux-android

# push host + the (portable) wasm and run on the phone
adb shell mkdir -p /data/local/tmp/jleak
adb push target/aarch64-linux-android/release/leak-host /data/local/tmp/jleak/leak-host
adb push ../../java-leak-repro/target/wasm/java-leak-repro.wasm /data/local/tmp/jleak/
adb shell chmod 755 /data/local/tmp/jleak/leak-host
adb shell 'cd /data/local/tmp/jleak && timeout 6 ./leak-host java-leak-repro.wasm'
```

Verified on a Pixel 2 XL (taimen, aarch64) 2026-06-18: self-drives at ~9M
ticks/6s (vs ~14M on desktop x86_64), identical `sink` values — same wasm, real
arm64 CPU.

### As a component

To run it the way wandr consumes guests (Component Model), embed the WIT
([`wit/leak.wit`](wit/leak.wit)) and wrap it with the preview1→preview2 adapter:

```bash
wasm-tools component embed wit target/wasm/java-leak-repro.wasm -o target/leak.embed.wasm
wasm-tools component new target/leak.embed.wasm \
  --adapt wasi_snapshot_preview1=../../external/skiko/wasi_snapshot_preview1.reactor.wasm \
  -o target/java-leak-repro.component.wasm
wasmtime run -W all-proposals=y --invoke 'run()' target/java-leak-repro.component.wasm
```
