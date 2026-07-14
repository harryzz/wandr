// [wandr Phase 2 · Store shim] @AppStorage — missing in OpenSwiftUI. Backs eleev's settings
// persistence. v1: in-session store (TODO: persist to the /state preopen). Reactivity is coarse
// (settings are read on the next view update), which is fine for the 2048 settings flow.
import OpenSwiftUI
import Foundation

/// Minimal UserDefaults stand-in behind @AppStorage. Single-threaded wasm → @unchecked Sendable.
public final class WandrDefaults: @unchecked Sendable {
    public static let shared = WandrDefaults()
    private var store: [String: Any] = [:]
    public func value<T>(_ key: String, as _: T.Type) -> T? { store[key] as? T }
    public func set(_ value: Any, _ key: String) { store[key] = value }
}

@propertyWrapper
public struct AppStorage<Value>: DynamicProperty {
    private let key: String
    private let defaultValue: Value
    private let read: (String) -> Value?
    private let write: (String, Value) -> Void

    public var wrappedValue: Value {
        get { read(key) ?? defaultValue }
        nonmutating set { write(key, newValue) }
    }
    public var projectedValue: Binding<Value> {
        Binding(get: { self.wrappedValue }, set: { self.wrappedValue = $0 })
    }
}

public extension AppStorage where Value == Bool {
    init(wrappedValue: Value, _ key: String) {
        self.key = key; self.defaultValue = wrappedValue
        self.read = { WandrDefaults.shared.value($0, as: Bool.self) }
        self.write = { WandrDefaults.shared.set($1, $0) }
    }
}
public extension AppStorage where Value == Int {
    init(wrappedValue: Value, _ key: String) {
        self.key = key; self.defaultValue = wrappedValue
        self.read = { WandrDefaults.shared.value($0, as: Int.self) }
        self.write = { WandrDefaults.shared.set($1, $0) }
    }
}
public extension AppStorage where Value == String {
    init(wrappedValue: Value, _ key: String) {
        self.key = key; self.defaultValue = wrappedValue
        self.read = { WandrDefaults.shared.value($0, as: String.self) }
        self.write = { WandrDefaults.shared.set($1, $0) }
    }
}
