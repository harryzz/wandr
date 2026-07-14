// [wandr Phase 2] SwiftUI-surface pieces eleev uses that OpenSwiftUI lacks. Dev-only preview
// scaffolding is stubbed (never rendered on wandr); Apple-fast-path modifiers become no-ops.
import OpenSwiftUI
import Foundation
#if canImport(WASILibc)
import WASILibc
#elseif canImport(Glibc)
import Glibc
#endif

// [wandr] one-shot "not implemented" reporter for the SwiftUI shim's no-op modifiers, mirroring
// OpenSwiftUICore's wandrWarnOnce — each gap prints `WANDR-UNIMPL:` once so silent no-ops surface.
nonisolated(unsafe) var _wandrShimWarned: Set<String> = []
public func wandrShimWarnOnce(_ message: String) {
    if _wandrShimWarned.insert(message).inserted {
        fputs("WANDR-UNIMPL: \(message)\n", stderr); fflush(stderr)
    }
}

// --- Xcode preview scaffolding (compile-only; not rendered) ---
public protocol PreviewProvider {
    associatedtype Previews: View
    @ViewBuilder static var previews: Previews { get }
}
public enum PreviewLayout { case device, sizeThatFits, fixed(width: CGFloat, height: CGFloat) }

public extension View {
    func previewLayout(_ value: PreviewLayout) -> some View { self }
    func previewDisplayName(_ value: String?) -> some View { self }
    // drawingGroup = Metal rasterization fast-path; a no-op is semantically fine off-Apple.
    func drawingGroup(opaque: Bool = false, colorMode: ColorRenderingMode = .nonLinear) -> some View {
        wandrShimWarnOnce("shim: .drawingGroup no-op (Metal rasterization fast-path; harmless)"); return self
    }
    // allowsHitTesting is now implemented for real in OpenSwiftUI (AllowsHitTesting.swift),
    // re-exported via `@_exported import OpenSwiftUI` — no shim stub needed.
}
