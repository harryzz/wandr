// G2 diagnostic: does PointerOffset.offset { .of(&$0.field) } compute the correct
// byte offset for non-trivial fields, and does the fake `invalid` base materialize?
//
// For each field shape we report:
//   computed   = PointerOffset.offset { .of(&$0.field) }.byteOffset   (the suspect path)
//   truth      = MemoryLayout<Base>.offset(of: \.field)               (materialization-free)
//   materialized = did the inout `invalid` base move off invalidScenePointer()?
//                  (the precise detector — copied from upstream PointerOffsetCompatibilityTests)
//
// Verdict OK  iff computed == truth AND not materialized.
//
// Cases are crash-isolated: pass a case name as argv[1] and only that case runs, so a
// segfault in one shape does not mask the others. With no arg, lists the case names.
//
// PointerOffset here is a verbatim copy of the live Compute source
// (PointerOffset_Compute_verbatim.swift in this target).

// MARK: - Field-shape fixtures (smallest structs per shape)

struct ClosureHolder {            // a struct that merely *holds* a closure (like ModifiedContent)
    var tag: Int
    var fn: () -> Void
}

final class RefBox {}

struct TrivialBase {              // control: all-trivial
    var a: Int
    var b: Int
}

struct StructFieldBase {          // field is a struct-that-holds-a-closure
    var head: Int
    var holder: ClosureHolder
}

struct FuncFieldBase {            // field is a BARE function type — the prime G2 suspect
    var head: Int
    var body: (Int, Int) -> Int
}

struct ClassRefBase {             // field is a class reference
    var head: Int
    var r: RefBox
}

struct Closure1 { var call: (Int, Int) -> Int }   // single-field struct wrapping ONLY a closure
struct WrappedFuncBase {          // the proposed FIX: project the wrapper struct, not the bare fn
    var head: Int
    var fn: Closure1
}

// MARK: - Harness

func pad(_ s: String, _ n: Int) -> String {
    s.count >= n ? s : s + String(repeating: " ", count: n - s.count)
}

func check<Base, Member>(
    _ label: String,
    truth: Int?,
    _ body: @escaping (inout Base) -> PointerOffset<Base, Member>
) {
    let truthStr = truth.map(String.init) ?? "nil"
    print("  trying \(pad(label, 18)) truth=\(pad(truthStr, 6)) stride(Base)=\(MemoryLayout<Base>.stride) ...")
    fflush(stdout)

    var materialized = false
    let fake = UnsafeRawPointer(PointerOffset<Base, Member>.invalidScenePointer())
    let computed = PointerOffset<Base, Member>.offset { (invalid: inout Base) in
        let here = withUnsafeMutablePointer(to: &invalid) { UnsafeRawPointer($0) }
        if here != fake { materialized = true }
        return body(&invalid)
    }.byteOffset

    let ok = (truth == nil || computed == truth) && !materialized
    print("  [\(ok ? "OK " : "BAD")] \(pad(label, 18))"
        + " computed=\(pad(String(computed), 12))"
        + " truth=\(pad(truthStr, 6))"
        + " materialized=\(materialized ? "YES" : "no ")")
    fflush(stdout)
}

#if canImport(Glibc)
import Glibc
#elseif canImport(WASILibc)
import WASILibc
#elseif canImport(Darwin)
import Darwin
#endif

