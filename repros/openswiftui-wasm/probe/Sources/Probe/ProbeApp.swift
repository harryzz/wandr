import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI
#if os(WASI)
import WASILibc
#endif

// PHASE 4 (in progress): wire OpenSwiftUI's real DisplayList into a drawing sink.
// Instead of the stdout renderer, drive the new `.wandr` renderer with a WandrDrawSink.
// On device the sink draws into a wasi:canvas CGContext (skia/EGL); here a PrintSink
// proves the SAME resolved fills flow through the new seam on desktop wasmtime.
// 2048 tile palette (the game's canonical look — one named source of truth).
func tileColors(_ v: Int) -> (bg: Color, fg: Color) {
    let dark = Color(red: 0.47, green: 0.43, blue: 0.40)
    let light = Color(red: 0.98, green: 0.96, blue: 0.93)
    switch v {
    case 2:    return (Color(red: 0.93, green: 0.89, blue: 0.85), dark)
    case 4:    return (Color(red: 0.93, green: 0.88, blue: 0.78), dark)
    case 8:    return (Color(red: 0.95, green: 0.69, blue: 0.47), light)
    case 16:   return (Color(red: 0.96, green: 0.58, blue: 0.39), light)
    case 32:   return (Color(red: 0.96, green: 0.49, blue: 0.37), light)
    case 64:   return (Color(red: 0.96, green: 0.37, blue: 0.23), light)
    case 128:  return (Color(red: 0.93, green: 0.81, blue: 0.45), light)
    case 256:  return (Color(red: 0.93, green: 0.80, blue: 0.38), light)
    case 512:  return (Color(red: 0.93, green: 0.78, blue: 0.31), light)
    case 1024: return (Color(red: 0.93, green: 0.77, blue: 0.25), light)
    case 2048: return (Color(red: 0.93, green: 0.76, blue: 0.18), light)
    default:   return (Color(red: 0.80, green: 0.76, blue: 0.71), dark) // empty
    }
}

struct TileCell: View {
    let value: Int
    var body: some View {
        let colors = tileColors(value)
        return ZStack {
            RoundedRectangle(cornerRadius: 6).fill(colors.bg)
            if value > 0 {   // conditional → DynamicContainer (the path we're fixing)
                Text("\(value)")
                    .font(.system(size: 30, weight: .bold))
                    .foregroundColor(colors.fg)
            }
        }
        .frame(width: 70, height: 70)
    }
}

// ForEach is broken off-Apple (Subgraph.index stub → OOB); the 4×4 grid is structurally
// fixed (only values change), so use explicit cells — no ForEach needed.
struct Row: View {
    let a: Int, b: Int, c: Int, d: Int
    var body: some View {
        HStack(spacing: 8) {
            TileCell(value: a); TileCell(value: b); TileCell(value: c); TileCell(value: d)
        }
    }
}

struct ContentView: View {
    let grid = [[2, 4, 8, 16], [32, 64, 128, 256], [512, 1024, 2048, 0], [0, 0, 2, 4]]
    var body: some View {
        // ForEach + conditional — the full dynamic-content path (the DynamicContainer fix).
        VStack(spacing: 8) {
            ForEach(0..<4, id: \.self) { r in
                HStack(spacing: 8) {
                    ForEach(0..<4, id: \.self) { c in
                        TileCell(value: grid[r][c])
                    }
                }
            }
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
    func drawText(
        _ text: String, x: Double, y: Double, width: Double, height: Double,
        fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float
    ) {
        print("wandr drawText \"\(text)\" x:\(Int(x)) y:\(Int(y)) w:\(Int(width)) h:\(Int(height)) size:\(Int(fontSize)) rgba(\(red), \(green), \(blue), \(opacity))")
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
