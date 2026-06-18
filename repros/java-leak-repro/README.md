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

`run()` drives a tight unbounded loop. Each iteration allocates a `Frame` that
**references itself** (`f.next = f`) — a reference cycle — then drops it, so it
becomes unreachable *cyclic* WasmGC garbage. This mirrors the Kotlin reproducer's
per-`suspendCoroutine` continuation/state-machine instance, whose
continuation↔context structure is likewise cyclic. Progress prints every
1,000,000 ticks.

The cycle is the whole point — see "Does it actually leak?" below.

## Does it actually leak?

Yes — **but only because of the cycle.** wasmtime's default WasmGC collector is
**deferred reference counting (DRC)**, which reclaims *acyclic* garbage promptly
but **cannot collect cycles**. Measured RSS (`leak-host`, wasmtime 45):

| variant | desktop x86_64 | device (Pixel 2 XL, 4 GB) |
|---|---|---|
| `Frame` with **no** `next` (acyclic) | **flat ~20 MB** over 92M ticks — no leak | flat |
| `Frame.next = f` (cyclic, this repro) | **0 → ~4 GB in ~10 s**, then a multi-second sweep stalls ticks | **0.5 → 2.2 GB in 8 s** |

So a naive "allocate and drop" loop does **not** leak under wasmtime — the DRC
collector frees it. The leak (bytecodealliance/wasmtime#13403) needs cyclic
garbage: DRC never reclaims it, so it piles up until a GC-heap-grow forces a sweep
that walks an ever-larger over-approximated-roots list (visible as the periodic
tick-stall). To watch the *non*-leaking baseline instead, drop the `f.next = f`
line and rebuild.

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

Watch RSS climb (it reaches multiple GB within ~10 s on desktop), then Ctrl-C.
Expected output:

```
leak-repro(java): self-driving repro starting -- pure allocation loop, WASI stdout only
leak-repro(java): tick #1000000 (sink=499999500000)
leak-repro(java): tick #2000000 (sink=1999999000000)
...
```

Ticks periodically stall for a second or two — that is the DRC sweep walking the
accumulated roots. (`sink` accumulates each `Frame.value` so the field can't be
optimized away.)

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
# run briefly and sample RSS (the leak grows fast on a 4 GB phone)
adb shell 'cd /data/local/tmp/jleak && ./leak-host java-leak-repro.wasm & \
  P=$(pidof leak-host); for i in $(seq 1 8); do sleep 1; \
  grep VmRSS /proc/$P/status; done; pkill -9 leak-host'
```

Verified on a Pixel 2 XL (taimen, aarch64) 2026-06-18: same wasm, real arm64 CPU —
RSS climbs **0.5 GB → 2.2 GB in 8 s** (the acyclic baseline stays flat). The leak
is identical to desktop; the phone just hits memory pressure sooner.

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
