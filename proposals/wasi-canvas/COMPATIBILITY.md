# wasi-canvas vs wandr's current model — hard-break analysis

> Written 2026-06-11, before drafting the WIT (the "check first" gate).
> Question: does anything in standardizing the 2D layer **completely break**
> wandr's shipped model? **Answer: NO — under one strategy decision and one
> red line, both stated below.**

## The strategy decision that removes almost every break

**`wasi:canvas` is a NEW package implemented over the SAME host renderer,
coexisting with `my:skiko-gfx` — not a migration.** The host already links
many packages side by side (skiko-gfx, wandr:crypto, wandr:video, alarm,
events, …); adding `wasi:canvas` is one more `add_to_linker` over the same
`SkiaRenderer` internals. Existing guests (Compose, dioxus-canvas, Slint,
chrome) keep importing `my:skiko-gfx` unchanged — zero rebuilds, the Kotlin
hand-written binding untouched, AOT caches untouched. New/ported guests opt
into `wasi:canvas`. Migration of any consumer is then per-consumer and
optional, forever.

## The one RED LINE (the only true model-breaker)

**The standard must not couple the canvas to a guest-owned event loop or
guest-created windows.** wandr's model is arbiter-authored geometry +
host-pushed events + host-driven `render-frame` (the reactor model that
makes zygote preload, on-demand rendering, bg-tick and lifecycle work). A
canvas package that *requires* `wasi:surface`-style guest-pulled pollables
and guest-constructed windows cannot host wandr's model. Therefore the
draft keeps **presentation and events OUT of the canvas package**: the
embedder hands the guest a `canvas` resource by its own contract (wandr:
inside `render-frame`; a wasi-gfx runtime: via `wasi:surface` +
graphics-context). This is also the correct layering on the merits — it is
exactly how Canvas2D is decoupled from the browser event loop. If a future
upstream process tried to mandate the coupling, that is the line wandr
advocates against (or stays on its own package — coexistence again).

## Per-change assessment (from docs/skiko-gfx-vs-wasi-gfx.md's to-do list)

