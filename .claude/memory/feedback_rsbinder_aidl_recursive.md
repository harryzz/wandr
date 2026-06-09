---
name: rsbinder-aidl-recursive-parcelable-limitation
description: "rsbinder-aidl 0.7.0 emits Vec<Box<Self>> for recursive parcelables but doesn't impl SerializeArray/DeserializeArray for Box<T>, so the build won't compile"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ade59596-71ca-44d3-bc3e-26f4f4ba5671
---

rsbinder-aidl 0.7.0 cannot express recursive AIDL parcelables. When an
AIDL declares a field like `MyType[] next` inside `parcelable MyType {}`,
the codegen emits `Vec<Box<MyType>>` (the Box breaks the Rust type
cycle). But `Box<T>` doesn't implement `SerializeArray` or
`DeserializeArray`, so the generated `write_to_parcel` / `read_from_parcel`
fail to compile with:

    the trait `SerializeArray` is not implemented for `Box<MyType>`
    the trait `DeserializeArray` is not implemented for `Box<MyType>`

**Why:** Most notably blocks `android.content.AttributionSourceState`,
which has `AttributionSourceState[] next` for permission-delegation
chains. Task 21 hit this when trying to upgrade the empty-stub
AttributionSourceState to its real shape (to set `packageName` for the
AAudio policy lookup).

**How to apply:** When stubbing AOSP AIDL types under
`wandr-host/vendor/aidl-stubs/` for rsbinder-aidl 0.7.0:

1. If the upstream type is non-recursive: declare the real fields.
2. If recursive (`SelfType[] next` etc.): one of —
   - Drop `next` entirely (breaks wire format if service is strict).
   - Substitute `int[] next` (matches the on-the-wire shape of an
     empty array, may break if service inspects type descriptor).
   - Hand-write the parcel encoder (task 16's `@nullable` workaround
     pattern: bypass codegen, build/parse parcel bytes directly).
   - Upgrade rsbinder-aidl past 0.7.0 if a newer version supports
     recursive parcelables.

The empty-stub approach (`parcelable Foo;` with no body) remains
correct for services that auto-fill caller context from the binder
metadata (AAudio fills `pid`/`uid`; AttributionSourceState's
`packageName` defaults to null but doesn't block openStream once
SHARED mode + registerClient + stereo + PROT_RW are right).

Related: [[project_wasm_runtime]], task 21 appendix.
