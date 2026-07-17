//
//  PlistConfiguration.swift
//  SwiftUI (apple-compat)
//
//  A working PlistConfiguration. `Bundle.main.path(forResource:ofType:)` traps on wasm
//  (swift-corelibs-Foundation stubs the main-bundle lookup -> `unreachable` inside its
//  `swift_once` initializer, before a failable init can even return nil) — this is the generic
//  workaround: read straight from the app's `/state`-sibling `/assets` preopen via POSIX, no
//  Bundle involved. `PlistConfiguration` isn't an Apple SDK type (it's an app-authored utility
//  name, e.g. eleev's own excluded `Utils/Plist/PlistConfiguration.swift`), so there's no real
//  type to shadow here — this is just declared directly in this target so any app code that
//  `import SwiftUI` sees it by bare name, same as `UserDefaults.swift`'s shim in this directory.
//
//  Also deliberately avoids `FileManager.default.contents(atPath:)` even as a Bundle-free
//  read path: confirmed (2026-07-17) it silently returns EMPTY Data (not nil, not a throw) for
//  real, non-empty files on this target, when `fstat`'s st_size comes back 0 for a wasi:filesystem
//  preopen. Root cause identified in swift-foundation's Data+Reading.swift — the `fileSize == 0`
//  fallback-chunked-read exists only for `#if os(Linux) || os(Android)`, `os(WASI)` isn't
//  included. Filed upstream: https://github.com/swiftlang/swift-foundation/issues/2120 — once
//  fixed, `readAsset` below could plausibly be replaced by plain `FileManager.default.contents`,
//  but there's no real benefit to doing so (this POSIX version already works); don't bother
//  unless the fix comes with some other reason to prefer FileManager specifically.
import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

public struct PlistConfiguration {
    public let name: String
    public let xml: Data

    public init?(name: String) {
        guard let data = Self.readAsset(name: name, ext: "plist") else { return nil }
        self.name = name
        self.xml = data
    }

    public func getItem(named name: String) -> [String: [String: String]]? {
        return try? PropertyListSerialization.propertyList(
            from: xml, options: .mutableContainersAndLeaves, format: nil
        ) as? [String: [String: String]]
    }

    /// Read `/assets/<name>.<ext>` in full via POSIX (no `Bundle`). `nil` if the file doesn't
    /// exist or is empty. Apps declare their asset mounts in package.toml; the host preopens
    /// them at /assets.
    private static func readAsset(name: String, ext: String) -> Data? {
        guard let file = fopen("/assets/\(name).\(ext)", "rb") else { return nil }
        defer { fclose(file) }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let read = buffer.withUnsafeMutableBytes { fread($0.baseAddress, 1, $0.count, file) }
            if read <= 0 { break }
            data.append(contentsOf: buffer[0..<read])
        }
        guard !data.isEmpty else { return nil }
        return data
    }
}
