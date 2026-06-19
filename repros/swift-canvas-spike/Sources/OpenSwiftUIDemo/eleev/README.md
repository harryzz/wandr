# eleev/swiftui-2048 sources (vendored for the OpenSwiftUI-on-wandr port)

Verbatim game-render sources from https://github.com/eleev/swiftui-2048 (MIT), pulled to run
eleev's ACTUAL SwiftUI on the wandr OpenSwiftUI/wasm stack. Adaptations are mechanical only:

- `import SwiftUI` → `import OpenSwiftUI` (+ `import Foundation` for CGFloat/CGPoint/CGRect/log2).
- Environment-key `defaultValue` marked `nonisolated(unsafe)` (Swift 6 strict concurrency).
- `.drawingGroup(...)` commented out (rasterization optimization not yet in OpenSwiftUI).
- PreviewProvider blocks stripped.

NOT yet pulled: the app chrome (App/Scene, side menu, settings, modals) and the audio/AppStorage/
notification layers (to be gated/shimmed). Current entry renders `TileBoardView` with a sample
matrix (see ../main.swift).

Status: compiles + the board frame/background render on-device. Tiles need the next OpenSwiftUI
gaps (ShapeStyleRendering.render(style:) + nested-GeometryReader sizing).
