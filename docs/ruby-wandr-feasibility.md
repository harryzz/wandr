# Ruby on wandr — feasibility memo

> Researched 2026-06-15 (web + repo source read, no spike). Companion to the
> other guest-UI evals (`avalonia-wandr-feasibility.md`, `swift-openswiftui-wandr-feasibility.md`,
> `qt-wandr-feasibility.md`, `flutter-wandr-feasibility.md`, `egui-wandr-feasibility.md`).
> Question: can a Ruby guest render on wandr through `wasi:canvas`, the way
> Slint/Avalonia/dioxus do?

## Verdict

**Viable-but-DIY guest, blocked mainly on the UI layer — NOT on the
toolchain.** This is a notably *softer* verdict than Swift/Qt: the Component
Model + custom-WIT plumbing Ruby would need is **real and already demonstrated
in-tree** (the `js` gem), not "unproven" or "diagram-only." The two real gaps:

1. **No generalized `componentize-ruby`.** Ruby *can* export/import a custom
   WIT world today, but only via a **hand-written C extension** (the pattern
   the official `js` gem uses). There is no tool that takes an arbitrary WIT
   world and generates the Ruby bridge (the way `componentize-py` does for
   Python). You'd write a "wandr bridge gem" modeled on the `js` gem.
2. **No Skia-backed Ruby UI framework — at all.** No Compose/Slint/Avalonia
   analog. Even with the bridge gem, you'd hand-draw through raw `wasi:canvas`.
   This is the same wall that gates everything except Slint/Avalonia/dioxus.

So: the component/WIT mechanism is *proven*; Ruby moves up to "you'd build a
bridge gem (with an in-repo template to copy) and then face the no-UI-framework
problem." Effort class for the bridge ≈ `componentize-py`, but starting from a
working example rather than a blank page.

## Correction to an earlier claim

An earlier read (trusting the ruby.wasm *docs page*, which lists only
`wasm32-unknown-wasip1`) concluded "ruby.wasm is wasip1 only / no component
model." **That was wrong.** Reading the repo shows a shipped wasip2 build. Lesson
(again): read the source tree, not the rendered docs. See
`[[feedback_read_source_first]]`.

## What the source actually shows

**1. ruby.wasm ships a `wasm32-unknown-wasip2` component build.**
- `builders/wasm32-unknown-wasip2/Dockerfile` — the build *environment* only
  (Debian bookworm + Rust 1.89 + a host Ruby 3.4.1 + Node 20 + katei's
  `wasi-preset-args`). No componentization commands live here.
- `Rakefile` drives it:
  ```ruby
  name: "ruby-head-wasm-wasip2",
  target: "wasm32-unknown-wasip2",
  enable_component_model: true        # the switch
  ```
- There is a published npm package `ruby-head-wasm-wasip2`.
- CRuby's C build is the *same* wasi-sdk path for both previews
  (`lib/ruby_wasm/build/product/crossruby.rb` matches `/^wasm32-unknown-wasi/`,
  which also matches `wasip2`); the p2 target layers the component-model
  wrapping on top.

**2. The base output is a command component.** Ruby runs as a `wasi:cli`-style
command component (run a script / REPL). `wasi-preset-args`
(kateinoigakukun/wasi-preset-args) bakes the `ruby` argv — note it operates at
the **wasip1 module level** (injects `args_get`/`args_sizes_get` proxies with
the argv encoded in instruction immediates), not on the component. Runnable, but
exports no custom contract.

**3. The `js` gem is a hand-rolled custom-WIT bridge — the key finding.**
`packages/gems/js/` defines its own WIT world and implements it as a C extension:
```wit
// packages/gems/js/wit/world.wit
package ruby:js;
world ext {
  import js-runtime;     // the JS host
  export ruby-runtime;   // Ruby VM control surface
}
```
```wit
// ruby-runtime.wit (excerpt)
resource rb-abi-value { }
ruby-init: func(args: list<string>);
rb-eval-string-protect: func(str: string) -> tuple<rb-abi-value, s32>;
rb-funcallv-protect: func(recv: borrow<rb-abi-value>, mid: rb-id,
                          args: list<borrow<rb-abi-value>>) -> tuple<rb-abi-value, s32>;
rb-intern: func(name: string) -> rb-id;
export-rb-value-to-js: func() -> rb-abi-value;
```
Implementation: `ext/js/witapi-core.c` + wit-bindgen-c-generated
`ext/js/bindgen/ext.c` (+ an embedded `ext_component_type.o`). I.e. **a C
extension that lifts/lowers the canonical ABI and bridges it to the Ruby
C-API.** `extconf.rb` gates it behind `--enable-component-model` /
`-DJS_ENABLE_COMPONENT_MODEL=1`.

