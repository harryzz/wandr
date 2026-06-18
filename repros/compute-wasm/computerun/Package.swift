// swift-tools-version: 6.0
import PackageDescription
let package = Package(
  name: "computerun",
  dependencies: [.package(path: "/tmp/Compute")],
  targets: [.executableTarget(
    name: "computerun",
    dependencies: [.product(name: "Compute", package: "Compute")],
    swiftSettings: [.enableExperimentalFeature("Extern")])]  // NOT .interoperabilityMode(.Cxx)
)
