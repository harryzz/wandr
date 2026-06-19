// DisplayListRenderer.swift — the wandr drawing backend: walk a DisplayList and draw it
// into a CGContext (→ wasi:canvas → skia/EGL). This is the OpenSwiftUI-side seam Option B
// targets — parallel to UIKitDisplayList/AppKitDisplayList/the GTK renderer, but for wandr.
//
// The walk is recursive exactly like OpenSwiftUI's: each Item positions to its frame; a
// `.content` draws; an `.effect` pushes graphics state (opacity layer / clip / transform)
// and recurses into the wrapped sub-DisplayList. Every op here is an already-shipped,
// device-verified CGContext call.
import WandrCG

func render(_ list: DisplayList, into cg: CGContext) {
    for item in list.items {
        cg.saveGState()
        // Position the item's local coordinate space at its frame origin.
        cg.translateBy(x: item.frame.origin.x, y: item.frame.origin.y)
        let local = CGRect(x: 0, y: 0, width: item.frame.width, height: item.frame.height)

        switch item.value {
        case .content(let content):
            switch content {
            case .color(let c):
                cg.setFillColor(c)
                cg.fill(local)
            case .shape(let path, let c):
                cg.setFillColor(c)
                cg.beginPath()
                cg.addPath(path)
                cg.fillPath()
            case .text(let s, let size, let c):
                cg.drawString(s, at: CGPoint(x: 0, y: 0), size: size,
                              color: c, maxWidth: item.frame.width)
            case .image(let img):
                cg.draw(img, in: local)
            }

        case .effect(let effect, let child):
            switch effect {
            case .opacity(let a):
                cg.beginTransparencyLayer(alpha: a)
                render(child, into: cg)      // sub-list composited at `a`
                cg.endTransparencyLayer()
            case .clip(let path):
                cg.beginPath()
                cg.addPath(path)
                cg.clip()
                render(child, into: cg)      // sub-list clipped to `path`
            case .transform(let t):
                cg.concatenate(t)
                render(child, into: cg)      // sub-list under the transform
            }
        }

        cg.restoreGState()
    }
}
