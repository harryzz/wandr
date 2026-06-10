# wac-resource-import — imported WIT resources DO link through `wac plug`

Focused reproducer proving a **non-bug** (2026-06-10): a `wac plug`-composed guest
that imports a host interface containing a WIT `resource` links and runs fine on
wasmtime 45. Built to disprove the belief (from the wandr.signal bring-up) that
"a wac composite can't resolve an imported resource".

## The misleading error this decodes

```
component imports instance `wandr:crypto/aead@0.1.0`, but a matching implementation
was not found in the linker: instance export `aead-key` has the wrong type:
resource implementation is missing
```

`wasmtime/crates/wasmtime/src/runtime/component/matching.rs` (`definition()`,
`TypeDef::ComponentInstance` arm): when the linker has **no instance at all** under
that name, wasmtime still descends into the expected instance's exports with
`actual = None` — and the first export checked (the resource) names the error. So
this almost always means *"this host process never ran that interface's
add_to_linker"*, **not** a resource-specific problem.

In wandr the way to hit that with a correct host **binary** is the stale-zygote
trap: pushing a new wandr-host to disk does not change the already-running zygote's
image; arbiter-launched apps fork from the OLD zygote (missing the new linker
wiring) while `--run-once` spawns fresh from disk and works. Fix: restart the
zygote (`run-hybrid-stack.sh --wandr-only`) after any host linker change.

## Layout / run

- `plug/` — wasm32-wasip2 **reactor**: imports `wandr:crypto/aead@0.1.0` (the
  `aead-key` resource), exports `test:rescheck/probe.run() -> string` doing a
  create/seal/open roundtrip through the host resource.
- `app/`  — wasm32-wasip2 **command**: imports `test:rescheck/probe`, prints `run()`.

```sh
(cd plug && cargo build --target wasm32-wasip2 --release)
(cd app  && cargo build --target wasm32-wasip2 --release)
wac plug app/target/wasm32-wasip2/release/rescheck-app.wasm \
  --plug plug/target/wasm32-wasip2/release/rescheck_plug.wasm -o composed.wasm
# package composed.wasm as components/ui.wasm with a wasi:cli/command package.toml,
# wandr-host --install, then:
wandr-host --run-once test.rescheck
# => composite says: resource-through-wac works
```

Canonical host-side resource pattern (what wandr-host does; matches wasmtime's own
`examples/resource-component/main.rs`): `bindgen!({ with: { "wandr:crypto/aead.aead-key":
AeadKeyState } })` (note the **dot** before the resource name) + `ResourceTable`
push/get/delete + impl the generated `HostAeadKey` trait + plain `add_to_linker`.
