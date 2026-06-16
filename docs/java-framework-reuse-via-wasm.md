# Reusing Android's Java framework logic as wasm components

> Design note (2026-06-16). A "heretic" alternative to wandr's reimplement-in-Rust
> approach: instead of rewriting system_server's logic in the native arbiter,
> **compile the battle-tested pure-Java framework pieces to wasm components and
> deliver them to whoever imports them** (component-model dependency delivery,
> applied to the framework itself). Companion: `tasks/111-native-iradio-client.md`
> (the marquee proof-of-concept), `docs/wasm-component-language-support.md`
> (JVM→wasm toolchain status), `docs/architecture-runtime.md`.

## The thesis: reuse vs rewrite

wandr's default move is **rewrite**: the native `wandr-arbiter` re-implements the
AMS/WMS/PMS/Alarm/Notification slice of `system_server` in Rust. That's clean for
*policy* the arbiter must own anyway — but it's brutal for large, fiddly,
battle-tested *logic* with no native equivalent to lean on. Task 111 is the poster
child: cellular telephony's brain is **RILJ** (`RIL.java` + the telephony
services), pure Java, and rewriting its request serialization / response parsing /
call-SMS-data state machines in Rust is a multi-month undertaking that re-derives
edge cases Google already handles.

The framework's real value is its **logic**, not its process model. The WASM
Component Model is, literally, a "deliver capability X to whoever imports it"
machine — and wandr already does cross-component dependency wiring (tasks 36/39).
So: **take the pure-Java pieces, compile them to wasm components, and reuse them
verbatim** — no ART, isolated, on-demand. Reuse over rewrite where the logic is
big and the policy isn't ours.

## Vision vs achievable (the honest north star)

**The dream:** `aosp-tree | our-tools > android.wasm` — point the pipeline
(TeaVM-WASI + AIDL2WIT + the Looper-on-step-executor shim + native-method
substitution) at the whole AOSP framework and get "Android, wasm-based"
automatically, no hand-porting. It *feels* like it should compile, because every
obstacle has a mechanical transform.

**Why "whole tree, zero manual work" is the fantasy line** — four irreducible
reasons:

1. **The native floor can't be auto-generated.** AIDL2WIT mechanizes the *Binder*
   boundary, but the *non-Binder* JNI surface is enormous and bottoms out in real
   native subsystems (skia, EGL, AudioFlinger, ICU text shaping, BoringSSL crypto,
   codecs, bionic). A tool can transpile the Java *calling* `nativeDrawText`; it
   cannot conjure the shaper behind it. Thousands of native methods need real
   implementations, not a transform.
2. **There are no component boundaries in the source.** `system_server` is one
   process of shared static singletons + ServiceManager + the resource/asset
   system + services reaching into each other. Components need *boundaries with
   WITs*, which don't exist in AOSP — you have to **decide** them. Design, not a
   pass.
3. **Runtime-model mismatch.** ART is multi-threaded, shared-heap, JIT; wasm
   components are single-threaded, isolated, WasmGC. Much framework code assumes
   threads + cross-process-mutable statics.
4. **TeaVM coverage + scale.** Reflection, dynamic class loading, `Unsafe`,
   ART-isms → per-class walls; and all-of-AOSP→wasm is a colossal artifact with no
   benefit on most paths.

> (The *other* way to get "whole tree, no manual work" is the un-elegant one:
> compile ART + bionic + framework as native → wasm and run **Android-in-a-wasm-VM**
> — emulation, not components. Huge, slow, zero component-model wins. The
> transpile-to-components path here is the elegant version; it trades "automatic"
> for "compounding.")

**The achievable shape — a compounding spectrum, not a button:** don't lift *all*
of AOSP — lift the **Java logic layer**, and run it on wandr's **native floor**,
which wandr is already building piece by piece (skia/EGL, AudioFlinger client,
sensors, input, HAL shims, the arbiter). The wasm-Java imports those capabilities
via WIT. **The "native floor I can't auto-generate" *is wandr itself*.** Concretely:

- A **liftability pipeline** analyzes each class's **native + transitive closure**,
  auto-lifts the ones whose surface is already shimmed, and **flags the rest** as
  "needs a host shim" or "needs a boundary decision."
- The auto-lifted **green-list grows every time wandr implements another native
  shim or wires another WIT** — work wandr does for its own sake anyway. So the
  "no manual work" claim is true *at the easy end and expanding*; the manual
  frontier is exactly the native floor + boundary design.

So the honest tagline is **not** "AOSP → one button → Android-wasm." It is: *"an
automated lifter for the shallow-native Java layer, riding wandr's native floor,
with a frontier that shrinks as wandr grows."* RILJ (telephony) is the first
non-trivial proof; each capability wandr adds widens what the lifter gets for free.

## The three obstacles — and why two are already solved

Lifting a `system_server` class to wasm faces three entanglements. The scorecard:

