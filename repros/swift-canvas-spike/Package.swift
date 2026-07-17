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
        // The wasi:audio/pcm bindings — same standalone-leaf treatment, for wandr-runtime's
        // WandrAudioPlayer. CSwiftSpike no longer generates its own copy of these either.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/CWasiAudio"),
        // CGContext/CGImageHandle/CGColor/CGGradient — real, wasi:canvas-backed, now live in
        // OpenCoreGraphics itself as the OpenCoreGraphicsWASICanvas target, instead of a per-app
        // vendored copy. Consumed directly (not via OpenCoreGraphicsShims): these render-glue
        // files are wasm32-wasip1-only, so Shims' cross-platform backend selection is never
        // exercised for them — only OpenSwiftUI itself (genuinely multi-platform) needs Shims.
        // NEXT-SESSION-TASKS.md #2.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/OpenCoreGraphics"),
        // The shared OpenSwiftUI-on-wandr reactor loop (WandrDrawSink conformer, wasi:canvas
        // embedding handshake, frame pacing, pointer forwarding) — per Sources/T2iles/RULES.md,
        // an app carries only Audio/Store/startup; everything else lives here, once.
        // NEXT-SESSION-TASKS.md #3.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/wandr-runtime"),
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
                // WandrHeadless.swift (the -DWANDR_HEADLESS deterministic test driver, which
                // bypasses wasi:canvas/WandrRuntime entirely) needs CGSize directly.
                .product(name: "OpenCoreGraphicsWASICanvas", package: "OpenCoreGraphics"),
                .product(name: "WandrRuntime", package: "wandr-runtime"),
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
            ],
            // T2ilesApp = @main/UserDefaults entry (WandrReactor replaces it). Audio/* is
            // eleev's own UNMODIFIED original now — apple-compat's Bundle/AudioToolbox shims
            // make it work as-is (no more per-app WandrAudio.swift replacement target needed).
            // Utils/Plist/PlistConfiguration.swift stays excluded: apple-compat's SwiftUI
            // already declares a same-named replacement (see that file's own doc comment) —
            // un-excluding this one would locally shadow it right back to the broken,
            // Bundle.main-trapping original (same-module declarations always win over imports).
            exclude: ["T2ilesApp.swift", "Utils/Plist/PlistConfiguration.swift"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedLibrary("wasi-emulated-signal", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-mman", .when(platforms: [.wasi])),
                .linkedLibrary("wasi-emulated-process-clocks", .when(platforms: [.wasi])),
                .unsafeFlags([
                    "-Xclang-linker", "-mexec-model=reactor",
                    // Three component-type object files, one per WIT world actually generated:
                    // this app's own (exports + metrics, no wasi:canvas/audio anymore),
                    // CWASICanvas's (wasi:canvas draw/types/embedding/layout), and CWasiAudio's
                    // (wasi:audio/pcm, via wandr-runtime's WandrAudioPlayer). All pure metadata
                    // (no code symbols), so wasm-tools composes them into one component's
                    // declared import/export surface — see NEXT-SESSION-TASKS.md #1.
                    "-Xlinker", "generated/swift_spike_component_type.o",
                    "-Xlinker", "/home/harry/wandr/swift/OpenSwiftUIProject/CWASICanvas/generated/cwasi_canvas_component_type.o",
                    "-Xlinker", "/home/harry/wandr/swift/OpenSwiftUIProject/CWasiAudio/generated/cwasi_audio_component_type.o",
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
                .product(name: "OpenCoreGraphicsWASICanvas", package: "OpenCoreGraphics"),
            ]
        ),
        // Phase 4b: OpenSwiftUI renders its DisplayList through a WandrDrawSink → CGContext.
        // The wasm-only link flags live here scoped to .wasi so they DON'T leak to the host
        // builds of OpenSwiftUI's Swift-macro plugin tools (which would fail on -lwasi-emulated-*).
        .executableTarget(
            name: "OpenSwiftUIDemo",
            dependencies: [
                "CSwiftSpike",
                .product(name: "WandrRuntime", package: "wandr-runtime"),
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
                    "-Xlinker", "/home/harry/wandr/swift/OpenSwiftUIProject/CWasiAudio/generated/cwasi_audio_component_type.o",
                ], .when(platforms: [.wasi])),
            ]
        ),
    ]
)
