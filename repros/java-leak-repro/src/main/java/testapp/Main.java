// Java analog of repros/wandr-leak-repro/src/wasmWasiMain/kotlin/Main.kt.
//
// Self-driving minimal reproducer for the wasmtime DRC sweep-cost issue
// (bytecodealliance/wasmtime#13403), written in pure Java and compiled with the
// TeaVM WEBASSEMBLY_GC_WASI target (task 113): a JS-free WasmGC module on the
// real wasi_snapshot_preview1 floor. No host imports beyond WASI stdout
// (System.out.println -> fd_write), no component model, no embedder, no external
// driver.
//
// Each iteration allocates a `Frame` that references ITSELF (f.next = f) -- a
// reference cycle -- then drops it, so it becomes unreachable *cyclic* WasmGC
// garbage. This mirrors the Kotlin reproducer's per-suspendCoroutine
// continuation/state-machine instance (whose continuation<->context structure is
// likewise cyclic).
//
// The cycle matters: wasmtime's default WasmGC collector is deferred reference
// counting (DRC), which reclaims ACYCLIC garbage promptly (a plain `new Frame()`
// loop stays flat at ~20 MB and does NOT leak) but CANNOT collect cycles. So the
// cyclic frames pile up until a GC-heap grow forces a sweep that walks an
// ever-larger over-approximated-roots list (RSS reaches multiple GB in seconds;
// ticks periodically stall during the sweep). Drop `f.next = f` to see the
// non-leaking baseline.
//
// TeaVM's WASI target emits no `_start`, so (unlike the Kotlin repro's plain
// `wasmtime run`) drive it through the exported `run`:
//
//   mvn -q clean package
//   wasmtime run -W all-proposals=y \
//       --invoke 'run()' target/wasm/java-leak-repro.wasm
//
// Let it run a few minutes and watch sweep cost climb, then Ctrl-C.

package testapp;

import org.teavm.interop.Export;

public final class Main {
    // Per-iteration "continuation"-like instance: a fresh WasmGC struct that is
    // unreachable the moment `tick()` consumes it -> collectable garbage.
    static final class Frame {
        long value;
        Frame next; // self-reference -> reference cycle (see awaitNextFrame)
    }

    private static Frame pending;
    private static long frameCount;
    private static long sink; // keeps the allocation observably live (no DCE)

    private static void awaitNextFrame() {
        Frame f = new Frame();
        f.value = frameCount;
        // Self-cycle: mirrors the Kotlin suspendCoroutine state-machine's
        // continuation<->context cycle. wasmtime's deferred-reference-counting
        // WasmGC collector reclaims ACYCLIC garbage fine, but cannot collect
        // cycles -> this is the garbage that actually leaks.
        f.next = f;
        pending = f;
    }

    private static void tick() {
        Frame c = pending;
        pending = null;
        if (c != null) {
            sink += c.value;
            frameCount++;
            // c is now unreachable -> WasmGC garbage
        }
    }

    @Export(name = "run")
    public static void run() {
        System.out.println("leak-repro(java): self-driving repro starting "
                + "-- pure allocation loop, WASI stdout only");
        awaitNextFrame();
        // Unbounded so the WasmGC heap fills and DRC sweeps repeatedly.
        while (true) {
            tick();
            awaitNextFrame();
            if (frameCount % 1_000_000L == 0L) {
                System.out.println("leak-repro(java): tick #" + frameCount + " (sink=" + sink + ")");
            }
        }
    }

    public static void main(String[] args) {
        // Entry not used: the WASI target emits no _start; call run() via --invoke.
    }
}
