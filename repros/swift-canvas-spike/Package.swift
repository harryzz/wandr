// swift-tools-version:6.0
import PackageDescription

// Task 114 P1 — Swift custom-WIT spike. Build for wasm with:
//   swift build --swift-sdk swift-6.3.2-RELEASE_wasm ... (see build.sh)
// OpenSwiftUIDemo (phase 4b) reuses CSwiftSpike + OpenCoreGraphics and adds OpenSwiftUI,
// rendering a real SwiftUI DisplayList through a WandrDrawSink → CGContext → wasi:canvas.
let package = Package(
    name: "swift-canvas-spike",
    dependencies: [
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/OpenSwiftUI"),
    ],
    targets: [
        // wit-bindgen-c generated surface (imports + the export trampolines).
        .target(name: "CSwiftSpike"),
        // No-op stub for the Apple-only Graph.mm symbol print_cycle (linked from UpdateStack.cpp, not called).
        .target(name: "ComputeStubs"),
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
                "ComputeStubs",
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
            ],
            // eleev/swiftui-2048 is Swift-5-era code; build it in Swift 5 language mode so
            // Swift 6 strict-concurrency (Sendable/data-race) checks don't reject it as-is.
            // (The Swift-version decision is a build setting here, not edits to eleev's sources.)
            swiftSettings: [.swiftLanguageMode(.v5)],
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
