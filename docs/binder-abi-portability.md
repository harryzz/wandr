# Binder ABI portability — generating rsbinder bindings across Android versions & devices

> Status: **DESIGN / DEFERRED tooling.** Not built. Captures the analysis behind
> the roadmap item. The reference device (Pixel 2 XL, Android 15) is hand-ported
> today; this doc is the plan for "works on any AOSP-aligned device" when the
> project goes public.
>
> Scope: **Boundary B only** (host ↔ native HAL/daemon via binder, see
> `post-art-roadmap.md` §3). This is NOT the guest WIT boundary and NOT `aidl2wit`
> (§4) — it is the host's *binder ABI* layer.

## 1. The problem

The wandr host replaces ART, so it must talk to the native daemons/HALs that
survive `--no-art` (wificond, `ISupplicant`/`IWifi`, `netd`, `IDnsResolver`,
SurfaceFlinger, AudioFlinger, sensors, keymint, …) directly over `/dev/binder`,
using **rsbinder** (pure-Rust binder; chosen over Android's in-tree `libbinder_rs`
to avoid linking the C++ `libbinder`).

Every such interface has a **binder ABI**: transaction codes (which integer maps
to which method) and parcel wire layouts (the exact byte sequence of each
argument/return). To call a service correctly, our generated Rust bindings must
match that ABI **exactly**. The ABI is fixed by the AIDL/C++ the *device* was
built from — so it varies by **Android version** (15 → 16 → 17 …) and, for vendor
HALs, by **device**.

For one phone this is a bounded, one-time port. For a public, multi-device,
multi-version project it is the central scaling problem — the difference between
"runs on a Pixel 2 XL" and "runs on any rooted AOSP-aligned device." This tooling
is, in effect, the project's moat.

## 2. What we do today (manual)

Per interface, by hand (see the task-90 wificond scan work as the worked example):

1. Find the device's matching AOSP source (clone the right `android15-release`
   subtree by build fingerprint).
2. Vendor a **trimmed** `.aidl`: keep only the methods we call, but **preserve
   method order** so transaction codes (`FIRST_CALL_TRANSACTION + decl index`)
   stay correct; stub the rest as `void reservedN()`.
3. For `cpp_header` (unstructured) parcelables — whose layout lives only in a
   hand-written C++ class — **read the C++ `writeToParcel`/`readFromParcel`** and
   re-declare a structured AIDL with fields in that exact wire order (or
   raw-transact and hand-frame the parcel).
4. Work around rsbinder codegen gaps (e.g. `@JavaOnlyImmutable` → empty struct;
   `cpp_header` → no marshalling; nullable-parcelable presence markers).

This works but is slow and brittle: every Android version or divergent device
can invalidate steps 2–3. The task-90 scan cost ~a day mostly on (3)+(4) — a
missing leading `int32(1)` presence marker on a parcelable arg, and a `cpp_header`
layout that had to be read from `scan_result.cpp`.

## 3. What actually varies — the 80/20 split

The surface is mostly well-behaved; the pain is concentrated.

| Class | Behaviour across versions/devices | Automatable? |
|---|---|---|
| **Stable AIDL** (most HALs, `netd`, `IDnsResolver`, keymint, SF/gui) | Versioned + content-**hashed**; methods are *appended* (old transaction codes stay valid); AOSP-uniform across devices. Runtime `getInterfaceVersion()`/`getInterfaceHash()` identify the exact revision. | **Yes — the easy ~80%.** |
| **Unstable AIDL** (wificond `nl80211`, system-internal) | No stability guarantee — methods may reorder/change between versions. No runtime version/hash. | Hard — needs the matching source per version. |
| **`cpp_header` parcelables** (`NativeScanResult`, `SingleScanSettings`, …) | Layout lives only in C++; a new field in 16/17 silently changes the wire format. | Hard — see §5. |
| **Vendor HAL *implementations*** | Implement the same AOSP-defined AIDL *contract*; wire ABI = the AIDL, so uniform at the binder level. Heavily-forked ROMs may patch interfaces. | Mostly yes (the contract is AOSP); forks need overlays. |

The stable-AIDL majority barely moves and is fully automatable. The cost is
concentrated in unstable AIDL + `cpp_header`, which is exactly where a generator
must focus.

## 4. The leverage: Google's AIDL toolchain

We are not starting from nothing. AOSP's `aidl` compiler emits
`--lang={cpp,java,ndk,rust}`. The **`--lang=rust`** backend is **ABI-correct by
construction** — same frontend that produced the device's C++/Java, so codes,
parcel layouts, presence markers, and annotation handling are all done the
official way. Critically:

- **rsbinder's API deliberately mirrors `libbinder_rs`** (`Strong`, `Status`, the
  `Parcel` API). `rsbinder-aidl` is essentially a *port* of the official Rust
  backend — so the two are close cousins, and our codegen gaps are exactly where
  the port **lags** the original.

Two ways to exploit this:

1. **Use `aidl --lang=rust` directly** — rejected: its output targets
   `libbinder_rs` → links the C++ `libbinder`, defeating rsbinder's pure-Rust
   premise.
