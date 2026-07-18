// swift-tools-version:6.0
import PackageDescription

// CWandrExports — a standalone leaf holding ONLY the reactor export surface (frame/pointer input
// handlers + shell-events/frame-pacing/startup) and the metrics import, generated via
// `wit-bindgen c wit --out-dir generated` against wit/cwandr-exports.wit. Depends on nothing, so the
// shared WandrReactorExports library can define the @_cdecl export implementations against these
// generated types without a package cycle through the app. Mirrors CWASICanvas/CWasiAudio.
let package = Package(
    name: "CWandrExports",
    products: [
        .library(name: "CWandrExports", targets: ["CWandrExports"]),
    ],
    targets: [
        .target(name: "CWandrExports"),
    ]
)
