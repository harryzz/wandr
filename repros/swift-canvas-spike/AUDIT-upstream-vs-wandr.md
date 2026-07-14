# eleev 2048 on OpenSwiftUI — audit: upstream gap vs. wandr patch

Question being answered: of the problems we hit porting eleev 2048 to OpenSwiftUI (off-Apple/wasm),
which are **upstream OpenSwiftUI incompleteness** (would fail on Apple too), which are **off-Apple-only
gaps** (Apple has them via CoreUI/AppKit), which are **wasm-specific** (Apple's 64-bit path is fine),
and which are **wandr patches we introduced**. Companion to `HANDOFF-eleev-openswiftui.md`.

## HEADLINE (from OpenSwiftUI's OWN README — Apple/Darwin status)
Upstream OpenSwiftUI declares itself **"early development"** and **"DO NOT use in production (App Store)."**
Its **"Current supported feature"** list literally reads:

> Color/Image/Path rendering **(Text is not supported yet)**
> Layout system · Animation system · onAppear/onDisappear · Basic geometry effect

- macOS support **⭐️⭐️⭐️ / "AppKit integration is partly implemented"**; iOS **⭐️⭐️⭐️⭐️ / "UIKit partly
  implemented"**; real devices unsupported (AttributeGraph link issue) — Simulator only.

**Consequence:** unmodified OpenSwiftUI would **NOT** render eleev 2048 correctly on Apple either —
2048 is ~half **Text** (score, tile numbers, modal titles, button labels, menu items) and OpenSwiftUI
**does not render Text yet**. So a large share of "3 weeks and 2048 still isn't perfect" is
**OpenSwiftUI being fundamentally incomplete**, which ANY port (ours, or an Apple-native build) inherits.
The wandr fork had to ADD the things upstream doesn't have (text, gesture routing, fills, symbols).

## Per-area categorization (with source evidence)

| Area | Category | Evidence | Would it fail on unmodified OpenSwiftUI *on Apple*? |
|---|---|---|---|
| **Reactive engine — `Compute` / `OpenAttributeGraph`** (the graph that IS SwiftUI's core: `@State`/`@Binding` invalidation, rule/attribute updates, subgraphs) | UPSTREAM-WIDE gap, the biggest one | OpenSwiftUI README: "The cross-platform OpenAttributeGraph is **not fully implemented**. It is only **API compatible** with AttributeGraph now… most of the core feature is only available on Apple built with the AttributeGraph variant." Off-Apple, upstream gives an API-shaped shell; the wandr fork made the actual `Compute` graph engine WORK (this is *why* @Published/reactivity, DRC scheduling, subgraph teardown, etc. behave at all off-Apple). | **YES off-Apple** (upstream is a shell); Apple uses Apple's real AttributeGraph. The hardest, mostly-invisible slice of the whole port. |
| **Text rendering** | UPSTREAM-WIDE gap | README: "Text is not supported yet". wandr ADDED host-shaped text (`StyledTextContentView` → wasi:canvas/Skia paragraph). | **YES** — no text at all. |
| **Non-color fills** (gradients / `.background(Material)`) — the "white rectangle" modals | UPSTREAM-WIDE gap | `ShapeStyleRendering.swift` `render(style:)` → `_openSwiftUIUnimplementedFailure()` for any non-`.color` fill, **UNCONDITIONALLY** (no `#if OPENSWIFTUI_LINK_COREUI` gate); reached via `renderItem` on the shared DisplayList path used by `CAHostingLayer` too. | **YES** — crashes/blank on gradients/materials. |
| **`Button` renders nothing** (was `EmptyView()`; `.buttonStyle` ignored) | UPSTREAM gap (partly) | `Button.swift` "Status: WIP (was Empty stub)"; the `ButtonStyle`/`PrimitiveButtonStyle` infra exists ("Status: Complete") but `Button` never consumed it. wandr implemented label + `.buttonStyle`. | **Likely YES** for the un-wired `Button`; needs confirming on a modern Mac. |
| **SF Symbols** (`Image(systemName:)`) | OFF-APPLE-only gap | Resolution is `#if OPENSWIFTUI_LINK_COREUI` (CUICatalog/SFSymbols, Darwin-only). wandr ADDED `OpenSFSymbols` for off-Apple. | **NO** — Apple resolves them via CoreUI. |
| **Named images** (`Image("Icon")`) / `Bundle.main` trap | OFF-APPLE-only gap | Loading is CUICatalog-gated; off-Apple resolves empty. `Bundle.main` traps on wasm (Foundation lazy global) — Apple has a real main bundle. | **NO** — Apple loads from the asset catalog. |
| **Gesture routing / arbitration** (smallest-area bind, co-delivery, hitFrame, freeze, greedy header, modal-button `.offset` hitFrame) | WANDR reimpl on an UPSTREAM TODO | Every routing file (`EventBindingManager`, `GestureViewModifier`, `HitTestBindingModifier`, `WandrRendererHost`) is dense with `[wandr]`; `GestureContainerFeature` is upstream-marked **`[TODO]`** and wandr **enables** it + adds the geometric bind path. Upstream's own gesture graph is unfinished. | **Unknown/likely partial** — upstream gesture handling is a TODO; on Apple it may lean on AppKit/UIKit hit-testing (which is only "partly implemented"). Needs the modern-Mac test. |
| **Conditional / `switch` view not swapping** (Settings/About blank) | WASM-specific | `ConditionalContent.swift` → `ConditionalMetadata.swift` (runtime metadata ABI). Prior wasm bugs were **hardcoded 32-vs-64-bit pointer/metadata** issues (`reference_openswiftui_conditional_wasm_metadata`, 2 fixed). Core conditional-view code is shared; the breakage is the wasm metadata ABI. | **NO** — Apple's 64-bit metadata path works. |
| **`ViewTransform.forEach` buffer off-by-one** (THE "cannot enter component instance" crash) | **UPSTREAM bug** (we fixed it) | `ViewTransform.swift` header: "Audited for 6.0.87, Status: WIP, ID: … (SwiftUICore)" — upstream reimplementation; the only `[wandr]` marks are our fix comments, so the buggy `withUnsafeTemporaryAllocation` + `reversed()` loop was upstream code. | **Latent on Apple too** — the uninitialized-slot read is a platform-independent logic bug; wasm just makes it deterministically fatal (garbage class-pointer retain → trap). |
| **`.allowsHitTesting` / animation-clock drive (`animPending`)** | WANDR (off-Apple plumbing) | Both were shim no-ops / reactor wiring wandr added; on Apple the real modifier + display link exist. | **NO.** |

## Bottom line for the "does OpenSwiftUI work on Apple?" question
Even on a modern Mac, **unmodified OpenSwiftUI will not render 2048 correctly today** — its own docs say Text
isn't supported and non-color fills aren't implemented, and AppKit/UIKit integration is "partly
implemented." So the honest split is roughly:
- **Big upstream gaps we had to fill:** the **`Compute`/OpenAttributeGraph reactive engine** (upstream is
  API-only off-Apple — the single hardest, mostly-invisible slice), **Text rendering**, **non-color
  fills**, **Button/ButtonStyle**, and (off-Apple) **symbols & images**. These are why so much work was
  needed — not our churn.
- **Genuinely wasm-only:** the conditional/switch metadata ABI (Apple is fine).
- **Genuinely ours (fair criticism):** the gesture-routing churn (built on an upstream TODO, then
  patched/reversed repeatedly) and the modal-button `.offset` hitFrame miss.
- **Upstream bugs we FIXED (credit, not churn):** the `ViewTransform` buffer off-by-one crash, and the
  `Bundle.main`/named-image traps.

The modern-Mac test you want is still worthwhile — but expect it to show OpenSwiftUI itself **cannot run
2048 out of the box** (no Text), which reframes the whole effort: the port isn't "fixing our mess on top
of a working OpenSwiftUI," it's "**finishing OpenSwiftUI enough to run a real app.**"
