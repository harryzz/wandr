// [wandr Phase 2] SwiftUI containers/styles OpenSwiftUI lacks (upstream open issues: List, Form,
// Navigation). Minimal wrappers so eleev's Settings/Modal screens compile + render approximately
// (flat VStack layout). buttonStyle/listStyle are cosmetic → no-ops.
import OpenSwiftUI

public extension View {
    func buttonStyle<S>(_ style: S) -> some View { self }
    func listStyle<S>(_ style: S) -> some View { self }
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
