// swift-tools-version: 6.3
import PackageDescription
let package = Package(
  name: "oag-baseline",
  dependencies: [ .package(path: "../OpenAttributeGraph") ],
  targets: [
    .target(name: "CStubs"),
    .executableTarget(
      name: "oagdataflow",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagchurn",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
  ]
)
