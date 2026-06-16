# Guest-language support for WASI components (language neutrality)

> Survey of which source languages can produce **WASI Preview 2 / Component
> Model** components today — the contract wandr uses for every guest. Captured
> 2026-05-31 to back the "language neutrality + multi-language coexistence"
> demonstration. Moves fast; re-check the sources before quoting.

## Why this matters for wandr

wandr's entire host↔guest boundary is **WIT + the Component Model** (see
`docs/architecture-host-guest-boundary.md`). The host exposes interfaces
(`my:skiko-gfx/*` — canvas, paragraph, display, keyboard, …) and links them into
any guest via `SkikoUi::add_to_linker`; a guest only has to **export the
`renderer` world** and import what it needs. Nothing in that contract is
Kotlin-specific. So **any language that can emit a component for our world is a
candidate guest**, and components from different languages **coexist** behind the
same WIT — exactly how the Rust chrome guests (`wandr.launcher`, `wandr.statusbar`,
`wandr.taskbar`, `wandr.dioxus.demo`) already run alongside the Kotlin/Compose
`wandr-app` and `wandr.ime.keyboard` today, and how cross-app deps compose via
`link.wac` / the host's component-type walk (`wire_dep_into_linker`).

**The bar** for a real wandr guest is stricter than "hello world": produce a
component for a **custom world with exports** (our `skiko-ui` / `renderer`), not
just the stock `wasi:cli/command`. The table below is graded on that bar.

## Status at a glance

| Language | Produces WASI-P2 components? | Toolchain | Custom-world **exports**? | P1→P2 adapter | Maturity |
|---|---|---|---|---|---|
| **Rust** | ✅ first-class (reference impl) | `cargo component` / `wit-bindgen` + `wasm-tools` | ✅ native | not needed (native wasip2) | Production |
| **C# / .NET** | ✅ yes — best managed-language story | `componentize-dotnet` (one NuGet: NativeAOT-LLVM + wit-bindgen + wasm-tools + WASI SDK) | ✅ full WIT import/export | hidden by toolchain | Preview (~0.7), usable |
| **Go (TinyGo)** | ✅ yes | `tinygo build -target=wasip2 --wit-package … --wit-world …` (shells out to wasm-tools embed+new) | ⚠️ yes, with friction¹ | hidden by toolchain | v0.34+ (use ≥0.39) |
| **Zig** | 🟡 manual only | wit-bindgen **C** generator + Zig C-interop → wasm32-wasip1 → `wasm-tools component new` w/ adapter | ✅ but hand-wired | **explicit** (std is P1) | DIY |
| **Java (JVM)** | 🔴 dormant | **teavm-wasi** forks (bytecode → wasm + CM bindings) | ✅ in the forks | hidden by fork | **Forks dormant (golem 2023 / fermyon 2022); wit-bindgen `teavm-java` gen removed** — see 2026 update below |
| **Kotlin/Wasm** *(what wandr uses)* | 🟡 via hand-rolled pipeline | KGP `compileProductionExecutableKotlinWasmWasi` → `wasm-tools component embed` → `component new --adapt` | ✅ (our `skiko-ui`) | **explicit** (wandr-fork reactor adapter, KT-86415) | Shipping in wandr |

¹ TinyGo's `wasip2` target is hardwired to `wasi:cli/command`; a non-CLI world
needs an explicit `--wit-world` plus `include wasi:cli/imports@0.2.0`. Cleaner
arbitrary-world support is tracked in tinygo-org/tinygo#4843.

## Per-language notes

- **Rust** — the reference. Native `wasm32-wasip2`, no adapter, cleanest WIT
  ergonomics. This is why wandr's light chrome guests are Rust.
- **C# / .NET** — `componentize-dotnet` gives a Rust/TinyGo-comparable
  experience from a single NuGet reference: AOT (NativeAOT-LLVM), real WIT
  import/export, custom worlds. Needs .NET 9/10. Still preview but functional —
  the **closest drop-in to a Compose-style managed guest**, and the strongest
  non-Rust option if we ever want a second high-level guest language.
- **Go (TinyGo)** — mature enough; the toolchain hides the embed+componentize
  steps. Stock `go` (`GOOS=wasip1`) is **Preview 1 only** — no native P2
  component; you must route through TinyGo (or wasm-tools by hand).
