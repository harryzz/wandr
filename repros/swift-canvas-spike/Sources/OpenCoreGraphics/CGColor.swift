//
//  CGColor.swift
//  OpenCoreGraphics
//
//  Added by the wandr fork (task 114): upstream OpenCoreGraphics has no CGColor yet,
//  but CGContext needs one. Minimal non-premultiplied sRGB color.

public import Foundation

public struct CGColor: Hashable, Sendable {
    public var red: CGFloat
    public var green: CGFloat
    public var blue: CGFloat
    public var alpha: CGFloat

    public init(red: CGFloat, green: CGFloat, blue: CGFloat, alpha: CGFloat) {
        self.red = red
        self.green = green
        self.blue = blue
        self.alpha = alpha
    }

    /// 0xAARRGGBB — the wandr `wasi:canvas` color encoding.
    public var argb: UInt32 {
        func b(_ v: CGFloat) -> UInt32 { UInt32(max(0, min(1, v)) * 255 + 0.5) }
        return (b(alpha) << 24) | (b(red) << 16) | (b(green) << 8) | b(blue)
    }
}
