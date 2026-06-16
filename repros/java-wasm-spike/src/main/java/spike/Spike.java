package spike;

// Pure-Java, zero android.*/native/Binder/Looper — the task-112 spike shape.
public final class Spike {
    // Non-trivial pure logic: GSM 7-bit septet packing length (an SMS-PDU primitive).
    public static int packedLen(int septets) {
        return (septets * 7 + 7) / 8;
    }
    public static void main(String[] args) {
        StringBuilder sb = new StringBuilder();
        for (int n = 1; n <= 5; n++) sb.append(packedLen(n)).append(' ');
        System.out.println("spike packedLen: " + sb.toString().trim());
    }
}
