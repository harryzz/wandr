#!/usr/bin/env python3
"""Post-regen patch for the JetBrains Kotlin wit-bindgen output (branch
`kotlin`, rev 6b9cb12): insert the LEADING
freeAllComponentModelReallocAllocatedMemory() into every generated IMPORT
wrapper, before its withScopedMemoryAllocator entry.

Why (the CLAUDE.md "required forever" rule): guest code can trip
cabi_realloc outside any binding — e.g. the FIRST identityHashCode()
(Any.toString / Any.hashCode) seeds Random.Default via wasiRandomGet,
which reallocs through the P1 adapter and leaves `reallocAllocator`
non-null. The next withScopedMemoryAllocator entry then throws "Can't
create new allocators while realloc-allocated memory is not freed".
The hand-written legacy bindings always opened with freeAll; the
generator only frees AFTER each call. Run this on every generated
binding file that contains import wrappers (NOT needed for the
Internal*.kt export stubs, which already lead with freeAll).

Usage: patch-kotlin-bindgen-freeall.py <file.kt> [...]
"""
import sys, re

MARK = "// [wandr post-regen] leading freeAll — see patch-kotlin-bindgen-freeall.py"
for path in sys.argv[1:]:
    with open(path) as f:
        s = f.read()
    if MARK in s:
        print(f"already patched: {path}")
        continue
    out = []
    for line in s.splitlines(keepends=True):
        stripped = line.lstrip()
        if stripped.startswith("kotlin.wasm.unsafe.withScopedMemoryAllocator {"):
            indent = line[: len(line) - len(stripped)]
            out.append(f"{indent}{MARK}\n")
            out.append(f"{indent}kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory()\n")
        out.append(line)
    with open(path, "w") as f:
        f.write("".join(out))
    print(f"patched: {path}")
