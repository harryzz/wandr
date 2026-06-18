// swift-tools-version:6.0
import PackageDescription

// Task 114 P1 — Swift custom-WIT spike. Build for wasm with:
//   swift build --swift-sdk swift-6.3.2-RELEASE_wasm ... (see build.sh)
let package = Package(
    name: "swift-canvas-spike",
    targets: [
        // wit-bindgen-c generated surface (imports + the export trampolines).
        .target(name: "CSwiftSpike"),
        // P2.3 — OpenCoreGraphics's CGContext implemented over wasi:canvas.
        .target(name: "CoreGraphicsWasi", dependencies: ["CSwiftSpike"]),
        // The guest: implements the reactor exports via @_cdecl, draws via CGContext.
        .executableTarget(
            name: "SwiftSpike",
            dependencies: ["CSwiftSpike", "CoreGraphicsWasi"]
        ),
    ]
)
