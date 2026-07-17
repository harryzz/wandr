// swift-tools-version:6.0
import PackageDescription

// CWASICanvas — a standalone leaf module holding ONLY the wasi:canvas *drawing* bindings
// (types/draw/embedding/layout), generated via `wit-bindgen c wit --out-dir generated` against
// wit/cwasi-canvas.wit. Depends on nothing, so packages ABOVE any one app — e.g.
// OpenCoreGraphics's CGContext (see NEXT-SESSION-TASKS.md #2) — can depend on it without a
// package cycle through the app that used to be the only place these bindings lived
// (repros/swift-canvas-spike's CSwiftSpike). See NEXT-SESSION-TASKS.md #1.
let package = Package(
    name: "CWASICanvas",
    products: [
        .library(name: "CWASICanvas", targets: ["CWASICanvas"]),
    ],
    targets: [
        .target(name: "CWASICanvas"),
    ]
)
