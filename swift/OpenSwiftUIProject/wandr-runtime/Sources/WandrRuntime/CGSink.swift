//
//  CGSink.swift
//  WandrRuntime
//
//  The shared WandrDrawSink conformer — every OpenSwiftUI-on-wandr app's DisplayList ->
//  wasi:canvas bridge. Per Sources/T2iles/RULES.md, drawing-feature forwarding (clip, path
//  fill, shadow, 3D, images, ...) is added ONCE here, never duplicated per app.

import OpenSwiftUI
import OpenCoreGraphicsWASICanvas

final class CGSink: WandrDrawSink {
    nonisolated(unsafe) var cg: CGContext?
    // Decoded-image cache, keyed by the DisplayList's per-image identity — a static bundle
    // image redraws every frame; decode host-side (wasi:canvas graphics.decode-image) once,
    // reuse the resource handle thereafter.
    nonisolated(unsafe) private var imageCache: [String: CGImageHandle] = [:]

    func beginFrame(width: Double, height: Double, version: UInt32) {}

    func fillRect(
        x: Double, y: Double, width: Double, height: Double,
        red: Float, green: Float, blue: Float, opacity: Float
    ) {
        guard let cg else { return }
        cg.setFillColor(CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
        cg.fill(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)))
    }

    func drawText(
        _ text: String, x: Double, y: Double, width: Double, height: Double,
        fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float,
        fontFamily: String
    ) {
        guard let cg else { return }
        cg.drawString(
            text, at: CGPoint(x: CGFloat(x), y: CGFloat(y)), size: CGFloat(fontSize),
            color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)),
            maxWidth: CGFloat(width), family: fontFamily
        )
    }

    func fillPath(
        svgPath: String, x: Double, y: Double, width: Double, height: Double,
        red: Float, green: Float, blue: Float, opacity: Float
    ) {
        guard let cg else { return }
        cg.setFillColor(CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
        cg.fill(svgPath: svgPath)
    }

    func pushClip(svgPath: String) {
        guard let cg else { return }
        cg.saveGState()
        cg.clip(svgPath: svgPath)
    }
    func popClip() { cg?.restoreGState() }

    var wandrSupportsProjection: Bool { true }
    func saveState() { cg?.saveGState() }
    func restoreState() { cg?.restoreGState() }
    func concat(
        m00: Double, m01: Double, m02: Double,
        m10: Double, m11: Double, m12: Double,
        m20: Double, m21: Double, m22: Double
    ) {
        cg?.concat3x3(
            CGFloat(m00), CGFloat(m01), CGFloat(m02),
            CGFloat(m10), CGFloat(m11), CGFloat(m12),
            CGFloat(m20), CGFloat(m21), CGFloat(m22)
        )
    }

    func fillPathShadow(
        svgPath: String, dx: Double, dy: Double, blur: Double,
        red: Float, green: Float, blue: Float, opacity: Float
    ) {
        cg?.fillShadowPath(
            svgPath, dx: CGFloat(dx), dy: CGFloat(dy), blur: CGFloat(blur),
            color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity))
        )
    }

    func drawImage(
        data: [UInt8], name: String, pixelWidth: Int, pixelHeight: Int,
        x: Double, y: Double, width: Double, height: Double, opacity: Float
    ) {
        guard let cg else { return }
        let image: CGImageHandle?
        if let cached = imageCache[name] {
            image = cached
        } else {
            image = cg.decodeImage(encodedData: data, pixelWidth: pixelWidth, pixelHeight: pixelHeight)
            if image == nil {
                rlog("drawImage: decodeImage FAILED for '\(name)' (\(data.count) bytes, \(pixelWidth)x\(pixelHeight))")
            }
            imageCache[name] = image
        }
        guard let image else { return }
        cg.drawImageFitting(
            image, in: CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)),
            opacity: opacity
        )
    }

    func endFrame() {}
}
