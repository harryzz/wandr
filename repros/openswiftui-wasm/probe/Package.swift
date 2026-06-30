// swift-tools-version: 6.2
//
// Phase-3 probe: prove OpenSwiftUI's ViewGraph + Compute pipeline executes on
// wasm32-wasip1 and emits a DisplayList. Builds a trivial @main App; on WASI
// `runApp` defaults to the stdout renderer (one-shot renderOnce()), so running
// the wasm under wasmtime prints the resolved display list — no host/WIT needed.
import PackageDescription

let package = Package(
    name: "Probe",
    products: [
        .executable(name: "probe", targets: ["Probe"]),
    ],
    dependencies: [
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/OpenSwiftUI"),
    ],
    targets: [
        // No-op stub for the Apple-only Graph.mm symbol print_cycle (linked, not imported).
        .target(name: "ProbeStubs"),
        .executableTarget(
            name: "Probe",
            dependencies: [
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
                "ProbeStubs",
            ],
            // eleev/swiftui-2048 is Swift-5-era; build in Swift 5 mode so Swift 6 strict
            // concurrency doesn't reject it as-is (build setting, not edits to eleev).
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
