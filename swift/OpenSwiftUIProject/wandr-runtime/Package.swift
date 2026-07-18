// swift-tools-version:6.0
import PackageDescription

// NEXT-SESSION-TASKS.md #3 — the shared OpenSwiftUI-on-wandr runtime. Per
// Sources/T2iles/RULES.md, an app carries ONLY Audio/Store/startup; everything else
// (the WandrDrawSink conformer, the wasi:canvas embedding handshake, frame pacing,
// pointer forwarding, the generic WandrAudioPlayer wasi:audio/pcm engine) is generic
// runtime glue that lives HERE, once, instead of being duplicated per app.
let package = Package(
    name: "wandr-runtime",
    products: [
        .library(name: "WandrRuntime", targets: ["WandrRuntime"]),
    ],
    dependencies: [
        .package(path: "../OpenSwiftUI"),
        .package(path: "../CWASICanvas"),
        .package(path: "../CWasiAudio"),
        .package(path: "../OpenCoreGraphics"),
    ],
    targets: [
        .target(
            name: "WandrRuntime",
            dependencies: [
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
                .product(name: "CWASICanvas", package: "CWASICanvas"),
                .product(name: "CWasiAudio", package: "CWasiAudio"),
                .product(name: "OpenCoreGraphicsWASICanvas", package: "OpenCoreGraphics"),
                "CWandrBoot",
            ]
        ),
        // Tiny C shim that calls the app's @main-generated `__main_argc_argv` reactor entry with the
        // correct C calling convention (a Swift-side decl mis-lowers it — see the header).
        .target(name: "CWandrBoot"),
    ]
)
