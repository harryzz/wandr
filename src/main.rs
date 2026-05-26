// Task 36 step 7 — Rust-side cross-app-dep smoke consumer.
//
// Imports `war:markdown/renderer@0.1.0` (a system-bundled component
// installed under `/data/wart/system-apps/`), calls its `render` export
// with a hardcoded CommonMark string, and exits 0 if the resulting
// document has at least one block — non-zero otherwise.
//
// Exists because the Kotlin smoke (`wart-app-md-smoke/`) hits a
// pre-existing Kotlin/Wasm + WASI-command-adapter throw at module init
// that masks the actual call. Rust on wasm32-wasip2 produces a clean
// `wasi:cli/command` shape with no such issue.

wit_bindgen::generate!({
    // `consumer` world imports war:markdown/renderer (the producer side
    // lives in markdown-renderer/, which uses `renderer-world` that
    // EXPORTS the same interface). wit-bindgen generates calling stubs
    // here, Guest impls there. `generate_all` tells wit-bindgen to emit
    // bindings for every interface in the world (only the markdown one
    // here).
    world: "consumer",
    path: "wit",
    generate_all,
});

use war::markdown::renderer::render;

fn main() {
    // Call the dep. If the cross-app linker proxy is wired correctly,
    // this dispatches into `wire_markdown_dep`'s closure in
    // wart-host/src/app_loader.rs → `renderer.call_render(...)` in the
    // markdown_renderer instance (deserialized into the same Store) →
    // returns the parsed Document back.
    let doc = render("# Hello\n\n**bold** world.");
    let blocks = doc.blocks.len();
    // Print via stderr (routed to logcat by wart-host's wasi_stderr
    // shim) so the log line shows up in the standard run_once trace.
    eprintln!("md-smoke-rust: render() returned {blocks} block(s)");
    if blocks == 0 {
        std::process::exit(1);
    }
}
