// Task-67 Phase-0 compile probe. The point is to compile the `libsignal-service`
// dependency tree for wasm32-wasip2 and catalog what breaks — not to run logic.
// Referencing one public symbol keeps the dep firmly in the build graph.

#[allow(unused_imports)]
use libsignal_service::configuration::SignalServers;

fn main() {
    // Force a reference so the crate is linked, not just compiled-then-pruned.
    let _ = SignalServers::Production;
    println!("signal-phase0 probe: libsignal-service linked");
}
