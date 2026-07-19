// [wandr] Small iOS/SwiftUI conveniences that real-world SwiftUI apps use but OpenSwiftUI-on-wandr
// lacks — surfaced by the andreiui/swift-calculator portability test. Purely additive shims, so
// stock SwiftUI compiles UNMODIFIED. (None of these touch the ported app's own source.)
import OpenSwiftUI

// MARK: - iOS system colors
//
// The calculator uses `Color(.systemBlue)` / `Color(.systemGray2)` / `Color(.systemGray4)`. Real
// SwiftUI's `Color(_:)` takes a `UIColor`; the standard iOS palette lives on the `UIColor` shim
// (UIKitShim.swift). Here is just the `Color(_: UIColor)` bridge. (`Color.Resolved`'s RGB init is
// package-internal, so the palette is built via the public `Color(red:green:blue:opacity:)`.)
public extension Color {
    init(_ uiColor: UIColor) { self = uiColor.color ?? .clear }
}

// MARK: - Missing view modifiers
public extension View {
    // `PlainButtonStyle` exists in OpenSwiftUI but as a `PrimitiveButtonStyle`, and only
    // `.buttonStyle(_: some ButtonStyle)` is wired — add the PrimitiveButtonStyle overload as a
    // no-op (plain == default button appearance).
    func buttonStyle<S: PrimitiveButtonStyle>(_ style: S) -> some View { self }

    // `.lineLimit(_:)` isn't implemented; no-op (a calculator display simply doesn't wrap).
    func lineLimit(_ limit: Int?) -> some View { self }
}
