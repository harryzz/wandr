// swift-tools-version: 6.3
import PackageDescription
let package = Package(
  name: "oag-baseline",
  dependencies: [ .package(path: "../../OpenAttributeGraph") ],
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
    .executableTarget(
      name: "oagrender",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagforeach",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagcompare",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagteardown",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagweakref",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagvalues",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagupdate",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagmemory",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagbridge",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oaggraph",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagsubgraph",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagoffset",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagattr",
      dependencies: [ .product(name: "OpenAttributeGraphShims", package: "OpenAttributeGraph"), "CStubs" ],
      swiftSettings: [ .enableExperimentalFeature("Extern") ],
      linkerSettings: [
        .unsafeFlags(["-L", "/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"], .when(platforms: [.linux])),
        .linkedLibrary("swiftDemangle", .when(platforms: [.linux])),
        .linkedLibrary("crypto", .when(platforms: [.linux])),
      ]
    ),
    .executableTarget(
      name: "oagrules",
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
