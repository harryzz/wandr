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
        // Apple-compatibility shims (SwiftUI/Combine/AudioToolbox) — shared, out of the app.
        .package(path: "/home/harry/wandr/swift/apple-compat"),
        // The wasi:canvas draw/types/embedding/layout bindings — a standalone leaf, generated
        // once and shared (NEXT-SESSION-TASKS.md #1). CSwiftSpike no longer generates its own
        // copy of these (same C symbol names would collide if both were linked).
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/CWASICanvas"),
        // CGContext/CGImage/CGColor/CGGradient — real, wasi:canvas-backed, now live in
        // OpenCoreGraphics itself (OpenCoreGraphicsShims re-exports the working implementation
        // on os(WASI)) instead of a per-app vendored copy. NEXT-SESSION-TASKS.md #2.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/OpenCoreGraphics"),
    ],
    targets: [
        // The Apple-compat shim modules moved to swift/apple-compat; consumed here as products
        // (module names still SwiftUI/Combine/AudioToolbox so eleev's imports resolve unmodified).
        .executableTarget(name: "ShimTest",
                          dependencies: [
                              .product(name: "SwiftUI", package: "apple-compat"),
                              .product(name: "Combine", package: "apple-compat"),
                              .product(name: "AudioToolbox", package: "apple-compat"),
                          ],
                          swiftSettings: [.swiftLanguageMode(.v5)]),
        // [Phase 2] The REAL eleev/swiftui-2048 app, dropped in unmodified behind the shims.
        // T2ilesApp.swift (@main, UserDefaults) is excluded — WandrReactor.swift is our entry.
        .executableTarget(
            name: "T2iles",
            dependencies: [
                .product(name: "SwiftUI", package: "apple-compat"),
                .product(name: "Combine", package: "apple-compat"),
                .product(name: "AudioToolbox", package: "apple-compat"),
                "CSwiftSpike",
                .product(name: "CWASICanvas", package: "CWASICanvas"),
                .product(name: "OpenCoreGraphicsShims", package: "OpenCoreGraphics"),
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
            ],
            // T2ilesApp = @main/UserDefaults entry (WandrReactor replaces it);
            // Audio/* = AudioToolbox-based Audio seam (WandrAudio replaces it).
            exclude: ["T2ilesApp.swift", "Audio/Audio.swift", "Audio/AudioSource.swift",
                      "Utils/Plist/PlistConfiguration.swift"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedLibrary("wasi-emulated-signal", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-mman", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-process-clocks", .when(platforms: [.wasi])),
                .unsafeFlags([
                    "-Xclang-linker", "-mexec-model=reactor",
                    // Two component-type object files, one per WIT world actually generated:
                    // this app's own (exports + audio + metrics, no wasi:canvas anymore) and
                    // CWASICanvas's (wasi:canvas draw/types/embedding/layout). Both are pure
                    // metadata (no code symbols), so wasm-tools composes them into one
                    // component's declared import/export surface — see NEXT-SESSION-TASKS.md #1.
                    "-Xlinker", "generated/swift_spike_component_type.o",
                    "-Xlinker", "/home/harry/wandr/swift/OpenSwiftUIProject/CWASICanvas/generated/cwasi_canvas_component_type.o",
                ], .when(platforms: [.wasi])),
            ]
        ),
        // wit-bindgen-c generated surface for THIS app's own exports + audio/metrics imports.
        // wasi:canvas bindings come from the CWASICanvas package instead (NEXT-SESSION-TASKS.md #1).
        .target(name: "CSwiftSpike"),
        // (ComputeStubs removed — Graph::print_cycle now lives in Compute's Graph.cpp under #if !TARGET_OS_MAC.)
        // The original spike: hand-built DisplayList drawn via CGContext.
        .executableTarget(
            name: "SwiftSpike",
            dependencies: [
                "CSwiftSpike",
                .product(name: "CWASICanvas", package: "CWASICanvas"),
                .product(name: "OpenCoreGraphicsShims", package: "OpenCoreGraphics"),
            ]
        ),
        // Phase 4b: OpenSwiftUI renders its DisplayList through a WandrDrawSink → CGContext.
        // The wasm-only link flags live here scoped to .wasi so they DON'T leak to the host
        // builds of OpenSwiftUI's Swift-macro plugin tools (which would fail on -lwasi-emulated-*).
        .executableTarget(
            name: "OpenSwiftUIDemo",
            dependencies: [
                "CSwiftSpike",
                .product(name: "CWASICanvas", package: "CWASICanvas"),
                .product(name: "OpenCoreGraphicsShims", package: "OpenCoreGraphics"),
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
                    "-Xlinker", "/home/harry/wandr/swift/OpenSwiftUIProject/CWASICanvas/generated/cwasi_canvas_component_type.o",
                ], .when(platforms: [.wasi])),
            ]
        ),
    ]
)
