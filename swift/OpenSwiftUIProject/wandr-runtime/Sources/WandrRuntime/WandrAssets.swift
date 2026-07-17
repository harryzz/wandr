//
//  WandrAssets.swift
//  WandrRuntime
//
//  Generic `/assets` bundle-file reading. `Bundle.main` traps on wasm (swift-corelibs-Foundation
//  stubs the main-bundle lookup -> `unreachable` inside its `swift_once` initializer, before a
//  failable init can even return nil) — every OpenSwiftUI-on-wandr app needs this same POSIX
//  workaround for its own Bundle-backed resource reads, so it lives here once instead of being
//  reinvented per app. Apps whose views reference an Apple-shaped type BY BARE NAME (no import —
//  same-module visibility, e.g. eleev's own `PlistConfiguration`) keep a thin local wrapper around
//  this function; only the actual POSIX read is shared.

import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

/// Read `/assets/<name>.<ext>` in full via POSIX (no `Bundle`). `nil` if the file doesn't exist
/// or is empty.
public func wandrReadAsset(name: String, ext: String) -> Data? {
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
