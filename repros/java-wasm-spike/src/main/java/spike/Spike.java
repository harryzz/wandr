package spike;

import org.teavm.interop.Export;

public final class Spike {
    // Pure Java arithmetic (no console/heap/clock) — the host will call this.
    @Export(name = "packed-len")
    public static int packedLen(int septets) {
        return (septets * 7 + 7) / 8;
    }

    // task 113 WASI floor: prints via System.out -> WasmGCSupport.putCharStdout
    // -> wasi_snapshot_preview1.fd_write. Pulls the floor into the module.
    @Export(name = "greet")
    public static void greet() {
        System.out.println("hello from java on wasi");
    }

    // task 113 WASI floor: exercises the real clock (clock_time_get).
    @Export(name = "now-millis")
    public static long nowMillis() {
        return System.currentTimeMillis();
    }
    public static void main(String[] args) {
        // entry not used by the test; the export is what we call.
    }
}