| Obstacle | Status | How |
|---|---|---|
| **Binder (AIDL stubs)** | ✅ solved | **AIDL2WIT** — generate a WIT from the `.aidl`; substitute the AIDL proxy's native `transact` with the WIT import. The wasm-compiled service "talks binder," but it's really calling a wandr WIT the host fulfills (rsbinder → real HAL, or the arbiter). Mechanical, same `.aidl` source. |
| **Handler/Looper** | ✅ tractable | `Looper`/`Handler`/`Message`/`MessageQueue` are **almost entirely pure Java**; only `MessageQueue`'s ~3–4 native methods (`nativeInit`/`nativePollOnce`/`nativeWake`/`nativeDestroy`) are native, and they exist only to block a thread on epoll/eventfd. In a **single-threaded wasm guest** you don't want a blocking poll — replace them with a **cooperative queue pumped by wandr's frame-stepped reactor** (`[[project_wandr_step_executor]]`). `nativePollOnce` → "next due message or yield to host"; delayed posts → the host scheduler/frame-pacing. The rest of Looper compiles untouched. |
| **JNI / native methods** | ⚠️ the real constraint, but **bounded per class** | wasm has no JNI; every `native` method needs a host implementation. TeaVM (and any Java→wasm) *forces* you to supply one (native-method substitution) — that's the hook. The obstacle is **proportional to a class's native surface**, so it becomes a **class-selection criterion**, not a wall. |

### Why the single-threaded constraint helps here
The usual wasip2 pain (no ambient threads; Kotlin/Wasm + TeaVM-WASI are
single-threaded — `[[feedback_wasi_threading]]`) is an **advantage** for this idea:
a Looper on a HandlerThread collapses to a cooperative pump with no real
concurrency to emulate. wandr already built that pump (the step-executor) for
async survival across export calls — a Looper is the same primitive with Java
message semantics on top.

## The class-selection rule

Lift classes with a **shallow, enumerable native surface**. The idea dies on
classes that touch deep native subsystems (ICU text, BoringSSL crypto JNI,
graphics, codecs) — those drag in too much. So:

1. Prefer **self-contained pure-Java libraries** (protocol codecs, parsers, format
   handlers, PDU/ASN.1) over whole services.
2. For services, pick ones whose non-Binder native surface is ~a handful of host
   functions (`SystemProperties` getprop, `Slog` logging — wandr has `host-log`,
   small `Parcel` marshalling).
3. Watch **transitive** imports — one innocent `import` can pull in ICU. Audit the
   native closure, not just the entry class.

## Marquee PoC: RILJ as a wasm component (alternative to task 111)

RILJ sits exactly on the liftable line: **mostly Java protocol logic** (the worst
thing to rewrite in Rust) bound to **Binder + a HandlerThread** (both now solved).
The PoC:

- **Binder** → AIDL2WIT on `android.hardware.radio` (+ the framework telephony
  AIDLs). On the Pixel 2 XL radio is HIDL `@1.4`, so the WIT is fulfilled by the
  C++ HIDL shim from task 111; on a Pixel 6/9a it's AIDL → rsbinder directly.
- **HandlerThread** → the step-executor cooperative pump.
- **Residual JNI** → ~3 host shims (`SystemProperties`, `Slog`, minor `Parcel`).

Result: task 111 flips from "**reimplement RILJ in Rust**" to "**compile RILJ to
wasm + shim its two host dependencies**" — far less code, and you inherit Google's
telephony state machine and its edge cases. Same trade applies to other
big-logic/small-policy services (package/manifest parsing, lock-settings,
SMS PDU).

## The gap: the toolchain (do this first)

There is **no maintained Java→Component-Model toolchain** today
(`docs/wasm-component-language-support.md`): the TeaVM-WASI forks are dormant
(golem 2023 / fermyon 2022) and upstream wit-bindgen removed the `teavm-java`
generator. So step zero is **reviving a Java→component path**:

- Upstream **TeaVM (`konsoletyper`) is active** and has both a linear-memory and a
  WasmGC backend → the compiler exists; the *component* wiring is what lapsed.
- Cheapest revival: TeaVM **core module** (linear-memory) → wandr's existing
  **P1→P2 adapter** (the same trick used for Kotlin/Wasm), + a revived/ported
  `teavm-java` binding generator (or hand-written canonical-ABI bindings, as wandr
  already does for skiko).
- wandr's **host is pre-positioned**: wasmtime here already runs WasmGC +
  exceptions + tail-calls (WebAssembly 3.0), which is what a JVM-family guest
  needs. The gap is entirely guest-side.

## Honest unknowns / risks

- **TeaVM Java coverage** — reflection, dynamic class loading, some generics edge
  cases, ART-specific behaviors; the target classes must stay within TeaVM's
  supported subset.
- **classlib + framework size** — pulling `android.*` + libcore for a service is
  large; on-demand load + component isolation mitigate, but startup/size need
  measuring.
- **Native-surface creep** — transitive deps can balloon the JNI shim count;
  enforce the selection rule with a native-closure audit.
- **Maintenance** — reviving the toolchain is wandr's own undertaking, not an
  off-the-shelf dependency.

## When to use which

- **Rewrite in the arbiter (Rust):** policy wandr must own (focus, roles, window
  layout, power, audio routing) — small, ours, performance-sensitive.
- **Reuse via wasm component (this note):** large, battle-tested, fiddly Java
  *logic* that isn't policy — telephony (RILJ), parsers, codecs, protocol stacks —
  where rewriting re-derives edge cases for no architectural gain.

The two are complementary: the arbiter stays the native coordinator; reused Java
components become the heavy *logic* libraries it (or guests) import via WIT.
