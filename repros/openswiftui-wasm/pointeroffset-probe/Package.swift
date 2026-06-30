// swift-tools-version: 6.2
//
// G2 diagnostic probe: characterize PointerOffset.offset { .of(&$0.field) } for
// trivial vs non-trivial (struct-holding-closure / bare-function / class-ref) field
// shapes, native (x86_64-linux) vs wasm32-wasip1.
//
// Self-contained: it vendors a VERBATIM copy of the live source
//   swift/OpenSwiftUIProject/Compute/Sources/Compute/Attribute/PointerOffset.swift
// (PointerOffset is pure-stdlib — no ComputeCxx dependency), so the probe builds with
// zero external deps. The materialization behavior under test is a property of the
// Swift compiler's codegen for that exact source, identical to its behavior inside the
// Compute module. (If a later step shows a module-boundary effect, escalate to linking
// the real Compute target.)
import PackageDescription

let package = Package(
    name: "PointerOffsetProbe",
    products: [
        .executable(name: "pointeroffset-probe", targets: ["pointeroffset-probe"]),
    ],
    targets: [
        .executableTarget(
            name: "pointeroffset-probe",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
