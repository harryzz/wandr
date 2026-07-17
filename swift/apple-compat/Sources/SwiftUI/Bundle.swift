//
//  Bundle.swift
//  SwiftUI (apple-compat)
//
//  A working Bundle. Real Foundation's `Bundle.main` traps on wasm (swift-corelibs-Foundation
//  stubs the main-bundle lookup -> `unreachable` inside its `swift_once` initializer, before any
//  method can even run). This file's `class Bundle` SHADOWS the real one for any app code that
//  `import SwiftUI` (same trick as UserDefaults.swift/PlistConfiguration.swift in this
//  directory) — so `Bundle.main.url(forResource:withExtension:)` etc. compile and run UNMODIFIED.
//
//  Deliberately minimal: only what eleev's own call sites actually use (`.main` +
//  `url(forResource:withExtension:)`). `/assets` preopen convention, same as PlistConfiguration
//  and WandrAudioPlayer — existence-checked via POSIX `access`, not a real file read, since the
//  caller (AudioServicesCreateSystemSoundID) only needs a URL identifying which asset, not bytes.
import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

public final class Bundle: @unchecked Sendable {
    public static let main = Bundle()

    private init() {}

    public func url(forResource name: String?, withExtension ext: String?) -> URL? {
        guard let name, let ext else { return nil }
        let path = "/assets/\(name).\(ext)"
        guard access(path, F_OK) == 0 else { return nil }
        return URL(fileURLWithPath: path)
    }
}
