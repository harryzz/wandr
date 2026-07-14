// swift-tools-version: 5.9
import PackageDescription

// OpenSFSymbols — open, cross-platform SF-Symbol name → open-icon resolution for OpenSwiftUI.
// Layer 1 is pure Swift with no dependencies (name → IconRef only; no rendering).
let package = Package(
    name: "OpenSFSymbols",
    products: [
        .library(name: "OpenSFSymbols", targets: ["OpenSFSymbols"]),
    ],
    targets: [
        .target(name: "OpenSFSymbols"),
        .testTarget(name: "OpenSFSymbolsTests", dependencies: ["OpenSFSymbols"]),
    ]
)
