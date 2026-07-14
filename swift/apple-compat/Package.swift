// swift-tools-version:6.0
import PackageDescription

// wandr Apple-compatibility layer. Shim modules that let UNMODIFIED Apple-framework code
// (`import SwiftUI` / `import Combine` / `import AudioToolbox`) compile and run on wandr by
// forwarding to the Open* reimplementations + filling the gaps they don't cover yet.
//
// The module NAMES are load-bearing — they MUST match Apple's, so app source stays untouched.
// This is SHARED: apps depend on these products, never on per-app copies. It is wandr-original
// code (not a fork), so it lives in-tree, not as a submodule.
// See swift/OpenSwiftUIProject/COMPONENTS-AND-BUILD.md §7 (target layering).
let package = Package(
    name: "apple-compat",
    products: [
        .library(name: "SwiftUI", targets: ["SwiftUI"]),
        .library(name: "Combine", targets: ["Combine"]),
        .library(name: "AudioToolbox", targets: ["AudioToolbox"]),
    ],
    dependencies: [
        // Absolute path to match the consumer packages exactly, so SwiftPM dedupes OpenSwiftUI
        // to a single package node in the graph.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/OpenSwiftUI"),
    ],
    targets: [
        // `import SwiftUI` → OpenSwiftUI + API OpenSwiftUI still lacks (List/Section/Link/
        // AppStorage/PreviewLayout/UIColor/UIUserInterfaceIdiom…).
        .target(name: "SwiftUI", dependencies: [.product(name: "OpenSwiftUI", package: "OpenSwiftUI")]),
        // `import Combine` → OpenCombine (carried transitively by OpenSwiftUI) + a
        // NotificationCenter bridge.
        .target(name: "Combine", dependencies: [.product(name: "OpenSwiftUI", package: "OpenSwiftUI")]),
        // `import AudioToolbox` → the AudioServices surface eleev uses (silent v1; → wasi:audio later).
        .target(name: "AudioToolbox"),
    ]
)
