// swift-tools-version:6.0
import PackageDescription

// Task 114 P1 — Swift custom-WIT spike. Build for wasm with:
//   swift build --swift-sdk swift-6.3.2-RELEASE_wasm ... (see build.sh)
let package = Package(
    name: "swift-canvas-spike",
    targets: [
        // wit-bindgen-c generated surface (imports + the `run` export trampoline).
        .target(name: "CSwiftSpike"),
        // The guest: implements exports_swift_spike_run via @_cdecl, calls imports.
        .executableTarget(
            name: "SwiftSpike",
            dependencies: ["CSwiftSpike"]
        ),
    ]
)
