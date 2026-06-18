//
//  CGContext.swift
//  OpenCoreGraphics
//
//  Created by Kyle on 1/11/26.  (upstream: Status: Empty)
//  wandr fork (task 114): implemented over wasi:canvas — each CoreGraphics call
//  lowers to a wasi:canvas verb (mapping: docs/swift-openswiftui-wandr-feasibility.md).
//  This is the body that fills OpenCoreGraphics's empty CGContext; the geometry/path
//  types (CGPoint/CGRect via Foundation, CGPath/PathElement/CGLineCap/CGLineJoin) are
//  OpenCoreGraphics's own.

public import Foundation
import CSwiftSpike

public final class CGContext: Hashable {
    // The host-rendered target. CoreGraphics doesn't expose this; the wandr embedder
    // hands the guest a wasi:canvas buffer and wraps it (see the guest's on_frame).
    private let canvas: wasi_canvas_draw_borrow_canvas_t

    // Graphics state — CoreGraphics is imperative: a mutable current path + paint state.
    private var elements: [PathElement] = []
    private var fillColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)
    private var strokeColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)
    private var lineWidthValue: CGFloat = 1
    private var lineCapValue: CGLineCap = .butt
    private var lineJoinValue: CGLineJoin = .miter
    private var miterLimitValue: CGFloat = 10

    /// wandr extension: build a context over a wasi:canvas buffer.
    public init(canvas: wasi_canvas_draw_borrow_canvas_t) { self.canvas = canvas }

    public func hash(into hasher: inout Hasher) { hasher.combine(ObjectIdentifier(self)) }
    public static func == (lhs: CGContext, rhs: CGContext) -> Bool { lhs === rhs }

    // ── graphics-state stack ────────────────────────────────────────────────────
    public func saveGState() { wasi_canvas_draw_method_canvas_save(canvas) }
    public func restoreGState() { wasi_canvas_draw_method_canvas_restore(canvas) }

    // ── CTM ─────────────────────────────────────────────────────────────────────
    public func translateBy(x: CGFloat, y: CGFloat) {
        wasi_canvas_draw_method_canvas_translate(canvas, Float(x), Float(y))
    }
    public func scaleBy(x: CGFloat, y: CGFloat) {
        wasi_canvas_draw_method_canvas_scale(canvas, Float(x), Float(y))
    }
    public func rotate(by radians: CGFloat) {
        wasi_canvas_draw_method_canvas_rotate(canvas, Float(radians * 180.0 / .pi))
    }

    // ── paint state ─────────────────────────────────────────────────────────────
    public func setFillColor(_ c: CGColor) { fillColor = c }
    public func setStrokeColor(_ c: CGColor) { strokeColor = c }
    public func setLineWidth(_ w: CGFloat) { lineWidthValue = w }
    public func setLineCap(_ cap: CGLineCap) { lineCapValue = cap }
    public func setLineJoin(_ join: CGLineJoin) { lineJoinValue = join }
    public func setMiterLimit(_ limit: CGFloat) { miterLimitValue = limit }

    // ── current path (uses OpenCoreGraphics's PathElement) ──────────────────────
    public func beginPath() { elements.removeAll(keepingCapacity: true) }
    public func move(to p: CGPoint) { elements.append(.moveToPoint(p)) }
    public func addLine(to p: CGPoint) { elements.append(.addLineToPoint(p)) }
    public func addQuadCurve(to p: CGPoint, control c: CGPoint) {
        elements.append(.addQuadCurveToPoint(c, p))
    }
    public func addCurve(to p: CGPoint, control1 c1: CGPoint, control2 c2: CGPoint) {
        elements.append(.addCurveToPoint(c1, c2, p))
    }
    public func addRect(_ r: CGRect) {
        let x = r.origin.x, y = r.origin.y, w = r.size.width, h = r.size.height
        elements.append(.moveToPoint(CGPoint(x: x, y: y)))
        elements.append(.addLineToPoint(CGPoint(x: x + w, y: y)))
        elements.append(.addLineToPoint(CGPoint(x: x + w, y: y + h)))
        elements.append(.addLineToPoint(CGPoint(x: x, y: y + h)))
        elements.append(.closeSubpath)
    }
    public func addPath(_ path: CGPath) { elements.append(contentsOf: path.elements) }
    public func closePath() { elements.append(.closeSubpath) }

    // ── drawing ─────────────────────────────────────────────────────────────────
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

    // ── lowering helpers ─────────────────────────────────────────────────────────
    private func paint(style: Int32, color: CGColor) -> wasi_canvas_types_paint_t {
        var p = wasi_canvas_types_paint_t()        // zero-init: option fields = none
        p.style = UInt8(style)
        p.color = color.argb
        p.alpha = 255
        p.anti_alias = true
        p.stroke_width = Float(lineWidthValue)
        p.stroke_cap = UInt8(lineCapValue.rawValue)
        p.stroke_join = UInt8(lineJoinValue.rawValue)
        p.stroke_miter = Float(miterLimitValue)
        return p
    }
    private func wasiRect(_ r: CGRect) -> wasi_canvas_types_rect_t {
        wasi_canvas_types_rect_t(x: Float(r.origin.x), y: Float(r.origin.y),
                                 width: Float(r.size.width), height: Float(r.size.height))
    }
    private func drawPath(style: Int32, color: CGColor) {
        let d = svgPathData()
        guard !d.isEmpty else { return }
        var p = paint(style: style, color: color)
        var s = swift_spike_string_t()
        d.withCString { swift_spike_string_set(&s, $0) }
        wasi_canvas_draw_method_canvas_draw_path(
            canvas, &s, UInt8(WASI_CANVAS_TYPES_FILL_RULE_NONZERO), &p)
    }
    /// Serialize the OpenCoreGraphics current path to an SVG path-data string
    /// (the form wasi:canvas's draw-path/clip-path take).
    private func svgPathData() -> String {
        var out = ""
        for e in elements {
            switch e {
            case let .moveToPoint(p): out += "M \(p.x) \(p.y) "
            case let .addLineToPoint(p): out += "L \(p.x) \(p.y) "
            case let .addQuadCurveToPoint(c, p): out += "Q \(c.x) \(c.y) \(p.x) \(p.y) "
            case let .addCurveToPoint(c1, c2, p):
                out += "C \(c1.x) \(c1.y) \(c2.x) \(c2.y) \(p.x) \(p.y) "
            case .closeSubpath: out += "Z "
            }
        }
        return out
    }
}
