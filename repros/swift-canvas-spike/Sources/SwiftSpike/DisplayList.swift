// DisplayList.swift — a faithful subset mirror of OpenSwiftUI's display-list model
// (OpenSwiftUICore/Render/DisplayList/DisplayList.swift), used to prototype the wandr
// rendering backend (Option B) in isolation, before OpenSwiftUI itself builds on wasm.
//
// Real model:
//   DisplayList            = [Item]
//   Item                   = { frame: CGRect, value: .content(Content) | .effect(Effect, DisplayList) }
//   Content.Value          = .color | .image | .shape(Path, Paint, FillStyle) | .text | …
//   Effect                 = .opacity(Float) | .clip(Path, FillStyle) | .transform | .mask | …
//
// This mirror keeps the same SHAPE (recursive items + effect-wrapped sub-lists) and the
// drawable/effect kinds that map onto our CGContext, so the renderer's design transfers
// 1:1 to the real types later.
import OpenCoreGraphicsShims

struct DisplayList {
    var items: [Item]
    init(_ items: [Item] = []) { self.items = items }

    struct Item {
        var frame: CGRect
        var value: Value
        init(_ value: Value, frame: CGRect) { self.value = value; self.frame = frame }

        enum Value {
            case content(Content)
            case effect(Effect, DisplayList)
        }
    }

    enum Content {
        case color(CGColor)               // ← .color(Color.Resolved): fill the item frame
        case shape(CGPath, CGColor)       // ← .shape(Path, AnyResolvedPaint, FillStyle)
        case text(String, CGFloat, CGColor) // ← .text(StyledTextContentView, CGSize)
        case image(CGImageHandle)         // ← .image(GraphicsImage)
    }

    enum Effect {
        case opacity(UInt8)               // ← .opacity(Float)
        case clip(CGPath)                 // ← .clip(Path, FillStyle)
        case transform(CGAffineTransform) // ← .transform(Transform)
    }
}
