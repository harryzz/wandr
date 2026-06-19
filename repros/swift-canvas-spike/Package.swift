// swift-tools-version:6.0
import PackageDescription

// Task 114 P1 — Swift custom-WIT spike. Build for wasm with:
//   swift build --swift-sdk swift-6.3.2-RELEASE_wasm ... (see build.sh)
// OpenSwiftUIDemo (phase 4b) reuses CSwiftSpike + OpenCoreGraphics and adds OpenSwiftUI,
// rendering a real SwiftUI DisplayList through a WandrDrawSink → CGContext → wasi:canvas.
let package = Package(
    name: "swift-canvas-spike",
    dependencies: [
        .package(path: "/tmp/OpenSwiftUI"),
    ],
    targets: [
        // wit-bindgen-c generated surface (imports + the export trampolines).
        .target(name: "CSwiftSpike"),
        // P2.3 — VENDORED OpenCoreGraphics (real upstream geometry/CGPath; see
        // Sources/OpenCoreGraphics/VENDORED.txt), with its empty CGContext.swift
        // implemented over wasi:canvas (hence the CSwiftSpike dep) + an added CGColor.
        // Module renamed WandrCG (dir kept) to avoid colliding with OpenSwiftUI's own
        // `OpenCoreGraphics` stub package once OpenSwiftUI is in the graph.
        .target(name: "WandrCG", dependencies: ["CSwiftSpike"], path: "Sources/OpenCoreGraphics"),
        // The original spike: hand-built DisplayList drawn via CGContext.
        .executableTarget(
            name: "SwiftSpike",
            dependencies: ["CSwiftSpike", "WandrCG"]
        ),
        // Phase 4b: OpenSwiftUI renders its DisplayList through a WandrDrawSink → CGContext.
        // The wasm-only link flags live here scoped to .wasi so they DON'T leak to the host
        // builds of OpenSwiftUI's Swift-macro plugin tools (which would fail on -lwasi-emulated-*).
        .executableTarget(
            name: "OpenSwiftUIDemo",
            dependencies: [
                "CSwiftSpike",
                "WandrCG",
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
            ],
            linkerSettings: [
                .linkedLibrary("wasi-emulated-signal", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-mman", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-process-clocks", .when(platforms: [.wasi])),
                .unsafeFlags([
                    "-Xclang-linker", "-mexec-model=reactor",
                    "-Xlinker", "generated/swift_spike_component_type.o",
                ], .when(platforms: [.wasi])),
            ]
        ),
    ]
)
