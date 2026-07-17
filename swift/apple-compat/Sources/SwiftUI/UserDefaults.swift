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
//  compile and run UNMODIFIED.
//
//  Delegates entirely to OpenSwiftUI's UserDefaultsStore — the SAME per-suite `/state/<suite>.plist`
//  engine `@AppStorage` itself uses, matching real Apple's actual relationship between the two
//  (AppStorage is a thin reactive wrapper OVER UserDefaults, not a separate store). This file adds
//  no persistence logic of its own — it's purely the name-shadow + Any-typed convenience surface.
import Foundation
import OpenSwiftUI

public final class UserDefaults: @unchecked Sendable {
    public static let standard = UserDefaults(suiteName: nil)!

    private let store: UserDefaultsStore

    public init?(suiteName: String?) {
        store = suiteName.map(UserDefaultsStore.suite) ?? .standard
    }

    public func set(_ value: Any?, forKey key: String) { store.set(value, forKey: key) }
    public func removeObject(forKey key: String) { store.removeObject(forKey: key) }
    public func object(forKey key: String) -> Any? { store.object(forKey: key) }
    public func integer(forKey key: String) -> Int { store.integer(forKey: key) }
    public func bool(forKey key: String) -> Bool { store.bool(forKey: key) }
    public func double(forKey key: String) -> Double { store.double(forKey: key) }
    public func float(forKey key: String) -> Float { store.float(forKey: key) }
    public func string(forKey key: String) -> String? { store.string(forKey: key) }
}