let cases: [String: () -> Void] = [
    "trivial": {
        check("TrivialBase.b", truth: MemoryLayout<TrivialBase>.offset(of: \.b)) {
            (invalid: inout TrivialBase) in .of(&invalid.b)
        }
    },
    "struct": {
        check("StructField.holder", truth: MemoryLayout<StructFieldBase>.offset(of: \.holder)) {
            (invalid: inout StructFieldBase) in .of(&invalid.holder)
        }
    },
    "func": {
        check("FuncField.body", truth: MemoryLayout<FuncFieldBase>.offset(of: \.body)) {
            (invalid: inout FuncFieldBase) in .of(&invalid.body)
        }
    },
    "class": {
        check("ClassRef.r", truth: MemoryLayout<ClassRefBase>.offset(of: \.r)) {
            (invalid: inout ClassRefBase) in .of(&invalid.r)
        }
    },
    "wrapped": {
        // THE FIX SHAPE: a single-field struct wrapping ONLY a closure (like the proposed
        // GestureBody2 wrapper). Projecting the WRAPPER (not the bare closure) must give the
        // correct offset and not materialize.
        check("Wrapped.fn(struct)", truth: MemoryLayout<WrappedFuncBase>.offset(of: \.fn)) {
            (invalid: inout WrappedFuncBase) in .of(&invalid.fn)
        }
    },
    // --- diagnostics ---
    "offsetof": {
        // Is MemoryLayout.offset(of:) available for each field shape?
        print("  offset(of: \\TrivialBase.b)      = \(String(describing: MemoryLayout<TrivialBase>.offset(of: \.b)))")
        print("  offset(of: \\StructFieldBase.holder) = \(String(describing: MemoryLayout<StructFieldBase>.offset(of: \.holder)))")
        print("  offset(of: \\FuncFieldBase.head) = \(String(describing: MemoryLayout<FuncFieldBase>.offset(of: \.head)))")
        print("  offset(of: \\FuncFieldBase.body) = \(String(describing: MemoryLayout<FuncFieldBase>.offset(of: \.body)))")
        print("  offset(of: \\ClassRefBase.r)     = \(String(describing: MemoryLayout<ClassRefBase>.offset(of: \.r)))")
        fflush(stdout)
    },
    "func_baseaddr": {
        // Pass the inout func-base into the closure but DO NOT project .body — return offset 0.
        // Crash here ⇒ passing inout non-trivial Base copies/retains the base value.
        print("  func_baseaddr: entering .offset ...") ; fflush(stdout)
        let po = PointerOffset<FuncFieldBase, Int>.offset { (invalid: inout FuncFieldBase) in
            let here = withUnsafeMutablePointer(to: &invalid) { UnsafeRawPointer($0) }
            print("  func_baseaddr: base addr=\(here) (no crash passing inout base)") ; fflush(stdout)
            return PointerOffset<FuncFieldBase, Int>(byteOffset: 0)
        }
        print("  func_baseaddr: done byteOffset=\(po.byteOffset)") ; fflush(stdout)
    },
    "func_fieldaddr": {
        // Project &invalid.body and take its address WITHOUT going through .of.
        // Crash here ⇒ projecting a bare-function field reads/copies the function value.
        print("  func_fieldaddr: entering .offset ...") ; fflush(stdout)
        let po = PointerOffset<FuncFieldBase, (Int, Int) -> Int>.offset { (invalid: inout FuncFieldBase) in
            let base = withUnsafeMutablePointer(to: &invalid) { UnsafeRawPointer($0) }
            print("  func_fieldaddr: base=\(base), projecting &invalid.body ...") ; fflush(stdout)
            let field = withUnsafeMutablePointer(to: &invalid.body) { UnsafeRawPointer($0) }
            print("  func_fieldaddr: field=\(field) offset=\(field - base)") ; fflush(stdout)
            return PointerOffset<FuncFieldBase, (Int, Int) -> Int>(byteOffset: field - base)
        }
        print("  func_fieldaddr: done byteOffset=\(po.byteOffset)") ; fflush(stdout)
    },
    "func_realbuf": {
        // DECISIVE: use a REAL zero-initialized Base buffer as the base (not the fake low
        // pointer), then project &buf.body. If offset == 8 (head:Int at 0, body next), then
        // reabstraction does NOT relocate the field — the crash was only the unmapped read,
        // and "use a real zeroed buffer" is the fix. If offset != 8, the field truly
        // materializes to a temp and the offset is unrecoverable via &field.
        print("  func_realbuf: allocating real zeroed FuncFieldBase ...") ; fflush(stdout)
        withUnsafeTemporaryAllocation(of: FuncFieldBase.self, capacity: 1) { buf in
            let raw = UnsafeMutableRawPointer(buf.baseAddress!)
            memset(raw, 0, MemoryLayout<FuncFieldBase>.stride)   // null fn-ptr + null ctx => safe to read
            let base = UnsafeRawPointer(raw)
            let field = withUnsafeMutablePointer(to: &buf[0].body) { UnsafeRawPointer($0) }
            print("  func_realbuf: base=\(base) field=\(field) offset=\(field - base) (expect 8)") ; fflush(stdout)
        }
    },
    "struct_realbuf_funcinner": {
        // Same real-buffer approach, projecting the INNER bare closure of a holder struct
        // (StructFieldBase.holder.fn) — confirms whether even a real buffer relocates a
        // directly-projected function l-value.
        print("  struct_realbuf_funcinner: ...") ; fflush(stdout)
        withUnsafeTemporaryAllocation(of: StructFieldBase.self, capacity: 1) { buf in
            let raw = UnsafeMutableRawPointer(buf.baseAddress!)
            memset(raw, 0, MemoryLayout<StructFieldBase>.stride)
            let base = UnsafeRawPointer(raw)
            let field = withUnsafeMutablePointer(to: &buf[0].holder.fn) { UnsafeRawPointer($0) }
            print("  struct_realbuf_funcinner: offset=\(field - base) (holder@8 + fn@8 => expect 16)") ; fflush(stdout)
        }
    },
]

print("=== PointerOffset G2 probe ===")
#if arch(wasm32)
print("target: wasm32-wasip1")
#else
print("target: native (\(MemoryLayout<Int>.size * 8)-bit)")
#endif
fflush(stdout)

let args = CommandLine.arguments
if args.count >= 2, let run = cases[args[1]] {
    run()
} else {
    // wasm runner can't pass argv easily — with no arg, run all in-process (first crash wins,
    // but the per-case "trying ..." line is flushed before each, so the log pinpoints it).
    for name in ["trivial", "struct", "func", "class"] {
        cases[name]!()
    }
}
