package spike;

import org.teavm.interop.Export;

public final class Spike {
    // Pure Java arithmetic (no console/heap/clock) — the host will call this.
    @Export(name = "packed-len")
    public static int packedLen(int septets) {
        return (septets * 7 + 7) / 8;
    }
    public static void main(String[] args) {
        // entry not used by the test; the export is what we call.
    }
}
