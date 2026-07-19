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
        // For AudioToolbox's AudioServices shim to actually play sound (via WandrAudioPlayer)
        // instead of being silent.
        .package(path: "/home/harry/wandr/swift/OpenSwiftUIProject/wandr-runtime"),
    ],
    // NOTE: deliberately NO module named `CoreGraphics` here. Providing one flips
    // `#if canImport(CoreGraphics)` to true across the WHOLE graph, activating OpenSwiftUICore's
    // Apple-only CG code paths (full CGContext/CGColorSpace/CGDataConsumer) that the WASI canvas
    // backend doesn't implement → build breaks. Stock code that `import CoreGraphics` for CGRect/
    // CGPoint/CGFloat should import Foundation instead (those types + trig live there off-Apple).
    targets: [
        // `import SwiftUI` → OpenSwiftUI + API OpenSwiftUI still lacks (List/Section/Link/
        // AppStorage/PreviewLayout/UIColor/UIUserInterfaceIdiom…).
        .target(name: "SwiftUI", dependencies: [.product(name: "OpenSwiftUI", package: "OpenSwiftUI"), "Combine"]),
        // `import Combine` → OpenCombine (carried transitively by OpenSwiftUI) + a
        // NotificationCenter bridge.
        .target(name: "Combine", dependencies: [.product(name: "OpenSwiftUI", package: "OpenSwiftUI")]),
        // `import AudioToolbox` → the AudioServices surface eleev uses, routed to WandrAudioPlayer.
        .target(name: "AudioToolbox", dependencies: [.product(name: "WandrRuntime", package: "wandr-runtime")]),
    ]
)