This proves the *technique* wandr needs (custom WIT export/import from Ruby)
works today. It is just (a) hand-written and specific to JS interop, and (b)
shaped as "host drives the VM" (eval-string / funcallv / `rb-abi-value`
handles), the *inverse* of what a reactive guest exports.

## What a wandr Ruby guest would require

The component/WIT plumbing all exists; the work is:

1. **A "wandr bridge" C-extension gem**, modeled on the `js` gem:
   - WIT world: `export` the wandr handlers (`frame-handler`/`on-frame`,
     `pointer-handler`/`on-pointer`, lifecycle), `import` `wasi:canvas`
     (draw/path/paragraph), `wasi:audio`, etc.
   - Each exported WIT func → enter the Ruby VM and `rb_funcall` a Ruby method
     (the inverse direction from the js gem, but the same primitives).
   - Each imported WIT func → a Ruby native method doing canonical-ABI
     lift/lower (copy the wit-bindgen-c output the js gem already vendors).
   - Effort ≈ `componentize-py`, but with the js gem as a working template.
2. **Asyncify vs the reactor.** CRuby leans on Asyncify (fibers/exceptions/GC
   scan). Need to confirm async state survives across export calls (the
   `[[project_wandr_step_executor]]` problem, in a different runtime) — design
   work, possibly the real risk.
3. **The whole UI by hand over `wasi:canvas`.** No Ruby retained-mode/Skia UI
   framework exists; you'd build layout + widgets yourself, or a thin
   immediate-mode helper. This is the dominant cost and the reason the verdict
   stays "blocked on the UI layer."

## Where Ruby sits vs the other evals

- **Better than Swift/Qt** on the toolchain axis: custom-WIT export from the
  guest is *demonstrated in-tree* (js gem), not unproven/absent. wasip2
  component build is *shipped*, not diagram-only.
- **Same wall as Swift/Qt/Flutter** on the framework axis: no Skia-backed UI.
- **Far behind Slint/Avalonia/dioxus** (shipped) and **behind Flutter**
  (whose web_ui→CanvasKit seam is a natural `wasi:canvas` retarget once
  standalone dart2wasm lands). Ruby has no such framework seam to retarget —
  the UI would be greenfield.

## Bottom line

The interesting, corrected fact: **Ruby already speaks the Component Model and
custom WIT** (wasip2 build + the `js` gem's wit-bindgen-c C-extension bridge).
That removes the toolchain doubt. But "can author a component" ≠ "can be a
wandr UI guest": you'd hand-build a bridge gem *and* an entire UI toolkit over
`wasi:canvas`. Revisit if (a) a generic `componentize-ruby` appears, or (b)
any Skia-backed Ruby UI framework emerges — until then it's a research project,
not a port. Best near-term Ruby use on wandr is the **command-component** form
(run Ruby scripts), which works off the shelf.

## Sources
- [builders/wasm32-unknown-wasip2/Dockerfile](https://github.com/ruby/ruby.wasm/blob/main/builders/wasm32-unknown-wasip2/Dockerfile)
  · [Rakefile](https://github.com/ruby/ruby.wasm/blob/main/Rakefile)
  · `lib/ruby_wasm/build/product/crossruby.rb`
- [packages/gems/js/wit/world.wit](https://github.com/ruby/ruby.wasm/blob/main/packages/gems/js/wit/world.wit)
  · [ruby-runtime.wit](https://github.com/ruby/ruby.wasm/blob/main/packages/gems/js/wit/ruby-runtime.wit)
  · `ext/js/witapi-core.c` · `ext/js/extconf.rb`
- [kateinoigakukun/wasi-preset-args](https://github.com/kateinoigakukun/wasi-preset-args)
- [wasm32-wasip2 — rustc book](https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32-wasip2.html)
- [componentize-py (the Python analog that Ruby lacks)](https://github.com/bytecodealliance/componentize-py)