- **Zig** — no native Component Model and **no Zig generator in wit-bindgen**.
  You use the C generator + Zig's C interop, build to wasip1, then adapt to P2 —
  i.e. essentially the same hand-rolled route wandr uses for Kotlin. Viable,
  lightest runtime, but all plumbing is manual.
- **Java** — no official support; Fermyon's `teavm-wasi` fork can emit
  components but isn't merged upstream. **Key gotcha for the WasmGC question:**
  TeaVM has *two separate* wasm backends — a **WasmGC** backend (browser-targeted,
  talks to JS/DOM) and the **linear-memory** backend that the Fermyon **WASI**
  fork builds on. They are **not combined**: there is **no Java → WasmGC + WASI**
  toolchain today. So a Java guest in a WASI host (like wandr) is **linear-memory**
  TeaVM + the *stock* P1→P2 adapter — not WasmGC. The only JVM-family language
  that ships **WasmGC + WASI together is Kotlin/Wasm** (what wandr uses). Important
  because Java and Kotlin "both make JVM bytecode" does **not** imply a shared
  wasm path: Kotlin/Wasm compiles Kotlin *source* → WasmGC directly (never via
  bytecode), while the Java route goes bytecode → TeaVM → linear-memory wasm.
  **2026 UPDATE — the Java→Component-Model path has stalled/regressed, not
  advanced:** the CM-capable forks are dormant (**golemcloud/teavm-wasi** last
  commit Sep 2023, **fermyon/teavm-wasi** Dec 2022), and **upstream wit-bindgen
  removed the `teavm-java` generator** ("unmaintained for a long time and never at
  feature parity"). So the one concrete Java→CM toolchain — which *could* export
  custom WIT (`wit-bindgen guest teavm-java --export …`) — is gone from the
  official tooling. Meanwhile **upstream TeaVM (`konsoletyper`) is very active**
  (commit 2026-06-15) and gained a WasmGC backend, but it targets **core wasm /
  WASI, not the Component Model**. Net: **no actively-maintained Java→CM toolchain
  exists today.** The cheapest revival path for a Java guest on wandr is **TeaVM
  core module + the same P1→P2 adapter wandr uses for Kotlin** (reuse the trick;
  linear-memory, not WasmGC) — wandr's undertaking, not an off-the-shelf option.
  GraalVM is not a candidate (it runs wasm *in* the JVM; native-image doesn't emit
  wasm components).
- **Kotlin/Wasm** (context) — our path: WasmGC output + a P1→P2 **reactor
  adapter** (wandr fork, the KT-86415 State-pin), hand-written `@WasmImport`/
  `@WasmExport` bindings. No native P2 target yet (watch KT-64568). Listed here
  so the survey shows where our own stack sits relative to the others.

## Interpreted / dynamic languages (different model)

Compiled languages turn *your code* into wasm. Interpreters instead **embed the
whole language runtime inside the component** and snapshot your script into it
(usually via **Wizer** pre-initialization). So a "Python component" is CPython +
your `.py` baked in; a "JS component" is a JS engine + your module. That has two
consequences for wandr: the per-component footprint is **megabytes of VM** (heavy
against our ~180 MB/app working-set finding — two Python guests = two CPython
copies unless shared), and execution is **interpreter-speed** — fine for
logic/plugin/automation guests, poor for a 60 fps render hot path.

| Language | WASI-P2 components? | Tooling | Notes |
|---|---|---|---|
| **JavaScript / TS** | ✅ yes, mature | `jco` + **ComponentizeJS** (embeds **StarlingMonkey** = SpiderMonkey→wasm, native WASI 0.2; Wizer snapshot) | ~8 MB base engine. Full WIT export by implementing the world as a JS module. TypeScript supported. Best-supported dynamic language. |
| **Python** | ✅ yes, mature | **componentize-py** (Bytecode Alliance; `pip install`) — embeds CPython, Wizer pre-init | Full WIT import/export, custom worlds. "Top-tier Wasm language." |
| **Ruby** | 🟡 P1 only | `ruby.wasm` (official CRuby port) | Preview 1 today (no sockets/HTTP); **no first-class componentize-ruby** for P2 yet — would need manual P1→P2 adapting. |
| **Lua** | 🟡 manual | `lua-wasi` builds exist (e.g. nalgeon/lua-wasi) | **No component-model tooling.** Practical route: embed Lua inside a *Rust* component and expose WIT from there (Lua as the scripting layer, not the component boundary). |
| **Perl** | ❌ effectively none | WebPerl (Emscripten→browser/JS) | No WASI-P2 / componentize path; browser-JS oriented. Most niche. |

**Shape of it:** JS and Python are first-class (Bytecode Alliance maintains
`ComponentizeJS` and `componentize-py`), with the same "implement the WIT world,
get a component" experience as Rust/Go — the engine embedding is hidden. Ruby is
P1-only, Lua and Perl have no first-class P2 story (you embed them inside a host
or a Rust component instead). For wandr, a JS or Python guest exporting our
`renderer` world is technically possible but better suited to **non-GUI logic /
plugin components** than the Compose-style render loop, because of the embedded-VM
size and interpreter speed.

## The common thread (good for the demo)

Almost every non-Rust path still runs **`wasm-tools component embed` + a P1→P2
reactor adapter** under the hood — the exact pipeline (and adapter dependency)
wandr already lives with for Kotlin/Wasm. Nobody but Rust has truly *escaped* the
adapter; the better toolchains just **hide** it behind one build command. So the
language-neutrality story is real and already proven in-tree:

- **Today, in wandr:** Rust guests + Kotlin/Compose guests coexist behind one WIT,
  composed by the same host linker and cross-app `link.wac`.
- **Drop-in next:** a **TinyGo** or **C#/.NET** guest exporting our `renderer`
  world would slot in with no host change — same `add_to_linker`, same install +
  AOT-precompile path (`Component::deserialize_file`), same arbiter lifecycle.
- **Coexistence mechanism:** WIT's canonical ABI is the lingua franca; the host
  bridges every interface regardless of guest language, and WAC plugs
  component-to-component deps — so a polyglot app graph is just more components
  behind the same interface types.

## Roadmap signals to watch

- **WASI 0.2** stabilized late 2024, broadly adopted through 2025 (wasmtime 30+,
  Spin, wasmCloud). This is what wandr targets.
- **WASI 0.3 / Preview 3** — native async I/O via the Component Model; first RC
  support landed in tooling late 2025. **WASI 1.0** planned for 2026.
- **Kotlin** — KT-64568 (native WASI Preview 2 target) is the only thing that
  would retire wandr's adapter; still Planned. See
  `[[reference_kotlin_wasm_component_model_status]]`.

## Sources

- [Compile Go to Wasm Components with TinyGo and WASI P2 — wasmCloud](https://wasmcloud.com/blog/compile-go-directly-to-webassembly-components-with-tinygo-and-wasi-p2/)
- [TinyGo #4843 — worlds besides wasi:cli/command](https://github.com/tinygo-org/tinygo/issues/4843)
- [Zig and the WASM Component Model — vigoo.dev](https://blog.vigoo.dev/posts/zig-wasm-component-model/)
- [componentize-dotnet — Bytecode Alliance](https://github.com/bytecodealliance/componentize-dotnet)
- [Simplifying components for .NET with componentize-dotnet](https://bytecodealliance.org/articles/simplifying-components-for-dotnet-developers-with-componentize-dotnet)
- [fermyon/teavm-wasi (Java + Component Model fork)](https://github.com/fermyon/teavm-wasi)
- [WASI & the Component Model: Current Status — eunomia](https://eunomia.dev/blog/2025/02/16/wasi-and-the-webassembly-component-model-current-status/)
- [componentize-py — Bytecode Alliance](https://github.com/bytecodealliance/componentize-py)
- [ComponentizeJS (JS → component, StarlingMonkey) — Bytecode Alliance](https://github.com/bytecodealliance/ComponentizeJS)
- [jco — JS toolchain for Wasm Components](https://github.com/bytecodealliance/jco)
- [ruby.wasm (CRuby → wasm, Preview 1)](https://github.com/ruby/ruby.wasm)
- [lua-wasi (Lua WASI build)](https://github.com/nalgeon/lua-wasi)
