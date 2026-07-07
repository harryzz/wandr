---
name: audit-wit-consumers-scan-binaries
description: "Finding which guests import a WASI/WIT interface (blast radius before a host-impl or WIT change) — scan BUILT components with `wasm-tools component wit`, NOT source grep; toolchains import networking/std interfaces implicitly (dead imports) that source grep misses."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 8be49912-f19b-4ab9-8c48-7e6978e7a1c6
---

To audit **which guests import/use a WIT interface** — the blast radius before
changing a host impl (e.g. p2→p3 `wasi:tls`/`wasi:sockets`) or a shared WIT —
**scan the built components with `wasm-tools component wit <file>` and grep the
import list**, NOT a source-level grep of `.wit`/`Cargo.toml`.

```bash
find apps -name '*.wasm' | grep -v /target/ | while read c; do
  hits=$(wasm-tools component wit "$c" 2>/dev/null | grep -iE 'wasi:sockets|wasi:tls')
  [ -n "$hits" ] && { echo "### $c"; echo "$hits"; }
done
```

**Why:** source grep only sees *explicit* imports and Rust Cargo deps. It **misses
toolchain-implicit imports**: managed / full-libc runtimes (**.NET via
componentize-dotnet**, TinyGo, anything on the full WASI SDK) declare the
**entire standard WASI world** (`sockets`/`filesystem`/`clocks`/`random`/`io`)
even for a **UI-only app** — because the runtime/BCL *references* those syscalls.
Lean custom-world Rust guests (the chrome guests import just `wasi:cli/stderr`)
only import what they actually reference. **Importing ≠ using — but linking is by
declaration**, so a dead import still must be satisfied by the host at
instantiation or the component fails to load.

Concrete miss it caught (task 115, p2→p3 wasi:tls/sockets): a source grep found
only the Rust networking guests; the binary scan revealed **`wandr.avalonia.demo`
(.NET) imports `wasi:sockets@0.2.0` as a dead import** — a non-Rust consumer that
a naive p2→p3 host swap would break. Also surfaced version fragmentation
(`wasi:sockets` at 0.2.0 / 0.2.9 / 0.2.12 across guests).

**How to apply:**
- Before any host-impl or shared-WIT change, run the scan across all built
  components; regenerate per release (imports change with toolchain versions).
- The scan is the **source of truth for ALL languages**. Rust `#[deprecated]`
  warnings are **Rust-only** and never surface non-Rust consumers — don't rely on
  them for the consumer list.
- Drop an old interface (e.g. p2) only when the scan list is empty — modulo dead
  imports that just need the host to keep satisfying them. Prefer **dual-serve**
  (keep old + add new additively) over replace.

Related: [[feedback_shared_wit_rebuild_all_consumers]] (a shared-type ABI break
hits every importer); [[reference_missing_instance_error_stale_zygote]]
("resource implementation is missing" = an imported instance the host didn't
provide). Live use: `tasks/115-signal-transport-wasip3-async.md` blast-radius.
