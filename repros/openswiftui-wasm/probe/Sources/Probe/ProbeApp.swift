import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI
#if os(WASI)
import WASILibc
#endif

// PHASE 4 (in progress): wire OpenSwiftUI's real DisplayList into a drawing sink.
// Instead of the stdout renderer, drive the new `.wandr` renderer with a WandrDrawSink.
// On device the sink draws into a wasi:canvas CGContext (skia/EGL); here a PrintSink
// proves the SAME resolved fills flow through the new seam on desktop wasmtime.
struct ContentView: View {
    var body: some View {
        VStack {
            Color.red
            Color.blue
        }
    }
}

struct ProbeApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}

// Desktop validation sink: print what a CGContext sink would draw.
final class PrintSink: WandrDrawSink {
    func beginFrame(width: Double, height: Double, version: UInt32) {
        print("wandr beginFrame \(Int(width))x\(Int(height)) v\(version)")
    }
    func fillRect(
        x: Double, y: Double, width: Double, height: Double,
        red: Float, green: Float, blue: Float, opacity: Float
    ) {
        print(String(format: "wandr fillRect x:%.1f y:%.1f w:%.1f h:%.1f rgba(%.3f, %.3f, %.3f, %.3f)",
                     x, y, width, height, red, green, blue, opacity))
    }
    func endFrame() { print("wandr endFrame") }
}

@main
struct Main {
    static func main() {
        let sink = PrintSink()
        renderWandrAppOnce(ProbeApp(), options: .init(sink: sink))
        #if os(WASI)
        // Exit before any retained-graph teardown reaches the Subgraph.forEach swiftcall
        // wall (the host is kept alive by the lib; this also flushes stdout).
        exit(0)
        #endif
    }
}