| Change | Break risk for the current model | Evidence / mitigation |
|---|---|---|
| De-Skia naming + behavior spec | **None** (new package; old keeps its names) | coexistence; semantics already documented per-verb in `docs/skia-wit-mapping.md` |
| u32 ids → WIT `resource`s | **None** — proven in OUR host already | `wandr:crypto` aead-key + `wandr:video` encoder/decoder are resources on this wasmtime-45 host; WAC + resources repro'd ([[reference_missing_instance_error_stale_zygote]]); zygote/COW story identical to today's id maps |
| `borrow<shader>` inside a `paint` record | **None** — validated 2026-06-11 | `wasm-tools component wit` accepts it (paint is param-only, satisfying the borrow rule) |
| Canvas as a resource (vs today's ambient module fns) | **None** — host-side it's the same SkiaRenderer; guest-side the embedder hands the handle to `render-frame` glue | also *fixes* warts: offscreen canvases become first-class (no `bc-*` verb family), recording stops being modal |
| `list<record>` returns (kill the indexed-getter protocols) | **None for wasi:canvas consumers** (Rust/C# handle it natively); a *Kotlin* migration would face the hand-lowering cost | Kotlin stays on `my:skiko-gfx` (its two-stage getters were built FOR it); precedent: `path-combine` already returns `list<u8>` to Kotlin fine |
| Unfrozen paint (blur/mask, per-corner radii records) | **None** — new package, new record | the v1 freeze was an artifact of the live ABI, not the design |
| Dropping `WasiDrawable`/`bc-*` from the standard | **None** — they're Compose-specific retained machinery | Compose keeps using them via `my:skiko-gfx`; pictures (plain display lists) DO standardize |
| Text: paragraph + glyph layers | **None** — both already exist; standard = cleaned signatures | paragraph stays optional-for-embedders; glyph layer is the portable floor |
| Host GPU vs CPU rendering | **None** — contract is already backend-agnostic | proven: same wasm renders GPU-GL on device, CPU-raster on desktop (task 101) |

## Second-order watch items (not breaks; recorded so they don't surprise)

1. **Kotlin-class guests and the canonical ABI.** If Compose ever migrates
   to `wasi:canvas`: import returns of lists/records are workable
   (path-combine precedent) but each is hand-written lowering;
   host→guest EXPORTED records remain blocked for Kotlin
   ([[feedback_wasi_cabi_realloc_export_block]]) — irrelevant to the canvas
   package itself (import-only), relevant if events ever standardize.
2. **Resource-handle overhead.** Every draw passing `borrow<paint.shader>`
   costs a handle-table lookup vs today's u32 map hit — same order of cost,
   measure during the first port, not a design risk.
3. **Upstream churn.** If this is ever submitted and the WASI process
   reshapes it, wandr keeps shipping on `my:skiko-gfx` regardless — the
   proposal is an offering, not a dependency.
4. **`.cwasm` cache keys.** A guest that adopts `wasi:canvas` changes its
   import shape → fresh AOT compile at install. Normal reinstall flow.

## Checked against WASI's own rules (2026-06-11, WebAssembly/WASI@main)

Question: does the "drawing only — no event loop, no windowing, canvas
handed by the embedder" scope violate any WASI process rule or design
principle? **No — every relevant rule points the same direction as the
draft, and one rule makes the draft's resource design MANDATORY:**

1. **In-charter.** The WASI Subgroup Charter's Scope list explicitly
   includes "APIs for graphics, audio, input devices" (docs/Charter.md).
2. **Partial-host implementability is explicitly blessed.**
   DesignPrinciples.md, Portability: "WASI's modular nature means that
   engines don't need to implement every API in WASI, so we don't need to
   exclude APIs just because some host environments can't implement
   them… we'll ultimately decide whether something is 'portable enough'
   on an API-by-API basis." A canvas no server host implements is fine.
3. **Decoupling is the prescribed style, not a deviation.**
   DesignPrinciples.md, Modularity: "WASI will include many interfaces
   that are not appropriate for every host environment, so WASI uses the
   component model's worlds mechanism to allow specific sets of APIs…" —
   splitting drawing from windowing/events is exactly this (and wasi-gfx
   itself splits webgpu/surface/frame-buffer into separate packages).
4. **Host-driven (reactor) guests are core WASI practice.** wasi:http's
   `proxy` world `export incoming-handler` — the HOST invokes the guest
   per request, the precise shape of wandr's render-frame model. Nothing
   anywhere mandates guest-owned loops; WASI 0.2's only event primitive
   (pollables) is itself just another importable interface.
5. **Capability rules REQUIRE the draft's resource design.**
   DesignPrinciples.md: "All access to external resources is provided by
   capabilities… WASI has no ambient authorities, meaning there are no
   global namespaces at runtime." Capabilities.md: multiple simultaneous
   resources (our main + offscreen canvases) ⇒ handles. Notably,
   `my:skiko-gfx`'s ambient module-level draw functions + u32 id
   namespaces would NOT pass this rule — the draft's canvas-as-handle
   (handed by the embedder = a runtime capability) is not just cleaner,
   it's the only standardizable shape.
6. **Interposition must be possible** (DesignPrinciples.md): any WASI API
   should be implementable by another component. The draft is plain
   funcs + resources, so a wasm interposer works — including the
   "wasi:canvas implemented over wasi:webgpu" layering, which this
   principle actively wants to exist.

~~Likely review pressure~~ RESOLVED in-draft (2026-06-11): creation
statics were refactored into a `graphics` FACTORY RESOURCE the embedder
grants alongside the canvas — every host-resident allocation (shaders,
images, offscreen canvases, recordings) now flows through a handle, so
embedders can attenuate it (quotas, deny decode). The only remaining
no-handle creation is `typeface.from-bytes` in the text package, kept
deliberately: it's pure computation over the guest's OWN bytes, not
access to an external resource (the distinction Capabilities.md draws).

## Verdict

**Proceed with the draft.** No hard breaks under the coexistence strategy;
one red line (no event-loop/windowing coupling), enforced by the draft's
scope; everything else is mechanical or strictly-better redesign.
