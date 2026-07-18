#ifndef CWANDR_BOOT_H
#define CWANDR_BOOT_H

// Run the guest app's own @main-generated reactor entry exactly once.
//
// A wandr guest is a wasip1 REACTOR (-mexec-model=reactor): there is no _start, so Swift's `@main`
// synthesizes `__main_argc_argv` but nothing auto-calls it. wandr-runtime's `on-init` calls this
// wrapper (from bootWandrReactorApp) to run the app's `App.main()` once. Doing the call in C (rather
// than a Swift @_silgen_name decl) guarantees the correct C calling convention `(i32,i32)->i32` — a
// Swift-side declaration lowers with Swift's convention and wasm-ld then traps on a signature mismatch.
int wandr_run_app_main(void);

#endif
