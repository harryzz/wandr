---
name: reference_wac_plug_imported_resource
description: "wac-plugged guest can't link an imported WIT resource (host interface) — \"resource implementation is missing\"; use a functions-only interface for the hot path"
metadata: 
  node_type: memory
  type: reference
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

A `wac plug`-composed guest (e.g. `apps/user/wandr.signal` = ui ◁ engine) **fails to
instantiate** when it imports a host interface that contains a `resource`, even though
the host `add_to_linker`s it correctly:

```
linker.instantiate failed: component imports instance `wandr:crypto/aead@0.1.0`,
but a matching implementation was not found in the linker:
instance export `aead-key` has the wrong type: resource implementation is missing
```

**Confirmed not the cause** (ruled out one by one): stale host binary (md5 matched
device), missing `add_to_linker` (present in BOTH `app_loader::instantiate` +
`instantiate_command`, committed 61e1ee7d), wac version (0.10.0 = latest), import
structure (leaf bench and composite have *byte-identical* `aead` instance types — both
`export "aead-key" (sub resource)` + `alias outer` the `types` import). The host's
resource registration works fine for a **leaf** `wasi:cli` guest via `--run-once`
(the `wandr.srtp.bench` + `wandr.crypto.test` guests link `wandr:crypto/aead`'s
resource and run). It is specifically the **wac-composite + imported-resource** combo
that wasmtime's component linker won't resolve (`matching.rs:83`, `actual = None`).
By contrast a *functions-only* imported interface composes through `wac plug` fine
(Signal already imports `wandr:audio-focus/focus` functions that way).

**FIX / pattern:** for a host interface a wac-plugged guest must import on a hot path,
provide a **functions-only (no resource) interface** — pass the key/state per call. We
added `wandr:crypto/aead-oneshot` (`seal`/`open` take `algo,key,nonce,aad,data`)
alongside the `aead` resource; the host keys AES-GCM per call (AES-256 schedule ≈
hundreds of ns, dwarfed by the cross-component call — device-measured one-shot ==
resource path: 3.0× audio / 8.4× video). The `aead-key` **resource** stays for
leaf/command guests; composed guests use `aead-oneshot`. See [[project_wandr_crypto_srtp_offload]].

Open question (not chased): whether this is a wac-plug hoisting bug or a wasmtime
composite-resource-import limitation. The functions-only workaround sidesteps it.
