---
name: bionic_compat NDK linker gotchas
description: Critical fixes for building against NDK on LineageOS — libdl.a stub, sysroot paths, ELF scanner load-bias bug
type: feedback
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
**Use the versioned API sysroot dir (e.g. `aarch64-linux-android/35/`), not the base dir.**

The base sysroot `aarch64-linux-android/` has `libdl.a` (stub returning 0) but NO `libdl.so`. The versioned dir `aarch64-linux-android/35/` has `libdl.so` (proper import stub). If you point `cargo:rustc-link-search` at the base dir, `dlsym` gets inlined as a zero-returning stub — `dlsym(RTLD_DEFAULT, "malloc")` always returns NULL.

**Why:** NDK intentionally omits `libdl.so` from the base dir; it lives in versioned subdirs. Without it, `-ldl` picks up the static stub from `libdl.a`.

**How to apply:** `build.rs` must use `sysroot/usr/lib/aarch64-linux-android/35` (or the appropriate API level) as the `rustc-link-search` path.

---

**Use `dlsym(RTLD_DEFAULT, "pthread_create")` instead of ELF section-header scanning.**

The old ELF scanner used the `r-xp` segment base as load bias, but the first `r--p` segment (file offset 0) is the actual load bias. On Android, the `r-xp` segment starts at a non-zero file offset (e.g. 0x10a000), so `base_rx + st_value` overshoots by that amount, landing in a wrong function (e.g. `inet_pton`).

**Why:** ELF shared libraries have their first LOAD segment at `p_vaddr=0`, but the text segment is at a higher offset. Using the rx-segment base gives the wrong load bias.

**How to apply:** Call `dlsym(RTLD_DEFAULT, "pthread_create")` directly — it finds the system libc.so version because `--wrap=pthread_create` renames ours to `__wrap_pthread_create` in the dynamic symbol table.

---

**Bootstrap arena needed for `dlsym` → malloc reentrancy.**

`dlsym(RTLD_DEFAULT, "malloc")` itself calls malloc internally. The first call to `__wrap_malloc` must handle this reentrant case with a static BSS bootstrap arena (128KB), not spin on REAL_MALLOC. Store `REAL_MALLOC=INIT=1` as a sentinel during lookup; reentrant calls see INIT and use the bootstrap arena.

**Why:** Without the bootstrap arena, the first `__wrap_malloc` → dlsym → malloc → `__wrap_malloc` loop spins forever (deadlock).
