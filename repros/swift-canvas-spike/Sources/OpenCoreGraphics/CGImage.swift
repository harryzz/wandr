//
//  CGImage.swift
//  OpenCoreGraphics
//
//  Created by Kyle on 1/11/26.  (upstream: an empty Hashable stub)
//  wandr fork (task 114): backed by a wasi:canvas host image resource. Create via
//  CGContext.makeImage(rgba:width:height:); the handle is dropped on deinit.
import CWASICanvas

public final class CGImage: Hashable, @unchecked Sendable {
    let handle: wasi_canvas_draw_own_image_t
    public let width: Int
    public let height: Int

    init(handle: wasi_canvas_draw_own_image_t, width: Int, height: Int) {
        self.handle = handle
        self.width = width
        self.height = height
    }

    deinit {
        wasi_canvas_types_image_drop_own(wasi_canvas_types_own_image_t(__handle: handle.__handle))
    }

    public func hash(into hasher: inout Hasher) { hasher.combine(ObjectIdentifier(self)) }
    public static func == (lhs: CGImage, rhs: CGImage) -> Bool { lhs === rhs }
}
