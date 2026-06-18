//
//  CGGradient.swift
//  OpenCoreGraphics
//
//  Added by the wandr fork (task 114): upstream OpenCoreGraphics has no CGGradient.
//  Colors with optional stop locations (0..1); CGContext.drawLinear/RadialGradient
//  lower it to wasi:canvas graphics.{linear,radial}-gradient shaders.

public import Foundation

public struct CGGradient: Sendable {
    public var colors: [CGColor]
    public var locations: [CGFloat]

    /// `locations` may be empty → stops are distributed evenly.
    public init(colors: [CGColor], locations: [CGFloat] = []) {
        self.colors = colors
        self.locations = locations
    }

    /// (offset, color) stops, offsets ascending in 0..1.
    func stops() -> [(CGFloat, CGColor)] {
        let n = colors.count
        guard n > 0 else { return [] }
        return colors.enumerated().map { i, c in
            let loc = i < locations.count ? locations[i]
                : (n == 1 ? 0 : CGFloat(i) / CGFloat(n - 1))
            return (loc, c)
        }
    }
}
