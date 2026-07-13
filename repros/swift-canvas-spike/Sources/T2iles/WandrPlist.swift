// [wandr Phase 2] Store-class seam substitution for eleev's Utils/Plist/PlistConfiguration.
// eleev reads `Strings.plist` via `Bundle.main.path(forResource:ofType:)`, but `Bundle.main`
// traps on wasm (swift-corelibs-Foundation stubs the main-bundle lookup → `unreachable`
// inside its `swift_once` initializer, before the failable init can return nil). This
// replacement is API-identical (`init?(name:)` + `getItem(named:)`) so eleev's views compile
// UNMODIFIED, but reads the plist from the host `/assets` preopen via POSIX — no Bundle.
// If the resource is absent, init returns nil and the callers fall back to their defaults.
import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

struct PlistConfiguration {
    let name: String
    let xml: Data

    init?(name: String) {
        self.name = name
        // Apps declare their asset mounts in package.toml; the host preopens them at /assets.
        guard let file = fopen("/assets/\(name).plist", "rb") else { return nil }
        defer { fclose(file) }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let read = buffer.withUnsafeMutableBytes { fread($0.baseAddress, 1, $0.count, file) }
            if read <= 0 { break }
            data.append(contentsOf: buffer[0..<read])
        }
        guard !data.isEmpty else { return nil }
        self.xml = data
    }

    func getItem(named name: String) -> [String: [String: String]]? {
        return try? PropertyListSerialization.propertyList(
            from: xml, options: .mutableContainersAndLeaves, format: nil
        ) as? [String: [String: String]]
    }
}
