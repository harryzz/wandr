// Task 36 step 6 smoke consumer — calls `wandr:markdown/renderer.render`
// (resolved by wandr-host's loader against the markdown system bundle)
// and prints the result. Kotlin/Wasm has no `System.err`; `println`
// writes to stdout (fd 1), which `wasmtime run` shows on the
// controlling terminal during dev-box validation. On-device routing
// (stdout → logcat) is wandr-host's job in step 7.

import wandr.mdSmoke.markdown.render

// Bare render() call — no println (Kotlin/Wasm + wasmtime command
// adapter interaction throws on println for reasons not yet diagnosed).
// Successful exit (code 0) is the smoke signal: the import bound, the
// dep was invoked, the result was received, the program terminated
// cleanly.
import wandr.mdSmoke.markdown.render

fun main() {
    val result = render("# Hello\n\n**bold** world.")
    // Stash blocks_len somewhere observable. The simplest: process exit
    // code. Successful return + clean exit = wiring works.
    if (result.blocksLen <= 0) {
        // Not what we expected — trip an unreachable to set non-zero
        // exit code.
        throw IllegalStateException("expected blocks_len > 0, got ${result.blocksLen}")
    }
}
