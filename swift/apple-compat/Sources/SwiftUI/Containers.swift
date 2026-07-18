// [wandr Phase 2] SwiftUI containers/styles OpenSwiftUI lacks (upstream open issues: List, Form,
// Navigation). Minimal wrappers so eleev's Settings/Modal screens compile + render approximately
// (flat VStack layout). buttonStyle/listStyle are cosmetic → no-ops.
import OpenSwiftUI

public extension View {
    // buttonStyle + allowsHitTesting are now implemented for real in OpenSwiftUI (re-exported via
    // `@_exported import OpenSwiftUI`), so the shim no longer stubs them.
    func listStyle<S>(_ style: S) -> some View { wandrShimWarnOnce("shim: .listStyle no-op (cosmetic)"); return self }
}
public struct InsetGroupedListStyle { public init() {} }
public struct GroupedListStyle { public init() {} }
public struct PlainListStyle { public init() {} }
public struct DefaultListStyle { public init() {} }

// List is still a shim (upstream OpenSwiftUI has no List — sections/styles/selection are a big
// surface), but it is now SCROLLABLE: its content is wrapped in OpenSwiftUI's real ScrollView
// (OpenSwiftUICore/View/Scroll/ScrollView.swift), so tall content (e.g. the 2048 Settings list,
// whose audio section sits below the board preview) can be reached.
// TODO: fold List itself into OpenSwiftUI the same way ScrollView was — a real view in the library,
// backed by this ScrollView — once its section/style/selection surface is covered.
public struct List<Content: View>: View {
    private let content: Content
    public init(@ViewBuilder content: () -> Content) { self.content = content() }
    public var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 0) { content }
        }
    }
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
