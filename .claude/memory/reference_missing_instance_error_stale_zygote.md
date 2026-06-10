---
name: reference_missing_instance_error_stale_zygote
description: "wasmtime \"resource implementation is missing\" really means the WHOLE import instance is absent from the linker — chief cause here is a stale zygote image after a host redeploy; wac+resource compositions work fine"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

**Misleading wasmtime error, decoded** (verified against wasmtime source +
on-device repro, 2026-06-10): `component imports instance X, but a matching
implementation was not found in the linker: instance export 'aead-key' has the
wrong type: resource implementation is missing` does NOT mean a resource-specific
linking problem. `matching.rs:50-52` — when the linker has **no instance named X at
all**, wasmtime still descends into the expected instance's exports with
`actual = None`, and the FIRST export checked (here the resource) produces the
error. So it usually means: this host process never ran `X`'s `add_to_linker`.

**The trap that produced it:** pushing a new wandr-host binary to disk does NOT
change the already-running **zygote's** image; apps launched via the arbiter FORK
FROM THE OLD ZYGOTE and miss every newly-linked interface, while `--run-once`
spawns fresh from disk and works. Checking `md5sum` of the on-disk binary proves
nothing about the running zygote. After any host change that adds/changes
`add_to_linker` wiring: `run-hybrid-stack.sh --wandr-only` (restart the zygote).
Same family as [[feedback_shared_wit_rebuild_all_consumers]].

**Proven NON-bug:** a `wac plug`-composed guest importing a WIT *resource*
(`wandr:crypto/aead`'s `aead-key`) links and runs fine on wasmtime 45
(repro: reactor importing the resource + command app, `wac plug`, install,
`--run-once` → "resource-through-wac works"). The earlier belief that wac
compositions can't link imported resources was FALSE (superseded memory deleted);
`wandr:crypto/aead-oneshot` is therefore a convenience API, not a workaround —
measured equal to the resource path at SRTP packet sizes, so both stay.

Canonical host-resource pattern (matches wasmtime's own
`examples/resource-component/main.rs`): `bindgen!({ with: { "pkg:iface/iface.resource": HostType } })`
+ `ResourceTable` push/get/delete + impl the generated `HostXxx` trait + plain
`add_to_linker`. The `with` key uses a DOT before the resource name.
