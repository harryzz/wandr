---
name: reference_redox_wandr
description: "Redox OS as wandr host — x86_64-VM plausible (gate = wasmtime-under-redoxer spike; skia → tiny-skia+parley pure-Rust backend idea), arm64 bare-metal = moonshot via open drivers (Qualcomm only, blobs unusable on non-Linux kernel), realistic path = Redox-under-AVF/pKVM with Linux keeping the blobs; memo = docs/redox-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

Discussion 2026-06-11, full memo: **`docs/redox-wandr-feasibility.md`**.
No spike run yet. Key conclusions:

- Guests are already OS-agnostic post wasi:canvas migration
  ([[project_wasi_canvas_migration]]); porting = host-side only.
- **x86_64 Redox VM**: winit + softbuffer both have Orbital backends →
  task-101 desktop-loop shape maps 1:1, CPU raster fits no-GPU Redox.
  Blockers ranked: (1) skia C++ cross-compile — escape hatch = second
  host backend on tiny-skia + parley serving the same wasi:canvas WIT
  (valuable beyond Redox: de-C++-ifies the host); (2) wasmtime not a
  supported platform — `signals_based_traps(false)` fallback if Redox
  signals too weak; (3) tokio/mio Redox support = murkiest dependency.
  The 80%-answer spike: run a hello-component under redoxer
  (`repros/redox-wasmtime-probe` someday).
- **arm64 phone**: vendor blobs are 3 classes — coprocessor firmware
  (kernel-agnostic, fine), userspace HAL blobs (bionic+Linux-ioctl,
  IMPOSSIBLE on a microkernel; libhybris doesn't transfer), open
  drivers (freedreno/ath10k/camss — Qualcomm's open stack is why
  postmarketOS works; Redox would port these as scheme daemons).
- **Realistic convergence**: Redox in a pKVM/AVF VM on a Pixel — Linux
  host keeps the blobs, Redox drives virtio; wandr's WIT service layer
  doesn't care whether a daemon speaks binder, a scheme, or virtio.
