#!/usr/bin/env python3
"""
Patch libwasm_android_host.so (skia-safe 0.93.1 build) to neutralize
bundled Skia bionic stubs that crash on LineageOS 22.2.

Root cause: Skia prebuilts bundle a full bionic copy including getauxval().
The bundled getauxval() calls __libc_shared_globals() which has a NULL auxv
pointer in the bundled copy, causing SIGSEGV at address 0x0.

Patches applied (build with bionic_compat.rs / __wrap_pthread_create):
  1. init_have_lse_atomics  @ 0x160934c  → ret            (.init_array ctor 0)
  2. __init_cpu_features    @ 0x16095fc  → ret            (.init_array ctor 1)
  3. getauxval              @ 0x166fb90  → smart stub:
  4. pthread_create[0]      @ 0x1665030  → B __wrap_pthread_create
     (LLD --wrap does not intercept LOCAL HIDDEN definitions, so we patch
      the entry point directly to redirect to our dlsym-based wrapper)
       AT_PAGESZ (6)  → 4096   (pthread_create needs this for stack sizing)
       AT_CLKTCK (17) → 100    (clock ticks per second)
       everything else → 0     (disables LSE/hwcap probing; AT_RANDOM=0 ok)

Patches 1+2 stop the crash from .init_array constructors.
Patch 3 stops Scudo's primary allocator from computing a garbage RegionBeg
(caused by AT_RANDOM returning NULL/bad value from the broken bundled stub).
Patch 4 redirects the bundled pthread_create entry point to __wrap_pthread_create
(defined in bionic_compat.rs) which uses dlsym to call the system version,
avoiding the __init_tcb crash on LineageOS 22.2.
"""
import struct, sys

def pack(*vals):
    return struct.pack(f'<{len(vals)}I', *vals)

RET = 0xd65f03c0

def vaddr_to_offset(data, virt_addr):
    e_phoff     = struct.unpack_from('<Q', data, 0x20)[0]
    e_phentsize = struct.unpack_from('<H', data, 0x36)[0]
    e_phnum     = struct.unpack_from('<H', data, 0x38)[0]
    for i in range(e_phnum):
        o        = e_phoff + i * e_phentsize
        p_type   = struct.unpack_from('<I', data, o)[0]
        p_offset = struct.unpack_from('<Q', data, o + 0x08)[0]
        p_vaddr  = struct.unpack_from('<Q', data, o + 0x10)[0]
        p_filesz = struct.unpack_from('<Q', data, o + 0x20)[0]
        if p_type == 1 and p_vaddr <= virt_addr < p_vaddr + p_filesz:
            return p_offset + (virt_addr - p_vaddr)
    return None

# ── getauxval smart stub (10 instructions = 40 bytes) ────────────────────────
# x0 = type (unsigned long)
#
#   cmp  x0, #6           ; AT_PAGESZ?
#   b.eq ret_pgsz         ; → 4096
#   cmp  x0, #17          ; AT_CLKTCK?
#   b.eq ret_clktck       ; → 100
#   movz x0, #0           ; default: 0
#   ret
# ret_pgsz:
#   movz x0, #0x1000      ; 4096
#   ret
# ret_clktck:
#   movz x0, #100         ; 100
#   ret
#
# b.eq at offset 4: target = offset 24 (skip 5 instructions = +20 bytes from this instr)
#   imm19 = 20/4 = 5  → b.eq: 0x54000040 | (5 << 5) = 0x540000a0? wait...
#   b.cond: bits[31:24]=0x54, bits[23:5]=imm19, bit[4]=0, bits[3:0]=cond
#   EQ=0000, imm19=5: 0b 0101_0100  000_0000_0000_0000_0101  0  0000
#   = 0x540000a0
#
# b.eq at offset 12: target = offset 32 (skip 5 instructions from offset 12 = +20)
#   From offset 12, target = 32, delta = 20, imm19 = 5 → 0x540000a0
#
GETAUXVAL_PATCH = pack(
    0xf100181f,   # cmp  x0, #6
    0x540000a0,   # b.eq pc+20  (→ ret_pgsz at offset 24)
    0xf100441f,   # cmp  x0, #17   (sub xzr, x0, #17)
    0x540000a0,   # b.eq pc+20  (→ ret_clktck at offset 32)
    0xd2800000,   # movz x0, #0
    RET,          # ret
    0xd2820000,   # movz x0, #0x1000  (4096)
    RET,          # ret
    0xd2800c80,   # movz x0, #100  (0x64)
    RET,          # ret
)

# B __wrap_pthread_create — compute from nm-derived addresses
PTHREAD_CREATE     = 0x166b500
WRAP_PTHREAD_CREATE = 0x75d9e8
delta = WRAP_PTHREAD_CREATE - PTHREAD_CREATE
assert delta % 4 == 0
imm26 = (delta // 4) & 0x3FFFFFF   # 26-bit two's complement
B_WRAP_PTHREAD = pack(0x14000000 | imm26)

PATCHES = [
    (0x160f81c, pack(RET),         "init_have_lse_atomics → ret"),
    (0x160facc, pack(RET),         "__init_cpu_features   → ret"),
    (0x1676060, GETAUXVAL_PATCH,   "getauxval → smart stub (pgsz=4096, clktck=100, else 0)"),
    (PTHREAD_CREATE, B_WRAP_PTHREAD, "pthread_create[0] → B __wrap_pthread_create"),
]

if len(sys.argv) != 2:
    print(f"Usage: {sys.argv[0]} <path-to-libwasm_android_host.so>")
    sys.exit(1)

so_path = sys.argv[1]
with open(so_path, 'rb') as f:
    data = f.read()

with open(so_path, 'r+b') as f:
    for vaddr, patch, desc in PATCHES:
        off = vaddr_to_offset(data, vaddr)
        if off is None:
            print(f"ERROR: could not locate 0x{vaddr:x} ({desc})")
            sys.exit(1)
        f.seek(off)
        orig = f.read(len(patch))
        f.seek(off)
        f.write(patch)
        print(f"  0x{vaddr:x}  (file+0x{off:x})")
        print(f"    orig: {orig.hex()}")
        print(f"    new:  {patch.hex()}")
        print(f"    [{desc}]")

print("Done.")
