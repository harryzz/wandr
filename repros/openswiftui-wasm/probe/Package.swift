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
        .package(path: "/tmp/OpenSwiftUI"),
    ],
    targets: [
        .executableTarget(
            name: "Probe",
            dependencies: [
                .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
            ]
        ),
    ]
)
