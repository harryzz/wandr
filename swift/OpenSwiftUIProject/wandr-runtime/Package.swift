// swift-tools-version:6.0
import PackageDescription

// NEXT-SESSION-TASKS.md #3 — the shared OpenSwiftUI-on-wandr runtime. Per
// Sources/T2iles/RULES.md, an app carries ONLY Audio/Store/startup; everything else
// (the WandrDrawSink conformer, the wasi:canvas embedding handshake, frame pacing,
// pointer forwarding) is generic runtime glue that lives HERE, once, instead of being
// duplicated per app.
let package = Package(
    name: "wandr-runtime",
    products: [
        .library(name: "WandrRuntime", targets: ["WandrRuntime"]),
    ],
    dependencies: [
        .package(path: "../OpenSwiftUI"),
        .package(path: "../CWASICanvas"),
        .package(path: "../OpenCoreGraphics"),
    ],
    targets: [
        .target(
            name: "WandrRuntime",
            dependencies: [
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
                .product(name: "CWASICanvas", package: "CWASICanvas"),
                .product(name: "OpenCoreGraphicsWASICanvas", package: "OpenCoreGraphics"),
            ]
        ),
    ]
)
