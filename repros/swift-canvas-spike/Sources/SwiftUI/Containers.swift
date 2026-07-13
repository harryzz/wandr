// [wandr Phase 2] SwiftUI containers/styles OpenSwiftUI lacks (upstream open issues: List, Form,
// Navigation). Minimal wrappers so eleev's Settings/Modal screens compile + render approximately
// (flat VStack layout). buttonStyle/listStyle are cosmetic → no-ops.
import OpenSwiftUI

public extension View {
    func buttonStyle<S>(_ style: S) -> some View { wandrShimWarnOnce("shim: .buttonStyle no-op (cosmetic)"); return self }
    func listStyle<S>(_ style: S) -> some View { wandrShimWarnOnce("shim: .listStyle no-op (cosmetic)"); return self }
}
public struct InsetGroupedListStyle { public init() {} }
public struct GroupedListStyle { public init() {} }
public struct PlainListStyle { public init() {} }
public struct DefaultListStyle { public init() {} }

public struct List<Content: View>: View {
    private let content: Content
    public init(@ViewBuilder content: () -> Content) { self.content = content() }
    public var body: some View { VStack(alignment: .leading, spacing: 0) { content } }
}

public struct Section<Header: View, Content: View>: View {
    private let header: Header
    private let content: Content
    public init(header: Header, @ViewBuilder content: () -> Content) { self.header = header; self.content = content() }
    public var body: some View { VStack(alignment: .leading, spacing: 0) { header; content } }
}
public extension Section where Header == EmptyView {
    init(@ViewBuilder content: () -> Content) { self.init(header: EmptyView(), content: content) }
}

// SwiftUI Link — renders the label (URL-open is a TODO; used only in AboutView).
public struct Link<Label: View>: View {
    private let label: Label
    public init(destination: URL, @ViewBuilder label: () -> Label) { self.label = label() }
    public var body: some View { label }
}
public extension Link where Label == Text {
    init(_ title: String, destination: URL) { self.init(destination: destination) { Text(title) } }
}
public extension Color {
    static var clear: Color { Color(red: 0, green: 0, blue: 0, opacity: 0) }
}
