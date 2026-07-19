// [wandr] LazyVGrid + GridItem — OpenSwiftUI has no grid container at all. This is a REAL functional
// grid (not a no-op): a custom `Layout` flows the children into rows, computing the column count from
// the available width for `.adaptive`, or from the column list for `.fixed`/`.flexible`. Enough to run
// a stock `AspectVGrid`-style memory-game board unmodified. (A fuller grid — pinned section
// headers/footers, per-column min/max clamping, spacing precedence — is a future OpenSwiftUI view,
// same as ScrollView/List.)
import OpenSwiftUI
import Foundation


/// A description of a single row/column in a grid.
public struct GridItem {
    public struct Size {
        enum Kind { case fixed(CGFloat); case flexible(CGFloat, CGFloat); case adaptive(CGFloat, CGFloat) }
        let kind: Kind
        public static func fixed(_ width: CGFloat) -> Size { Size(kind: .fixed(width)) }
        public static func flexible(minimum: CGFloat = 10, maximum: CGFloat = .infinity) -> Size { Size(kind: .flexible(minimum, maximum)) }
        public static func adaptive(minimum: CGFloat, maximum: CGFloat = .infinity) -> Size { Size(kind: .adaptive(minimum, maximum)) }
    }
    public var size: Size
    public var spacing: CGFloat?
    public var alignment: Alignment?
    public init(_ size: Size = .flexible(), spacing: CGFloat? = nil, alignment: Alignment? = nil) {
        self.size = size
        self.spacing = spacing
        self.alignment = alignment
    }
}

/// Section header/footer pinning options (accepted for source compatibility; pinning is a no-op here).
public struct PinnedScrollableViews: OptionSet, Sendable {
    public let rawValue: UInt32
    public init(rawValue: UInt32) { self.rawValue = rawValue }
    public static let sectionHeaders = PinnedScrollableViews(rawValue: 1 << 0)
    public static let sectionFooters = PinnedScrollableViews(rawValue: 1 << 1)
}

/// A grid that grows vertically, arranging its children into the columns you specify.
public struct LazyVGrid<Content: View>: View {
    private let columns: [GridItem]
    private let rowSpacing: CGFloat
    private let content: Content

    public init(
        columns: [GridItem],
        alignment: HorizontalAlignment = .center,
        spacing: CGFloat? = nil,
        pinnedViews: PinnedScrollableViews = [],
        @ViewBuilder content: () -> Content
    ) {
        self.columns = columns
        self.rowSpacing = spacing ?? 8
        self.content = content()
    }

    public var body: some View {
        GridFlowLayout(columns: columns, rowSpacing: rowSpacing) { content }
    }
}

/// Row-major flow layout: computes a column count from the proposed width, sizes every cell to the
/// resulting column width, and stacks rows top-to-bottom. Row height = tallest cell in that row (so
/// aspect-ratio'd children keep their shape).
private struct GridFlowLayout: Layout {
    let columns: [GridItem]
    let rowSpacing: CGFloat

    private var columnSpacing: CGFloat { columns.first?.spacing ?? rowSpacing }

    private func columnCount(forWidth width: CGFloat) -> Int {
        if columns.count == 1, case let .adaptive(minimum, _) = columns[0].size.kind {
            let s = columnSpacing
            guard minimum + s > 0 else { return 1 }
            return max(1, Int((width + s) / (minimum + s)))
        }
        return max(1, columns.count)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: LayoutSubviews, cache: inout Void) -> CGSize {
        let width = proposal.width ?? 0
        let n = columnCount(forWidth: width)
        let colWidth = cellWidth(total: width, columns: n)
        var total: CGFloat = 0
        var i = 0
        while i < subviews.count {
            let end = min(i + n, subviews.count)
            total += rowHeight(subviews, i..<end, colWidth: colWidth)
            if end < subviews.count { total += rowSpacing }
            i = end
        }
        return CGSize(width: width, height: total)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: LayoutSubviews, cache: inout Void) {
        let n = columnCount(forWidth: bounds.width)
        let s = columnSpacing
        let colWidth = cellWidth(total: bounds.width, columns: n)
        var y = bounds.minY
        var i = 0
        while i < subviews.count {
            let end = min(i + n, subviews.count)
            let h = rowHeight(subviews, i..<end, colWidth: colWidth)
            for j in i..<end {
                let col = j - i
                let x = bounds.minX + CGFloat(col) * (colWidth + s)
                subviews[j].place(
                    at: CGPoint(x: x, y: y),
                    anchor: .topLeading,
                    proposal: ProposedViewSize(width: colWidth, height: h)
                )
            }
            y += h + rowSpacing
            i = end
        }
    }

    private func cellWidth(total: CGFloat, columns n: Int) -> CGFloat {
        guard n > 0 else { return total }
        return (total - CGFloat(n - 1) * columnSpacing) / CGFloat(n)
    }

    private func rowHeight(_ subviews: LayoutSubviews, _ range: Range<Int>, colWidth: CGFloat) -> CGFloat {
        var h: CGFloat = 0
        for j in range {
            h = max(h, subviews[j].dimensions(in: ProposedViewSize(width: colWidth, height: nil)).height)
        }
        return h
    }
}

// The string-title convenience initializer real SwiftUI provides on Button (only Button(action:label:)
// exists in OpenSwiftUI).
public extension Button where Label == Text {
    init(_ title: String, action: @escaping () -> Void) {
        self.init(action: action) { Text(title) }
    }
}
