// [wandr Phase 2] SwiftUI-surface pieces eleev uses that OpenSwiftUI lacks. Dev-only preview
// scaffolding is stubbed (never rendered on wandr); Apple-fast-path modifiers become no-ops.
import OpenSwiftUI
import Foundation

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
    func drawingGroup(opaque: Bool = false) -> some View { self }
    // allowsHitTesting — TODO real gating; identity for now.
    func allowsHitTesting(_ enabled: Bool) -> some View { self }
}
