// [wandr Phase 2] `Combine` shim → OpenCombine (ObservableObject / @Published / operators), so
// eleev's `import Combine` resolves unmodified on wasm. (OpenCombineFoundation doesn't build on
// wasm — URLSession/OperationQueue/unfair-locks — so its NotificationCenter.publisher bridge is
// reimplemented minimally in NotificationBridge.swift instead.)
@_exported import OpenCombine
