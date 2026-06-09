# Task 51 — Dynamic lang-plugin loading (host-mediated)

> Status: 🔲 scoped, not started — 2026-05-28
>
> Polish follow-up to task 49 step 5. Removes the IME's hardcoded
> plugin registry; new languages drop in as `wandr.lang.<id>.wandrpkg`
> with zero IME-side changes.

## Why

Task 49 step 5 shipped per-plugin WIT package names because two
deps cannot share a `linker.instance(name)` entry in wandr-host's
`wire_dep_into_linker`. The cost: the IME hard-codes its known
plugins in `LangAdapter.kt` (one `@WasmImport` block per plugin,
one entry in the static `plugins: List<Loader>` registry). Adding
a new language requires editing the IME *and* re-shipping its
wandrpkg.

Goal: invert the relationship. The IME asks the host for the list
of installed lang plugins; the host enumerates them and proxies
the calls. New languages install + work without touching the IME.

## Design (sketch)

**Host side** (`wandr-host`):
- New WIT verbs on the existing `my:skiko-gfx/keyboard` interface
  (or a new `my:skiko-gfx/lang-plugins` interface):
  ```wit
  record lang-plugin-info {
      id:        string,   // "bg" / "fr" / "de" …
      name:      string,   // "Български"
      locale:    string,
      is-rtl:    bool,
  }
  enumerate-lang-plugins: func() -> list<lang-plugin-info>;
  // `lang-id` = the per-plugin id; host dispatches to the right
  // installed component.
  get-lang-layout: func(lang-id: string, shifted: bool)
      -> list<list<key-def>>;
  ```
- Host startup scans `<APPS_ROOT>/system-apps/wandr.lang.*/`. For
  each, eagerly `Engine::deserialize_file` the cwasm + cache
  `wasmtime::component::Instance` + the two `Func` handles
  (`get-info`, `get-layout`). Map keyed by lang-id.
- `enumerate-lang-plugins` returns the cached infos.
- `get-lang-layout(lang-id, shifted)` looks up the cached Func,
  calls it with a single-`Val::Bool` param, lifts the returned
  layout-variant out of the dep's linear memory, and re-lowers it
  into the consumer's typed return area.

**IME side** (`wandr.ime.keyboard`):
- `LangAdapter.kt` collapses to ~30 LoC: one `@WasmImport` for
  each new host verb; one `loadAllLangPlugins()` that calls
  `enumerate-lang-plugins`, then for each id calls
  `get-lang-layout(id, false/true)`, wraps via
  `ImeKeyboardDefaults.wrapLanguageLayout`, returns the list.
- Static `plugins: List<Loader>` registry deleted.
- `[dependencies]` block in `package.toml` deleted (host owns
  plugin lifecycle, not the IME's installer manifest).

**Plugin contract** (wandr.lang.\*):
- Each plugin still exports `wandr:keyboard-lang-<id>/lang@0.1.0`
  (the per-plugin package convention stays — keeps the contract
  visible at the wandrpkg layer + makes plugins individually
  introspectable with `wasm-tools component wit`).
- The HOST is responsible for instantiating each plugin into its
  own `Store` and calling its exports; the IME no longer directly
  imports the plugin's interface.

## Trade-offs

- Wins:
  - Zero IME rebuilds for new languages.
  - Single source of truth for which plugins exist (filesystem
    scan instead of compile-time registry).
  - Cleaner cabi-boundary: the host's typed wasmtime bindgen
    handles the lift, no more hand-rolled canonical-ABI Kotlin.
- Costs:
  - Host gains plugin lifecycle responsibility — currently apps
    only have one `Engine`/`Linker`/`Store` per process; the host
    will spin up an extra Store per lang plugin at startup.
    Memory: each precompiled lang cwasm is small (~80 KB), so the
    incremental cost is ~80 KB × N plugins; negligible.
  - One extra WIT verb shape (`enumerate-lang-plugins`) lives on
    the host's main interface, slightly fattening the contract.
  - Doesn't help non-IME consumers; if another app wants lang
    plugins it would need its own host-mediated bridge or import
    the IME's interface (probably wrong layering).

## Steps

1. **Host scan.** New `wandr-host/src/lang_plugins.rs` —
   directory-scan of `<APPS_ROOT>/system-apps/wandr.lang.*/0.1.0/cache/lang.cwasm`,
   `Engine::deserialize_file` per plugin, store
   `Vec<{ id, instance, get_info_func, get_layout_func }>` in a
   `HostState` field.
2. **New WIT verbs.** Add `enumerate-lang-plugins` +
   `get-lang-layout` to `wit/skiko-gfx.wit` under a new
   `interface lang-plugins`. Mirror to wandr-app/IME deps.
3. **Host impl.** `wandr-host/src/lang_plugins_impl.rs` — proxy
   `get-lang-layout(id, shifted)` to the cached Func, lift +
   re-lower the layout-variant. Use wasmtime typed function APIs
   (define a local Rust struct matching the lang-variant shape
   and `TypedFunc<(bool,), (LayoutVariant,)>` if possible — avoids
   manual canonical-ABI dance).
4. **IME refactor.** `LangAdapter.kt` collapses to one
   `enumerate-lang-plugins`-driven loop. Delete the per-plugin
   `@WasmImport` blocks and the static `plugins` registry.
   `package.toml` `[dependencies]` block removed.
5. **Smoke.** Install bg + fr as before, IME picks them up via
   the new host enumeration. Add a third throwaway plugin
   (wandr.lang.de mock) to prove zero-touch: install wandrpkg,
   IME shows German on next launch with no code change.

## Estimated effort

~4-6 hours. The Rust side is the bulk (typed function call
lift/lower + per-plugin Store management); the Kotlin side
shrinks by ~100 LoC.

## When to do this

Wait until:
- A third language plugin is concretely needed (proves the
  zero-touch property is worth the host refactor), OR
- The cabi_realloc record-with-strings bug (see
  `feedback_wasi_cabi_realloc_export_block`) becomes more pressing
  and we want to retire the hand-written Kotlin canonical-ABI
  lifts in `LangAdapter.kt`, OR
- Distributing wandr externally — third-party language plugins
  shouldn't require shipping a custom IME build.