2. **Retarget the official backend (or its `--dumpapi` IR) to rsbinder** — emit
   rsbinder-flavoured Rust from Google's frontend; inherit correct
   annotation/ABI handling, swap only the runtime calls (`binder::` →
   `rsbinder::`). This is the principled route, and it subsumes the
   "extract the AST/IR and reconstruct" idea.

What the official toolchain does **not** solve: `cpp_header` parcelables (opaque
to *every* backend — their layout is hand-written C++), and "wrong AIDL version"
(you still need the right source; the compiler can't invent it). Transaction
codes are *not* a compiler difference — rsbinder and `aidl` compute them
identically from the same AIDL; our mismatches were always a wrong/trimmed AIDL.

## 5. The hard core — `cpp_header` parcelable marshalling

The layout exists only as executable C++ (`parcel->writeInt32(...)` sequences).
Two ways to capture it:

- **Flavor A — read it (static clang AST):** walk the `writeToParcel` AST and
  emit equivalent Rust. Easy for flat field lists, but real ones have **loops**
  (`NativeScanResult.radio_chain_infos`) and **conditionals**
  (`SingleScanSettings` `if (!freqs.empty())`) — handling those means writing a
  mini symbolic-executor.
- **Flavor B — run it (compile the real C++):** compile the actual
  `writeToParcel`/`readFromParcel` + a **minimal `Parcel` shim** (~100 lines: a
  byte buffer with `writeInt32`/`writeByteVector`, since plain-data parcelables
  carry no binders/FDs) into a tiny **native static lib for aarch64-android** and
  **FFI-call it from Rust**. It handles every loop/conditional perfectly because
  it *is* the code — and it **auto-adapts across Android versions** (recompile
  the version's C++; no re-port). Precedent exists: the host already links C++
  (`libsf_surface`, the framework-shim).

**Wasm note:** C++→wasm is mature and wandr runs wasm, so Flavor B *could* be
wasm. But for host-side, trusted, data-only marshalling the sandbox buys nothing;
a native static lib + FFI is simpler. Wasm would only matter if the marshalling
had to run sandboxed/in-guest, which it does not. **Flavor B (native) is the
recommended automation** — it reuses Google's exact code with zero
reverse-engineering and is the only approach that scales cleanly per version.

## 6. The generator (proposed pipeline)

Keyed on the target device itself:

1. **Runtime introspection (needs a live device):**
   - Confirm the interface descriptor (`INTERFACE_TRANSACTION`).
   - For **stable** AIDL: query `getInterfaceVersion()` / `getInterfaceHash()` →
     exact revision.
   - For **unstable** AIDL: a **transaction-code prober** — transact each code and
     classify the reply (`UNKNOWN_TRANSACTION` vs error vs success) to map which
     codes exist. **Caveat:** safe for *presence* detection (descriptor + version
     are read-only by design), but invoking unknown methods can have
     **side-effects** (`tearDownInterface`, `disconnect`) — probe read-only-ish,
     never hammer mutating codes.
2. **AOSP-source-by-fingerprint (offline):** from `ro.build.*` + the hashes,
   resolve the exact AOSP `.aidl` + (for `cpp_header`) `.cpp` revision. AOSP is
   public and fully version-tagged.
3. **Codegen (offline):** generate rsbinder bindings via the retargeted official
   backend (§4) — or, interim, `rsbinder-aidl` with the annotation fixes
   **upstreamed** (`@JavaOnlyImmutable` etc.). Auto-trim to called methods while
   preserving codes.
4. **`cpp_header` (offline):** compile the version's real C++ marshalling +
   minimal Parcel shim → native lib → FFI (Flavor B).
5. **Per-device quirks overlay:** a small hand-maintained layer for genuinely
   divergent (forked-ROM) interfaces.

Online steps = #1; everything else is offline source processing (needs the
device's fingerprint once).

## 7. Honest limits

- Scales cleanly across **AOSP-aligned** targets (Pixels, GSI, AOSP-derived ROMs)
  because AOSP is the ground truth.
- **Heavily-forked vendor ROMs** (OneUI, HyperOS) patch HALs/services and need
  device-specific overlays, or won't be fully covered. The credible public claim
  is *"any rooted, reasonably-AOSP-aligned device,"* not literally every phone —
  state this up front rather than overpromising.
- The cleanest long-term home for the codegen fixes is **upstream `rsbinder-aidl`**
  (actively developed), not a permanent private fork.

## 8. Recommendation / phasing

1. **Now:** hand-port the reference device (done). The `cpp_header` parcelables in
   scope are few; manual is cheaper than the tool *today*.
2. **Before going public:** build the generator (§6). That is exactly when the
   "what about Android 16 / my phone?" questions arrive, and the answer being
   "run the generator against your device" rather than "wait for a hand-port" is
   what makes wandr read as a platform, not a one-device demo.
3. **Incrementally:** cover the well-behaved stable-AIDL 80% first (hash →
   source → codegen); add the transaction-code prober + Flavor-B `cpp_header`
   for the hard 20%; keep a thin per-device quirks overlay.

See `post-art-roadmap.md` §3 (Boundary B), §4 (why not `aidl2wit`), and §14.
</content>
