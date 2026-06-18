//
//  CGContext.swift
//  OpenCoreGraphics
//
//  Created by Kyle on 1/11/26.  (upstream: Status: Empty)
//  wandr fork (task 114): implemented over wasi:canvas — each CoreGraphics call
//  lowers to a wasi:canvas verb (mapping: docs/swift-openswiftui-wandr-feasibility.md).
//  The 3 mapping "gaps" are emulated here with existing wasi:canvas verbs (no
//  contract change): line dash (guest path-walk), offset+color drop shadow
//  (draw-twice + mask-blur), and alpha mask-clip (save-layer + dst-in blend).

public import Foundation
import CSwiftSpike

/// CoreGraphics blend modes, mapped to wasi:canvas blend-mode. Raw values match
/// Apple's CGBlendMode.
public enum CGBlendMode: Int32, Sendable {
    case normal = 0, multiply, screen, overlay, darken, lighten, colorDodge, colorBurn
    case softLight, hardLight, difference, exclusion, hue, saturation, color, luminosity
    case clear, copy, sourceIn, sourceOut, sourceAtop, destinationOver, destinationIn
    case destinationOut, destinationAtop, xor, plusDarker, plusLighter

    var wasi: UInt8 {
        switch self {
        case .normal: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC_OVER)
        case .multiply: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_MULTIPLY)
        case .screen: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SCREEN)
        case .overlay: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_OVERLAY)
        case .darken: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DARKEN)
        case .lighten: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_LIGHTEN)
        case .colorDodge: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_COLOR_DODGE)
        case .colorBurn: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_COLOR_BURN)
        case .softLight: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SOFT_LIGHT)
        case .hardLight: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_HARD_LIGHT)
        case .difference: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DIFFERENCE)
        case .exclusion: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_EXCLUSION)
        case .hue: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_HUE)
        case .saturation: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SATURATION)
        case .color: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_COLOR)
        case .luminosity: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_LUMINOSITY)
        case .clear: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_CLEAR)
        case .copy: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC)
        case .sourceIn: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC_IN)
        case .sourceOut: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC_OUT)
        case .sourceAtop: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC_ATOP)
        case .destinationOver: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DST_OVER)
        case .destinationIn: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DST_IN)
        case .destinationOut: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DST_OUT)
        case .destinationAtop: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_DST_ATOP)
        case .xor: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_XOR)
        case .plusLighter: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_PLUS)
        case .plusDarker: return UInt8(WASI_CANVAS_TYPES_BLEND_MODE_SRC_OVER) // no wasi analog
        }
    }
}

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
    private var blendMode: CGBlendMode = .normal
    // dash
    private var dashLengths: [CGFloat] = []
    private var dashPhase: CGFloat = 0
    // shadow
    private var shadow: (dx: CGFloat, dy: CGFloat, blur: CGFloat, color: CGColor)?

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
    public func setBlendMode(_ mode: CGBlendMode) { blendMode = mode }

    /// Gap #1 — line dash. wasi:canvas has no dash; we dash the geometry guest-side.
    public func setLineDash(phase: CGFloat, lengths: [CGFloat]) {
        dashPhase = phase
        dashLengths = lengths
    }
    /// Gap #2 — offset+color drop shadow. `mask-blur` has no offset/color, so we
    /// draw each shape twice (offset shadow copy with mask-blur, then the real shape).
    public func setShadow(offset: (CGFloat, CGFloat), blur: CGFloat, color: CGColor) {
        shadow = (offset.0, offset.1, blur, color)
    }
    public func clearShadow() { shadow = nil }

    // ── transparency layers (Gap #3 building block) ─────────────────────────────
    public func beginTransparencyLayer(alpha: UInt8 = 255) {
        wasi_canvas_draw_method_canvas_save_layer(canvas, nil, alpha)
    }
    public func endTransparencyLayer() { wasi_canvas_draw_method_canvas_restore(canvas) }

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

    public func fill(_ r: CGRect) { drawRect(r, style: WASI_CANVAS_TYPES_PAINT_STYLE_FILL, color: fillColor) }
    public func stroke(_ r: CGRect) { drawRect(r, style: WASI_CANVAS_TYPES_PAINT_STYLE_STROKE, color: strokeColor) }
    public func fillEllipse(in r: CGRect) { drawOval(r, style: WASI_CANVAS_TYPES_PAINT_STYLE_FILL, color: fillColor) }
    public func fillPath() {
        emitWithShadow(svg: svgPathData(elements), style: WASI_CANVAS_TYPES_PAINT_STYLE_FILL, color: fillColor)
    }
    public func strokePath() {
        let svg = dashLengths.isEmpty ? svgPathData(elements) : dashedSVG()
        emitWithShadow(svg: svg, style: WASI_CANVAS_TYPES_PAINT_STYLE_STROKE, color: strokeColor)
    }

    // ── lowering ─────────────────────────────────────────────────────────────────
    private func drawRect(_ r: CGRect, style: Int32, color: CGColor) {
        var rect = wasiRect(r)
        if let s = shadow {
            wasi_canvas_draw_method_canvas_save(canvas)
            wasi_canvas_draw_method_canvas_translate(canvas, Float(s.dx), Float(s.dy))
            var sp = paint(style: style, color: s.color, blur: s.blur)
            wasi_canvas_draw_method_canvas_draw_rect(canvas, &rect, &sp)
            wasi_canvas_draw_method_canvas_restore(canvas)
        }
        var p = paint(style: style, color: color, blur: nil)
        wasi_canvas_draw_method_canvas_draw_rect(canvas, &rect, &p)
    }
    private func drawOval(_ r: CGRect, style: Int32, color: CGColor) {
        var rect = wasiRect(r)
        var p = paint(style: style, color: color, blur: nil)
        wasi_canvas_draw_method_canvas_draw_oval(canvas, &rect, &p)
    }
    private func emitWithShadow(svg: String, style: Int32, color: CGColor) {
        guard !svg.isEmpty else { return }
        if let s = shadow {
            wasi_canvas_draw_method_canvas_save(canvas)
            wasi_canvas_draw_method_canvas_translate(canvas, Float(s.dx), Float(s.dy))
            emitPath(svg, style: style, color: s.color, blur: s.blur)
            wasi_canvas_draw_method_canvas_restore(canvas)
        }
        emitPath(svg, style: style, color: color, blur: nil)
    }
    private func emitPath(_ svg: String, style: Int32, color: CGColor, blur: CGFloat?) {
        var p = paint(style: style, color: color, blur: blur)
        var s = swift_spike_string_t()
        svg.withCString { swift_spike_string_set(&s, $0) }
        wasi_canvas_draw_method_canvas_draw_path(
            canvas, &s, UInt8(WASI_CANVAS_TYPES_FILL_RULE_NONZERO), &p)
    }
    private func paint(style: Int32, color: CGColor, blur: CGFloat?) -> wasi_canvas_types_paint_t {
        var p = wasi_canvas_types_paint_t()        // zero-init: option fields = none
        p.style = UInt8(style)
        p.color = color.argb
        p.alpha = 255
        p.anti_alias = true
        p.stroke_width = Float(lineWidthValue)
        p.stroke_cap = UInt8(lineCapValue.rawValue)
        p.stroke_join = UInt8(lineJoinValue.rawValue)
        p.stroke_miter = Float(miterLimitValue)
        p.blend = blendMode.wasi
        if let b = blur {
            p.blur.is_some = true
            p.blur.val.style = UInt8(WASI_CANVAS_TYPES_BLUR_STYLE_NORMAL)
            p.blur.val.sigma = Float(b)
        }
        return p
    }
    private func wasiRect(_ r: CGRect) -> wasi_canvas_types_rect_t {
        wasi_canvas_types_rect_t(x: Float(r.origin.x), y: Float(r.origin.y),
                                 width: Float(r.size.width), height: Float(r.size.height))
    }

    // ── path → SVG path-data (plain + dashed) ───────────────────────────────────
    private func svgPathData(_ els: [PathElement]) -> String {
        var out = ""
        for e in els {
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

    /// Flatten the current path into polylines (curves sampled) for dashing.
    private func subpaths() -> [[CGPoint]] {
        var paths: [[CGPoint]] = []
        var cur: [CGPoint] = []
        var start = CGPoint(x: 0, y: 0)
        var last = CGPoint(x: 0, y: 0)
        func flush() { if cur.count > 1 { paths.append(cur) }; cur = [] }
        func ensure() { if cur.isEmpty { cur = [last] } }
        for e in elements {
            switch e {
            case let .moveToPoint(p): flush(); cur = [p]; start = p; last = p
            case let .addLineToPoint(p): ensure(); cur.append(p); last = p
            case let .addQuadCurveToPoint(c, p):
                ensure()
                let n = 16
                for i in 1...n {
                    let t = CGFloat(i) / CGFloat(n), mt = 1 - t
                    cur.append(CGPoint(x: mt*mt*last.x + 2*mt*t*c.x + t*t*p.x,
                                       y: mt*mt*last.y + 2*mt*t*c.y + t*t*p.y))
                }
                last = p
            case let .addCurveToPoint(c1, c2, p):
                ensure()
                let n = 24
                for i in 1...n {
                    let t = CGFloat(i) / CGFloat(n), mt = 1 - t
                    cur.append(CGPoint(
                        x: mt*mt*mt*last.x + 3*mt*mt*t*c1.x + 3*mt*t*t*c2.x + t*t*t*p.x,
                        y: mt*mt*mt*last.y + 3*mt*mt*t*c1.y + 3*mt*t*t*c2.y + t*t*t*p.y))
                }
                last = p
            case .closeSubpath: ensure(); cur.append(start); flush(); last = start
            }
        }
        flush()
        return paths
    }

    /// Gap #1 — split the path into dash on/off runs and emit only the "on" runs.
    private func dashedSVG() -> String {
        let pattern = dashLengths.filter { $0 > 0 }
        guard !pattern.isEmpty else { return svgPathData(elements) }
        let total = pattern.reduce(0, +)
        var out = ""
        for pts in subpaths() where pts.count > 1 {
            var idx = 0, on = true, rem = pattern[0]
            var phase = dashPhase.truncatingRemainder(dividingBy: total)
            if phase < 0 { phase += total }
            while phase > 1e-9 {
                if phase >= rem { phase -= rem; idx = (idx + 1) % pattern.count; on.toggle(); rem = pattern[idx] }
                else { rem -= phase; phase = 0 }
            }
            var penDown = false
            for i in 1..<pts.count {
                var a = pts[i - 1]; let b = pts[i]
                var segLen = (b.x - a.x).magnitude == 0 && (b.y - a.y).magnitude == 0
                    ? 0 : (((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y))).squareRoot()
                if segLen == 0 { continue }
                let dx = (b.x - a.x) / segLen, dy = (b.y - a.y) / segLen
                while segLen > 1e-9 {
                    let step = Swift.min(rem, segLen)
                    let nx = a.x + dx * step, ny = a.y + dy * step
                    if on {
                        if !penDown { out += "M \(a.x) \(a.y) "; penDown = true }
                        out += "L \(nx) \(ny) "
                    } else { penDown = false }
                    a = CGPoint(x: nx, y: ny)
                    segLen -= step
                    rem -= step
                    if rem <= 1e-6 {
                        idx = (idx + 1) % pattern.count; on.toggle(); rem = pattern[idx]
                        if !on { penDown = false }
                    }
                }
            }
        }
        return out
    }
}
