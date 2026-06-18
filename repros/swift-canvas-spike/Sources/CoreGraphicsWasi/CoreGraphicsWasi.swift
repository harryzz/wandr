// Task 114 P2.3 — OpenCoreGraphics's `CGContext` (currently an empty stub) implemented
// over wasi:canvas. This is the drawing body that slots into OpenCoreGraphics: it
// keeps the CoreGraphics API shape (saveGState/CTM/current-path/fill/stroke) and
// lowers each call to the wasi:canvas verbs (the mapping from
// docs/swift-openswiftui-wandr-feasibility.md). The geometry/path types mirror
// OpenCoreGraphics's (which already ships CGRect/CGPath/CGAffineTransform); CGColor
// is added here since OpenCoreGraphics lacks it.
import CSwiftSpike

public typealias CGFloat = Double

public struct CGPoint { public var x: CGFloat; public var y: CGFloat
    public init(x: CGFloat, y: CGFloat) { self.x = x; self.y = y } }
public struct CGSize { public var width: CGFloat; public var height: CGFloat
    public init(width: CGFloat, height: CGFloat) { self.width = width; self.height = height } }
public struct CGRect {
    public var origin: CGPoint; public var size: CGSize
    public init(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
        origin = CGPoint(x: x, y: y); size = CGSize(width: width, height: height)
    }
}

/// Non-premultiplied sRGB color. (OpenCoreGraphics has no CGColor yet.)
public struct CGColor {
    public var red: CGFloat, green: CGFloat, blue: CGFloat, alpha: CGFloat
    public init(red: CGFloat, green: CGFloat, blue: CGFloat, alpha: CGFloat) {
        self.red = red; self.green = green; self.blue = blue; self.alpha = alpha
    }
    /// 0xAARRGGBB for wasi:canvas.
    var argb: UInt32 {
        func b(_ v: CGFloat) -> UInt32 { UInt32(max(0, min(1, v)) * 255 + 0.5) }
        return (b(alpha) << 24) | (b(red) << 16) | (b(green) << 8) | b(blue)
    }
}

/// CoreGraphics drawing context, host-rendered via wasi:canvas. Construct one per
/// frame from a `borrow<canvas>` (see the guest's on_frame).
public final class CGContext {
    private let canvas: wasi_canvas_draw_borrow_canvas_t
    // Graphics state (CoreGraphics is imperative: current path + fill/stroke state).
    private var pathData = ""
    private var fillColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)
    private var strokeColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)
    private var lineWidth: CGFloat = 1

    public init(canvas: wasi_canvas_draw_borrow_canvas_t) { self.canvas = canvas }

    // ── state stack ──────────────────────────────────────────────────────────
    public func saveGState() { wasi_canvas_draw_method_canvas_save(canvas) }
    public func restoreGState() { wasi_canvas_draw_method_canvas_restore(canvas) }

    // ── CTM ──────────────────────────────────────────────────────────────────
    public func translateBy(x: CGFloat, y: CGFloat) {
        wasi_canvas_draw_method_canvas_translate(canvas, Float(x), Float(y))
    }
    public func scaleBy(x: CGFloat, y: CGFloat) {
        wasi_canvas_draw_method_canvas_scale(canvas, Float(x), Float(y))
    }
    public func rotate(by radians: CGFloat) {
        wasi_canvas_draw_method_canvas_rotate(canvas, Float(radians * 180.0 / 3.141592653589793))
    }

    // ── color / line state ─────────────────────────────────────────────────────
    public func setFillColor(_ c: CGColor) { fillColor = c }
    public func setStrokeColor(_ c: CGColor) { strokeColor = c }
    public func setLineWidth(_ w: CGFloat) { lineWidth = w }

    // ── current path (serialized to SVG path-data for wasi:canvas) ──────────────
    public func beginPath() { pathData = "" }
    public func move(to p: CGPoint) { pathData += "M \(p.x) \(p.y) " }
    public func addLine(to p: CGPoint) { pathData += "L \(p.x) \(p.y) " }
    public func addQuadCurve(to p: CGPoint, control c: CGPoint) {
        pathData += "Q \(c.x) \(c.y) \(p.x) \(p.y) "
    }
    public func addCurve(to p: CGPoint, control1 c1: CGPoint, control2 c2: CGPoint) {
        pathData += "C \(c1.x) \(c1.y) \(c2.x) \(c2.y) \(p.x) \(p.y) "
    }
    public func addRect(_ r: CGRect) {
        let x = r.origin.x, y = r.origin.y, w = r.size.width, h = r.size.height
        pathData += "M \(x) \(y) L \(x + w) \(y) L \(x + w) \(y + h) L \(x) \(y + h) Z "
    }
    public func closePath() { pathData += "Z " }

    // ── fills / strokes ────────────────────────────────────────────────────────
    public func clear(_ c: CGColor) { wasi_canvas_draw_method_canvas_clear(canvas, c.argb) }

    public func fill(_ r: CGRect) {
        var p = paint(style: WASI_CANVAS_TYPES_PAINT_STYLE_FILL, color: fillColor)
        var rect = wasiRect(r)
        wasi_canvas_draw_method_canvas_draw_rect(canvas, &rect, &p)
    }
    public func stroke(_ r: CGRect) {
        var p = paint(style: WASI_CANVAS_TYPES_PAINT_STYLE_STROKE, color: strokeColor)
        var rect = wasiRect(r)
        wasi_canvas_draw_method_canvas_draw_rect(canvas, &rect, &p)
    }
    public func fillPath() { drawPath(style: WASI_CANVAS_TYPES_PAINT_STYLE_FILL, color: fillColor) }
    public func strokePath() { drawPath(style: WASI_CANVAS_TYPES_PAINT_STYLE_STROKE, color: strokeColor) }

    // ── lowering helpers ───────────────────────────────────────────────────────
    private func paint(style: Int32, color: CGColor) -> wasi_canvas_types_paint_t {
        var p = wasi_canvas_types_paint_t()       // zero-init: options = none
        p.style = UInt8(style)
        p.color = color.argb
        p.alpha = 255
        p.anti_alias = true
        p.stroke_width = Float(lineWidth)
        return p
    }
    private func wasiRect(_ r: CGRect) -> wasi_canvas_types_rect_t {
        wasi_canvas_types_rect_t(x: Float(r.origin.x), y: Float(r.origin.y),
                                 width: Float(r.size.width), height: Float(r.size.height))
    }
    private func drawPath(style: Int32, color: CGColor) {
        guard !pathData.isEmpty else { return }
        var p = paint(style: style, color: color)
        var s = swift_spike_string_t()
        pathData.withCString { swift_spike_string_set(&s, $0) }
        wasi_canvas_draw_method_canvas_draw_path(
            canvas, &s, UInt8(WASI_CANVAS_TYPES_FILL_RULE_NONZERO), &p)
    }
}
