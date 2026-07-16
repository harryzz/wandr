// [wandr Phase 2] Store-class seam substitution for eleev's UserDefaults-backed board-size
// persistence (GameBoardSizeState). Confirmed by instrumented run: UserDefaults.set/integer
// round-trips correctly WITHIN a process, but never touches disk — the app's `/state` preopen
// (its declared read-write storage dir) stayed empty across writes, so nothing survives a process
// restart. Persist directly to `/state` instead, same minimal POSIX read/write style as
// WandrPlist's `/assets` read.
import Foundation
#if canImport(WASILibc)
import WASILibc
#endif

enum WandrBoardSizeStore {
    private static let path = "/state/board-size"

    static func read() -> Int? {
        guard let file = fopen(path, "rb") else { return nil }
        defer { fclose(file) }
        var buffer = [UInt8](repeating: 0, count: 16)
        let read = buffer.withUnsafeMutableBytes { fread($0.baseAddress, 1, $0.count, file) }
        guard read > 0, let s = String(bytes: buffer[0..<read], encoding: .utf8) else { return nil }
        return Int(s.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    static func write(_ value: Int) {
        guard let file = fopen(path, "wb") else { return }
        defer { fclose(file) }
        let s = "\(value)"
        _ = s.withCString { fwrite($0, 1, strlen($0), file) }
    }
}
