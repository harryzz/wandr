# `my:skiko-gfx` vs wasi-gfx/wasi:webgpu — relationship, differences, and the standardization question

> Written 2026-06-11, closing the guest-UI survey. Grounded: wasi-webgpu
> upstream re-inventoried (Phase 2, wit 0.0.1 — 1093 lines / 222 funcs of
> WebGPU-as-WIT), `reference_wasi_webgpu_gfx` memory, and the shipped
> `wit/skiko-gfx.wit` + `docs/skia-wit-mapping.md`.

## The one-sentence answer

They are **two different layers of the same stack, not competitors**:
wasi-gfx/wasi:webgpu standardizes the **GPU-driver layer** (guest owns the
renderer, host provides a device), while `my:skiko-gfx` occupies the
**2D-canvas/UI layer** (host owns the renderer, guest sends drawing
*semantics*) — a layer that has **no WASI standard at all today**, which is
exactly where a "wasi-canvas" proposal grown from our contract would sit.
The web platform ships both layers for the same reason (WebGPU *and*
Canvas2D); native does too (Vulkan *and* Skia/Direct2D).

## Side by side

| | `my:skiko-gfx` (wandr) | wasi:webgpu + wasi-gfx (surface/frame-buffer) |
|---|---|---|
| Layer | 2D vector canvas + text (SkCanvas semantics) | raw GPU (pipelines, WGSL shaders, buffers, encoders) + window surface + raw framebuffer |
| Renderer lives | **HOST** (Skia on host GPU; fonts, shaping, caches host-side) | **GUEST** (each app ships its renderer: wgpu, Slint-GPU, bevy, …) |
| Guest size | tiny (dioxus guests are sub-MB of UI logic; Slint 8.7 MB incl. framework) | renderer + shaders + fonts in every guest (tens of MB) |
| Boundary traffic | per-draw-op calls, amortized by host-side retained structures (pictures, drawables, text-blob caches, paragraphs) | bulk buffer/texture uploads + command-buffer submits (fewer, fatter crossings) |
| Text | first-class, BOTH models (host-shaped `paragraph` ≈ skparagraph; guest-shaped `draw-glyphs` for parley/HarfBuzz-class guests) | none — guests bring their own shaper + rasterize into atlases |
| Scope beyond pixels | a full app-platform world: input events, IME, lifecycle, frame-pacing, window metrics, clipboard, theme… | deliberately graphics-only; `wasi:surface` carries surface input/resize/frame events |
| Host semantic control | high — host knows it's drawing UI (insets, density, on-demand rendering, per-app fps caps, accessibility someday) | none — opaque pixels; host can't reason about content |
| Power/perf fit for app UI | proven: 10–20 ms frames, ~0.7–1% idle CPU via frame-pacing | per-app GPU contexts; great for games/engines, wasteful for idle-mostly UIs |
| Status | **shipping** on a real device; 3 frameworks live (Compose, dioxus-canvas, Slint) + 2 analyzed ports (Avalonia, Flutter-gated) | Phase 2 proposal, wit `0.0.1`, no releases, API churn expected |
| Consumer adaptation | per-framework backend (the ports we wrote) | any wgpu-targeting renderer runs ~unmodified |

## What's genuinely the same

- **The sandbox-safe remoting discipline.** Both value-encode across the
  canonical ABI with opaque handles for heavy objects — our
  image/shader/typeface/picture ids play the role of WebGPU's
  buffer/texture/pipeline resources. No raw pointers, no shared memory
  (the same property that made WebGPU the only raw GPU API worth remoting
  is the property our canvas stream was built on).
- **A surface/present model.** Our `begin-frame`/`end-frame` against a
  host-owned surface ≈ `wasi:surface` + swapchain present.
- **Batching as the perf answer.** Theirs: command encoders. Ours:
  host-retained pictures/drawables so per-frame traffic shrinks to
  "replay + deltas."

## What's fundamentally different

Renderer ownership — everything else follows from it. Host-owned rendering
is wandr's premise (wasm can't reach the GPU/HW efficiently; the host keeps
semantic control and the power budget), and it's what made the Slint port
2 days and text "free." Guest-owned rendering is the right answer for the
workloads we explicitly punt on: games, 3D, custom engines, egui/bevy —
the `reference_wasi_webgpu_gfx` verdict stands: **a second path beside
skiko-gfx, never a replacement.**

## How they compose (both directions)

- **wasi-gfx ON wandr:** the task-93 `sf_media` child surfaces are exactly
  the host primitive a `wasi:surface` implementation needs (child
  SurfaceControl + BBQ producer per guest surface; `frame-buffer` = CPU
  upload into it; `webgpu` = host wgpu device → ANativeWindow; input = the
  existing per-host SfInputEvent routing). If a game/engine guest ever
  matters, wandr can host wasi-gfx alongside skiko-gfx.
- **skiko-gfx ON wasi-gfx:** a "wasi-canvas" host could itself be
  implemented over wasi:webgpu (Skia-on-wgpu), making the 2D layer
  portable to any wasi-gfx runtime — the same way Canvas2D sits on the
  GPU stack in browsers. That's the strongest argument that the two
  proposals belong in one family rather than in competition.

## Could ours become a standard? ("wasi-canvas")

**The gap is real and ours is the only shipped contender we know of**: WASI
has a proposal for the GPU layer and nothing for the 2D/UI layer, while
`my:skiko-gfx` has what proposals usually lack — a production
implementation (wandr-host), multi-framework consumers in three languages
(Kotlin/Compose, Rust/dioxus + Slint, with C# analyzed), and an empirical
API surface derived from real usage (`docs/skia-wit-mapping.md` is
effectively the draft spec). The web's Canvas2D proves the layer
standardizes well.

What would have to change to be proposable (honest list):

1. **De-Skia the contract.** Rename to canvas semantics (`my:skiko-gfx` →
   `wasi:canvas`), specify behavior (fill rules, blend modes, sampling)
   by reference to canvas/SVG/CSS semantics rather than "what Skia does."
   Our mapping doc already documents each verb's semantics — half the work.
2. **Resources instead of u32 ids.** Images/shaders/typefaces/pictures/
   paragraphs as WIT `resource`s (we used u32s for the hand-written Kotlin
   binding's sake). Mechanical but ABI-breaking — a v2 event.
3. **Shed the wandr-isms.** Launcher/status/assets/haptics/sensors etc.
   are wandr platform interfaces, not canvas. The proposal would be
   `canvas` + `text` (paragraph and glyph layers, both — our unique
   insight: managed-runtime guests need host shaping, native-runtime
   guests bring their own) + optionally `surface` reused FROM wasi-gfx.
4. **Fix the protocol warts.** The two-stage indexed-getter protocols
   (rects-for-range, line-metrics) and the 16-flat-arg dodges exist for
   historical marshaling reasons — a standard returns `list<record>`.
5. **Standards process reality.** A WASI proposal needs a champion,
   meetings, portability criteria, and ≥2 implementations at phase 3+.
   That's a long campaign, and wandr doesn't need it to keep shipping.

**Recommended path (recorded, not scheduled):** keep evolving additively;
when the surface stabilizes, publish the WIT + mapping doc as a versioned
public repo (a de-facto spec others can implement — the way wasi-gfx itself
started); only then consider the formal WASI phase-0 pitch, ideally
positioned as the 2D companion to wasi-gfx (the champions overlap is
natural — same org, complementary layers). The asset that ages best either
way is the one we already maintain: the empirically-validated API surface
and its semantics doc.
