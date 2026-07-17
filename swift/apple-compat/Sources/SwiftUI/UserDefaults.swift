//
//  UserDefaults.swift
//  SwiftUI (apple-compat)
//
//  A working UserDefaults. Real Foundation's UserDefaults doesn't persist to disk on wasm
//  (confirmed by instrumented run: set/integer round-trip correctly WITHIN a process, but never
//  touch disk — there's no cfprefsd/CFPreferences backing on this target). This file's `class
//  UserDefaults` SHADOWS the real one for any app code that `import SwiftUI` (apple-compat's
//  Shim.swift already `@_exported import Foundation`, which is how eleev's own unmodified code
//  sees `UserDefaults` at all) — so `UserDefaults.standard.set(...)`/`.integer(forKey:)` etc.
//  compile and run UNMODIFIED, now backed by a real per-suite plist file under the app's /state
//  preopen. One plist file per SUITE (matching how real UserDefaults actually persists — one
//  domain = one plist), not one file per key: /state/<suite>.plist, holding every key in that
//  suite as a dictionary. `.standard` uses the suite name "standard".
import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

public final class UserDefaults: @unchecked Sendable {
    public static let standard = UserDefaults(suiteName: nil)!

    private let suiteName: String
    private var cache: [String: Any]?

    public init?(suiteName: String?) {
        self.suiteName = suiteName ?? "standard"
    }

    private var path: String { "/state/\(suiteName).plist" }

    private func load() -> [String: Any] {
        if let cache { return cache }
        guard let file = fopen(path, "rb") else {
            cache = [:]
            return [:]
        }
        defer { fclose(file) }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let read = buffer.withUnsafeMutableBytes { fread($0.baseAddress, 1, $0.count, file) }
            if read <= 0 { break }
            data.append(contentsOf: buffer[0..<read])
        }
        let dict = (try? PropertyListSerialization.propertyList(from: data, options: [], format: nil)) as? [String: Any]
        let result = dict ?? [:]
        cache = result
        return result
    }

    private func save(_ dict: [String: Any]) {
        cache = dict
        guard let data = try? PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0) else {
            return
        }
        guard let file = fopen(path, "wb") else { return }
        defer { fclose(file) }
        data.withUnsafeBytes { buf in
            _ = fwrite(buf.baseAddress, 1, buf.count, file)
        }
    }

    public func set(_ value: Any?, forKey key: String) {
        var dict = load()
        dict[key] = value
        save(dict)
    }

    public func removeObject(forKey key: String) {
        var dict = load()
        dict.removeValue(forKey: key)
        save(dict)
    }

    public func object(forKey key: String) -> Any? { load()[key] }

    public func integer(forKey key: String) -> Int {
        (load()[key] as? Int) ?? 0
    }

    public func bool(forKey key: String) -> Bool {
        (load()[key] as? Bool) ?? false
    }

    public func double(forKey key: String) -> Double {
        (load()[key] as? Double) ?? 0
    }

    public func float(forKey key: String) -> Float {
        (load()[key] as? Float) ?? 0
    }

    public func string(forKey key: String) -> String? {
        load()[key] as? String
    }
}
