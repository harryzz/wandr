// swift-tools-version:6.0
import PackageDescription

// CWasiAudio — a standalone leaf module holding ONLY the wasi:audio/pcm bindings, generated via
// `wit-bindgen c wit --out-dir generated` against wit/cwasi-audio.wit. Depends on nothing, so
// wandr-runtime's WandrAudioPlayer can depend on it without a package cycle through the app that
// used to be the only place these bindings lived (repros/swift-canvas-spike's CSwiftSpike).
// Mirrors CWASICanvas — see NEXT-SESSION-TASKS.md #1.
let package = Package(
    name: "CWasiAudio",
    products: [
        .library(name: "CWasiAudio", targets: ["CWasiAudio"]),
    ],
    targets: [
        .target(name: "CWasiAudio"),
    ]
)
