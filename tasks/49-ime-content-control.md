# Task 49 — IME content control: editor-driven layouts + language plugins

> **Status:** 🔲 scoped 2026-05-27, revised 2026-05-27. Two follow-ups
> to task 47 step 3c that share the same inbound-delivery
> infrastructure:
>
>   1. The IME should adapt its layout to the focused editor's type
>      (numeric pad for `KeyboardType.Number`, phone keypad for `Phone`,
>      etc) instead of always showing the English QWERTY.
>   2. Languages should ship as installable plugins (`war.lang.fr`,
>      `war.lang.de`, …) so adding a new language doesn't require
>      rebuilding the IME app.
>
> Folded into one task because both extend the same `layoutName` /
> `layouts: List<KeyboardLayout>` mechanism in `ImeKeyboard.kt` and
> both depend on the same delivery path (arbiter → IME).
>
> **Revision (pre-implementation review):** the original v1 of this
> doc proposed adding a new `interface ime-events` to
> `wit/skiko-gfx.wit` and a new `wart-host/src/ime_outbound.rs`
> socket file. Both were redundant:
>
>   - `wit/ime.wit` (shipped in task 47 step 1) ALREADY defines
>     `interface ime { on-editor-attached(info), on-editor-detached() }`
>     with a proper typed `input-type` enum and `editor-info` record.
>     `war.ime.keyboard`'s WIT just needs to `export war:ime/ime`.
>   - `wart-host/src/ime_inbound.rs` (shipped in step 3a) ALREADY has
>     the per-host socket listener + queue + render-loop drain. It
>     just needs new message types — `editor-attached` /
>     `editor-detached` alongside the existing `key-event`. Same
>     socket file (`/data/local/tmp/wart-host-<pid>.sock`), same
>     listener thread, same queue, just additional `InboundEvent`
>     variants and additional dispatch arms.
>
> Net: smaller diff than v1 promised. The genuinely new pieces are
> (a) the host-side wasmtime bindings to CALL into the IME's
> exported `war:ime/ime`, and (b) the keyboard-lang plugin contract.

## Why this task exists

Today `war.ime.keyboard` shows the English QWERTY for every focused
editor — a calculator demanding digits gets the same alphabetic
keyboard as a chat box. And adding French / German / Spanish today
means editing `ImeKeyboard.kt`, rebuilding the IME, repacking the
warpkg, and pushing — a heavy operation for what should be a single
file drop.

The protocol pieces are already partially in place. From step 1:

```wit
enum input-type { text, number, phone, email, url, password, multiline-text }
record editor-info { input-type, hint, initial-text, selection-start, selection-end }
```

…and wart-app forwards an `EditorInfo` to the arbiter via
`attach-editor`. The arbiter stores it. But the IME guest never
sees it — the inbound delivery into the IME's process isn't wired.
Wiring that delivery is the prerequisite for both features.

## Pre-task design decisions

**D1. Two distinct concerns share one mechanism.** Both features
ultimately resolve to "pick a `KeyboardLayout` from a list". They
differ in WHO picks:
  - Editor-driven (numeric/phone/email): the **host** picks via
    `on-editor-attached(input-type)`; user can't override.
  - Language: the **user** picks via the 🌐 cycle; the layout list
    is composed from built-ins + plugins.

**D2. Editor-type override wins.** If the focused field is
`KeyboardType.Number`, the IME shows the numeric pad and the 🌐
button is hidden / disabled until the field defocuses. Restores
the user's last-selected language on detach.

**D3. Built-in vs plugin: hybrid.** Universal layouts ship in the
IME app (English, Numeric, Phone, Email, Password, Symbols,
Symbols2, Emoji). Languages ship as plugins (`war.lang.bg`,
`war.lang.fr`, …). Bulgarian moves OUT of built-ins as the first
example — proves the migration path.

**D4. Plugin contract is a thin "data + future-composition"
interface.** `lang.get-info()` returns metadata; `lang.get-layout(
shifted)` returns rows. Future CJK / Indic / autocorrect IMEs
extend with `start-composition` / `commit-composition` /
`current-composition` — out of scope for this task but the
contract leaves room.

**D5. Stage 1 = declared deps; Stage 2 = dynamic discovery.** This
task ships Stage 1 (war.ime.keyboard's `package.toml` declares
which languages it supports). Stage 2 (the arbiter exposes
`list-installed war.lang.*` and the IME picks them up at startup
without manifest changes) is queued for later — same architecture,
just a discovery layer on top.

**D6. One socket per host, multiple message types.** The existing
`/data/local/tmp/wart-host-<pid>.sock` (opened by `ime_inbound.rs`
in step 3a, currently carrying only `key-event` messages) is the
delivery channel for ALL arbiter-→-host messages. New types
`editor-attached <input-type> <hint> <initial-text-len>
<selection-start> <selection-end>` and `editor-detached` get
parsed alongside the existing `key-event`. The host process
knows whether it's playing editor-role or IME-role based on its
launch flag (`--standalone-overlay` = IME); messages of the
"wrong" type for the current role are logged + dropped.

**D7. Wart-app's hardcoded `input-type="text"` is NOT a one-line
change.** The existing `WasiKeyboardController.show()` overrides
`SoftwareKeyboardController.show(): Unit` which takes no
arguments — there's no field context in scope. Two options:
  - **MVP hack:** TextFieldCard reads its own
    `KeyboardOptions.keyboardType` and writes it to a state
    holder before calling `keyboardController.show()`. The
    controller's `show()` reads from the state holder.
    Not idiomatic Compose, but lightweight.
  - **Proper:** wire through Compose's
    `PlatformTextInputMethodRequest` — a multi-file refactor
    that crosses Compose's text-input plumbing.

This task picks the MVP hack; the proper refactor is a separate
follow-up worth its own task if a real BasicTextField use case
demands it.

## Steps

### Step 1 — Inbound delivery: socket layer + host→guest bindings (~2 h)

The piece both features need. Two sub-layers:

**1a. Socket-layer (extend `ime_inbound.rs`):**

The file already opens `/data/local/tmp/wart-host-<pid>.sock` in
step 3a and parses `key-event` messages. Extend with two new
message types:

```
editor-attached <input-type> <hint-quoted> <initial-text-len> <selection-start> <selection-end>
editor-detached
```

- `input-type` is the bare enum-tag string (`text` / `number` /
  `phone` / …) — same set as `war:ime/input-type`.
- `hint-quoted` is space-escaped (spaces → `_`) per the existing
  attach-editor CLI convention, OR newline-terminated and
  read-to-EOL — pick whatever's simplest given the existing
  parser shape. (`initial-text` is currently dropped at the wire
  layer because the existing `attach-editor` CLI uses positional
  args; the inbound socket is binary-safe so we can carry the
  real string. Decide during implementation.)
- `editor-detached` takes no args.

In `ime_inbound.rs`:
- Extend `enum InboundEvent` with `EditorAttached { info: EditorInfo }`
  and `EditorDetached`.
- Add parse arms in `parse_and_queue` for the two new prefixes.
- `drain_queue()` already returns the polymorphic enum.

In `wart-host/src/standalone.rs`:
- The per-frame drain at the existing `match ev` site grows two
  new arms — `EditorAttached { info }` → host calls IME guest's
  exported `war:ime/ime.on-editor-attached(info)`; `EditorDetached`
  → host calls `on-editor-detached()`.

In `wart-arbiter/src/main.rs`:
- `cmd_attach_editor` already validates + stores `EditorFocus`.
  After successful auto-overlay-promote, also write
  `editor-attached <type> <hint> <text-len> <selstart> <selend>`
  to the active IME's per-host socket
  (`/data/local/tmp/wart-host-<ime-pid>.sock`). Reuses the
  `deliver_to_host` one-shot-connect helper already in main.rs.
- `cmd_detach_editor` writes `editor-detached`.

**1b. Host→guest bindings for `war:ime/ime`:**

The `war:ime/ime` interface (`on-editor-attached(info)` /
`on-editor-detached()`) is EXPORTED by the IME guest and called
by the host. Today the host's bindgen only knows about
`skiko-ui` (which exports only `renderer`). To call into the IME
guest's `war:ime/ime`, two options:

- **Option A — Second `bindgen!` macro.** Add:
  ```rust
  mod ime_bindings {
      wasmtime::component::bindgen!({
          path: "../wit/",
          world: "war:ime/ime-client-world",
      });
  }
  ```
  Generates typed `EditorInfo` struct + typed `call_on_editor_attached`.
  Cleaner; matches the existing skiko-ui bindgen pattern. The
  guest must be linked against that world, so `war.ime.keyboard`'s
  `wit/war-ime-keyboard.wit` needs to `include` `ime-client-world`
  (or restructure to use it as the base world).

- **Option B — Untyped `Val` calls.** Look up the export at
  runtime via the wasmtime Component API:
  ```rust
  let ime_iface = instance.get_export_index(&mut store, None, "war:ime/ime@0.1.0").unwrap();
  let on_attached = instance.get_export_index(&mut store, Some(&ime_iface), "on-editor-attached").unwrap();
  let func = instance.get_func(&mut store, on_attached).unwrap();
  // Build EditorInfo as wasmtime::component::Val::Record(...)
  func.call(&mut store, &[Val::Record(...)], &mut [])?;
  ```
  Verbose but no bindgen plumbing. Matches the pattern task 39
  uses for cross-app dep wiring.

Option A is cleaner architecturally; Option B has lower bind-time
overhead and is closer to what task 39 already does. **Pick Option A
unless the second bindgen! causes name collisions or build-time
pain** — the typed `EditorInfo` makes the call site readable.

**Wart-app side: nothing changes.** wart-app already calls
`Ime.Import.notifyEditorAttached(type, ...)` (outbound from the
editor); that's the existing path. Step 2 fills in the real
`type` instead of hardcoded `"text"`.

**`war.ime.keyboard` side: export the interface.**

- `war.ime.keyboard/wit/war-ime-keyboard.wit` — add
  `export war:ime/ime;` (already has `include my:skiko-gfx/skiko-ui`).
  May need to restructure to be `world ime-keyboard { include
  my:skiko-gfx/skiko-ui; include war:ime/ime-client-world; }`
  or split the export off — verify during impl.
- `war.ime.keyboard/wit/deps/ime/ime.wit` — mirror of
  `wit/ime.wit`.
- `war.ime.keyboard/src/wasmWasiMain/kotlin/generated/SkikoUi.kt`
  (or a new file): hand-add `@WasmExport` extern declarations
  for `war:ime/ime.on-editor-attached` and `on-editor-detached`.
  These are EXPORTS, not imports — the canonical-ABI lowering
  is different (caller-allocated return area, etc.). The
  `record editor-info { input-type, hint, initial-text,
  initial-selection-start, initial-selection-end }` shape needs
  hand-written lift code on the wasm side.

**Success criterion:** `wart-arbiter attach-editor <wart-app-pid>
number` → adb logcat shows the IME's exported
`on-editor-attached` Kotlin function fired with
`input-type="number"`.

### Step 2 — Editor-driven layout switching (~1 h)

The IME-side reaction. Adds the numeric / phone / email / password
layouts and the policy that picks based on input-type.

- **`war.ime.keyboard/src/wasmWasiMain/kotlin/ImeKeyboard.kt`** — new
  built-in layouts:
  - `Numeric`: digits + `.` + `-` + ⌫⏎ in a 4×4 grid. Used for
    `input-type=number`.
  - `Phone`: digits + `*` + `#` + `,` + pause + ⌫⏎. Used for
    `input-type=phone`.
  - `Email`: full English QWERTY + dedicated `@` and `.com` /
    `.org` keys in the modifier row. Used for `input-type=email`.
  - `Url`: same as Email but `.com` / `/` / dedicated TLD keys.
  - `Password`: same shape as English but `isLanguage = false`,
    no autocorrect hint (future task), no shift→caps-lock
    (single-shift only).

- **State plumbing** — new `currentEditorType: MutableState<String>`
  in the composable, defaults to `"text"`. The IME's exported
  `on-editor-attached(info)` Kotlin handler stores info.input_type
  here.

- **Layout pick** — replaces the simple `layoutName` lookup:
  ```kotlin
  fun pickLayout(
      editorType: String,
      userSelectedLang: String,
      requestedLayout: String,
  ): String = when (editorType) {
      "number", "multiline-text" -> if (editorType == "number") "Numeric" else userSelectedLang
      "phone" -> "Phone"
      "email" -> "Email"
      "url"   -> "Url"
      "password" -> "Password"   // letters but no extras
      else -> requestedLayout    // user toggled via 123 / 🌐 / 😀
  }
  ```
  The 🌐 button cycles `userSelectedLang` (English / Bulgarian /
  French / …) and is disabled when editor-type forces a layout.

- **`wart-app/src/wasmWasiMain/kotlin/WasiKeyboardController.kt`** —
  passes the focused field's keyboardType through. The
  `SoftwareKeyboardController.show()` signature has NO arguments,
  so we can't read the keyboardType inside the controller from
  Compose's standard path. **MVP hack** (per D7):
  ```kotlin
  class WasiKeyboardController : SoftwareKeyboardController {
      val isVisible: MutableState<Boolean> = mutableStateOf(false)
      // NEW: writable from outside; TextFieldCard sets this on focus.
      var pendingKeyboardType: KeyboardType = KeyboardType.Text
      override fun show() {
          isVisible.value = true
          val type = pendingKeyboardType.toImeWireString()  // see helper
          Ime.Import.notifyEditorAttached(type, /* hint */ "", /* initial */ "", 0u, 0u)
      }
      override fun hide() { … }
  }

  // In TextFieldCard, before calling controller.show():
  .onFocusChanged { fs ->
      if (fs.isFocused) {
          keyboardController.pendingKeyboardType = KeyboardOptions(...).keyboardType
          keyboardController.show()
      } else {
          keyboardController.hide()
      }
  }
  ```
  Not idiomatic — Compose's proper path is
  `PlatformTextInputMethodRequest`, but threading that through
  is a multi-file refactor across `wart-app/.../wasmWasiMain/`
  Compose actuals. The MVP hack covers the demo cases (TextField
  Card has the type at the focus-changed callsite anyway).
  + add at least one Number TextField and one Phone TextField to
  wart-app's demo so the path is exercised on the smoke.

**Success criterion:** tap a Number TextField in wart-app → IME
displays the numeric keypad. Tap a normal TextField → IME goes back
to English QWERTY. Tap a Phone TextField → IME shows phone keypad.

### Step 3 — Language plugin contract (~1 h)

The `war:keyboard-lang/lang` WIT package.

- **`wit/keyboard-lang.wit`** (new):
  ```wit
  package war:keyboard-lang@0.1.0;

  interface lang {
      record info {
          name: string,         // "Français"
          locale: string,       // "fr-FR" (BCP-47)
          is-rtl: bool,
      }
      record key-def {
          display: string,
          code-point: u32,
          key-id: u32,
          width: f32,
      }
      type key-row = list<key-def>;
      record layout-variant {
          rows: list<key-row>,
      }
      get-info:   func() -> info;
      get-layout: func(shifted: bool) -> layout-variant;
  }
  world lang-world { export lang; }
  ```

- Mirror to `wart-app/wit/deps/` and `war.ime.keyboard/wit/deps/`
  per the established WIT mirror rule.

- **Move Bulgarian out.** Delete the `Bulgarian` `KeyboardLayout`
  from `ImeKeyboardDefaults` and create `war.lang.bg/` (Rust cdylib
  exporting the same data via the WIT interface). Proves the
  migration pattern.

**Success criterion:** `wasm-tools component wit
war.lang.bg/components/lang.wasm` validates against
`wit/keyboard-lang.wit`. The IME's English QWERTY still works after
Bulgarian is removed (because it's loaded as a plugin now).

#### Step 3 results

- `wit/keyboard-lang.wit` written, validated with `wasm-tools component
  wit`. Mirrored to `war.ime.keyboard/wit/deps/keyboard-lang/`. Not
  mirrored to `wart-app/wit/deps/` — wart-app doesn't import the
  contract.
- `war.lang.bg/` Rust cdylib added: `Cargo.toml` + `src/lib.rs`
  (~60 LoC). Exports `war:keyboard-lang/lang@0.1.0` with `get-info` →
  `{ name = "Български", locale = "bg-BG", is-rtl = false }` and
  `get-layout(shifted)` returning the 3-row БДС-style ЯВЕРТЫ data
  (uppercase + lowercase variants) verbatim from the Kotlin source
  that lived in `ImeKeyboardDefaults.Bulgarian`. Build:
  `cargo build --target wasm32-wasip2 --release` succeeds (~44 s cold);
  `wasm-tools component wit target/.../war_lang_bg.wasm` shows
  `export war:keyboard-lang/lang@0.1.0`.
- `war.ime.keyboard` updated:
  - `ImeKeyboard.kt`: `Bulgarian` KeyboardLayout deleted; `layouts()`
    list no longer references it; file-top comment + `RealComposeApp`
    comment updated to point at task 49 step 5 for plugin loading.
- IME re-built via `compileProductionExecutableKotlinWasmWasi` — passes,
  English QWERTY still drives the 🌐 cycle (now a 1-element cycle until
  step 5 wires plugins in).
- The `scripts/build-system-warpkgs.sh` integration is deferred to step
  5 (alongside the IME's `[dependencies]` declaration and the Kotlin
  `LangAdapter`). Step 3's scope was just the contract + first plugin.

### Step 4 — Sample language plugin: war.lang.fr (~1 h)

New sibling repo `war.lang.fr/` (Rust cdylib). Static AZERTY data.

- `Cargo.toml` — `cdylib`, depends on `wit-bindgen` 0.36+ for Rust.
- `src/lib.rs` — implements the `lang` interface. ~150 LoC of
  hand-coded `KeyDef`s for AZERTY layout (normal + shifted).
- `wit/lang.wit` — local copy of the contract.
- `package.toml`:
  ```toml
  app_id      = "war.lang.fr"
  version     = "0.1.0"
  world       = "war:keyboard-lang/lang-world"
  kind        = "system"
  composition = "same-store"

  [components]
  lang = "components/lang.wasm"
  ```

- **`scripts/build-system-warpkgs.sh`** — add steps to build / package
  / push / install the new language warpkgs (war.lang.bg from
  step 3 and war.lang.fr from this step).

**Success criterion:** `wart-host --install war.lang.fr.warpkg`
succeeds; `<APPS_ROOT>/system-apps/war.lang.fr/0.1.0/` contains
the precompiled cwasm.

#### Step 4 results

- `war.lang.fr/` Rust cdylib added — sibling of `war.lang.bg/`,
  same shape (~60 LoC). `get-info` → `{ "Français", "fr-FR",
  is-rtl=false }`. `get-layout(false)` returns standard AZERTY
  letter rows (`azertyuiop / qsdfghjklmù / wxcvbnà`);
  `get-layout(true)` returns uppercase variants (without ù/à —
  Shift-AZERTY drops the trailing accent keys to match physical
  French keyboard behavior). Lone accent dead-keys deferred (no
  dead-key concept in `KeyDef`).
- Build: `cargo build --target wasm32-wasip2 --release` → 26 s.
  `wasm-tools component wit` shows
  `export war:keyboard-lang/lang@0.1.0`.
- `scripts/build-system-warpkgs.sh` extended with `FR_WASM` /
  `FR_PKG` / `pack_warpkg` block / push + install loop entry —
  pattern-identical to the war.lang.bg additions in step 3.
- Step 5 declares both plugins as IME `[dependencies]`; the
  device 🌐 cycle becomes English → Bulgarian → French.

### Step 5 — Dynamic loading in war.ime.keyboard (~1 h)

The IME enumerates declared lang deps + loads them via the existing
generic dep wiring.

- **`war.ime.keyboard/package.toml`** — add `[dependencies]`:
  ```toml
  [dependencies]
  bg = { system = "war.lang.bg", version = "0.1.0", interface = "war:keyboard-lang/lang@0.1.0" }
  fr = { system = "war.lang.fr", version = "0.1.0", interface = "war:keyboard-lang/lang@0.1.0" }
  ```

- **`war.ime.keyboard/src/wasmWasiMain/kotlin/LangAdapter.kt`** (new) —
  the bridge:
  - Hand-written WIT bindings (wit-bindgen 0.53.1 has no Kotlin
    generator — see `feedback_wit_bindgen_no_kotlin_generator`).
    Models `lang.get-info` + `lang.get-layout(shifted)`.
  - `loadAllLangPlugins(): List<KeyboardLayout>` — calls each
    declared lang's `get-info` + `get-layout(false/true)` once
    at startup, builds `KeyboardLayout` structs, returns the list.

- **`war.ime.keyboard/src/wasmWasiMain/kotlin/ImeKeyboard.kt`** —
  `ImeKeyboardDefaults.layouts()` becomes:
  ```kotlin
  fun loadAllLayouts(): List<KeyboardLayout> {
      val builtins = listOf(English, Numeric, Phone, Email, Url, Password, Symbols, Symbols2, Emoji)
      val plugins  = LangAdapter.loadAllLangPlugins()  // [Bulgarian, French, ...]
      return builtins + plugins
  }
  ```
  Pass the combined list to `ImeKeyboard(...)`. The 🌐 cycle
  iterates `userSelectedLang` candidates filtered by
  `isLanguage = true` — both built-in English + all plugins.

- **Plumbing the wart-host generic dep wiring** — task 39 already
  registers proxy closures via `LinkerInstance::func_new`. No
  changes to wart-host. The IME's wasm just declares the imports;
  the loader walks them.

**Success criterion:** tap 🌐 in the IME → cycles
`English → Bulgarian → French`. Each language renders correctly.

### Step 6 — Smoke + memory + close-out (~30 min)

- Device smoke covering the four pivots:
  1. Tap a regular TextField → English QWERTY.
  2. Tap a Number TextField → numeric keypad. 🌐 disabled.
  3. Defocus + retap regular field → English. Tap 🌐 once →
     Bulgarian. Tap 🌐 again → French AZERTY.
  4. Tap a Phone TextField → phone keypad. Phone-specific keys
     (`*`, `#`, pause) visible.

- New memory `feedback_ime_layout_arbitration.md` — captures the
  editor-type-override vs user-selected-language dichotomy and the
  built-in-vs-plugin split.

- Update `tasks/47-ime-via-guest-app.md` Step 4 with a forward
  reference: "task 49 supplies the inbound socket; step 4 reuses
  it for tap-outside-to-hide / `request-hide` / focus changes."

## File-touch map

| Repo | File(s) | Why |
|---|---|---|
| **wart** | `wit/keyboard-lang.wit` (new) | Plugin contract for lang plugins |
| **wart** | `tasks/49-ime-content-control.md` | This file, with results section per step |
| **wart** | `scripts/build-system-warpkgs.sh` | Build / package the new lang warpkgs |
| **wart-host** | `src/ime_inbound.rs` | Extend with `editor-attached` / `editor-detached` parsing + new `InboundEvent` variants |
| **wart-host** | `src/standalone.rs` | New drain arms calling IME's `on-editor-attached` / `on-editor-detached` |
| **wart-host** | `src/lib.rs` | Second `bindgen!` macro for `war:ime/ime-client-world` (Option A) OR untyped `Val` calls in standalone.rs (Option B) |
| **wart-arbiter** | `src/main.rs` | `cmd_attach_editor` / `cmd_detach_editor` write to IME's per-host socket via existing `deliver_to_host` helper |
| **wart-app** | `src/wasmWasiMain/kotlin/WasiKeyboardController.kt` | `pendingKeyboardType` field + `show()` reads it; per-D7 MVP hack |
| **wart-app** | `src/wasmWasiMain/kotlin/RealComposeApp.kt` | TextFieldCard sets `pendingKeyboardType` in `.onFocusChanged` |
| **wart-app** | demo screens add Number + Phone TextFields | Smoke coverage |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/ImeKeyboard.kt` | New built-in layouts (Numeric / Phone / Email / Url / Password) + pickLayout |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/LangAdapter.kt` (new) | Hand-written WIT bindings + loader for lang plugins |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/generated/SkikoUi.kt` (or new file) | `@WasmExport` declarations for `war:ime/ime.on-editor-*` (the EXPORT side — needs canonical-ABI lift for `editor-info` record) |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/RealComposeApp.kt` | Wire the new exported `on-editor-attached` → updates `currentEditorType` MutableState |
| **war.ime.keyboard** | `package.toml` | `[dependencies]` for lang plugins |
| **war.ime.keyboard** | `wit/war-ime-keyboard.wit` | `import keyboard-lang` + restructure world to export `war:ime/ime` |
| **war.ime.keyboard** | `wit/deps/ime/ime.wit` (new mirror) | Mirror of `wart/wit/ime.wit` |
| **war.ime.keyboard** | `wit/deps/keyboard-lang/keyboard-lang.wit` (new) | Mirror |
| **war.lang.bg** (new repo) | full Rust cdylib | Bulgarian, extracted from built-ins |
| **war.lang.fr** (new repo) | full Rust cdylib | French AZERTY |

**Not touched (corrections from v1):**

- `wit/skiko-gfx.wit` + 3 mirrors — no changes. v1 proposed a new
  `ime-events` interface; that's redundant with `war:ime/ime`.
- `wart-host/src/ime_outbound.rs` — no new file. The existing
  `ime_inbound.rs` covers it.
- `skiko/.../generated/{Internal,}SkikoUi.kt` — no `ime-events`
  Kotlin bindings needed (the IME-side exports go in
  `war.ime.keyboard`'s own generated files, NOT in skiko's
  shared bindings).

## Boundary with task 47 step 4

Step 4 of task 47 (InputFlinger focus arbitration + auto-hide)
shares the same arbiter-→-host inbound channel. The split:

- **Task 49 owns the wire format.** The `editor-attached` /
  `editor-detached` messages and their parsing in
  `ime_inbound.rs`. The host→guest bindings for `war:ime/ime`.
- **Step 4 owns the policy.** When does the arbiter emit
  `request-hide`? What does tap-outside-the-IME's-input-window
  mean? Step 4 grows `ime_inbound.rs` with whatever ADDITIONAL
  message types it needs (e.g. `request-hide`, future
  `config-changed`) — task 49 doesn't pre-add them.

If step 4 needs to dispatch an event INTO the IME guest (e.g.
"please hide yourself" as a guest-callable WIT verb), it can
piggy-back on the `war:ime/ime` interface — either by adding a
new func there or by adding a NEW interface alongside. Task 49
doesn't pre-decide; step 4 picks when it's actually implementing.

## Considerations / risks

- **Generic dep wiring (task 39) loads deps as `lower_to_func`
  closures with `Val`-boxing.** For the IME's startup load (called
  once per language), boxing cost is negligible. Future stateful
  IMEs that call `current-composition` at 60 Hz might need a
  typed wrapper — defer until that happens.

- **Hand-written Kotlin WIT bindings are tedious.** The `lang`
  interface has variable-length data (lists of records with
  strings); the canonical-ABI lift / lower is non-trivial. Plan
  to spend most of step 5's hour on this. Reference
  `wart-app/.../MarkdownImports.kt` — the markdown renderer has
  the same shape (records with strings in lists) and the lift
  pattern carries over.

- **The `war:ime/ime` EXPORT side is harder than typical IMPORTs.**
  Per `feedback_canonical_abi_import_export_asymmetry`, the
  canonical-ABI lowering of an EXPORT (host calls into guest)
  differs from an IMPORT (guest calls into host). The IME's
  `on-editor-attached(info: editor-info)` lift needs a
  return-area-style hand-written stub on the wasm side. If
  Option A's second `bindgen!` works cleanly, this is hidden
  behind generated code; if Option B (untyped Val) is chosen,
  the IME-side Kotlin still needs hand-rolled lift code for the
  `editor-info` record. Plan extra time in step 1 for whichever
  path is chosen.

- **Layout-pick order matters when both fire.** If
  `attach-editor(number)` arrives BEFORE the IME's first frame
  composition, the state needs to be set early. The inbound
  event lands in the wart-host's drain queue regardless of
  composition state; once the IME guest is instantiated, the
  next drain delivers it. Risk: race between first frame and
  first event — accept "first frame might show English briefly"
  as MVP.

- **Password mode is mostly TODO.** This task just gives Password
  its own layout entry to make the IME show "no suggestions / no
  autocorrect" visually. Real password handling (no clipboard, no
  swipe trails, etc.) is its own task — call it 49b if it grows.

- **Wart-host needs to know which surface is the IME.** Today
  `--standalone-overlay` is the signal. The
  `editor-attached`/`editor-detached` drain arms should only run
  in IME-role hosts; in editor-role hosts they'd be a no-op
  (logged + dropped). The host knows its role via the launch
  flag.

- **Wart-app's hardcoded `inputType="text"` is the MVP hack
  (D7).** Proper plumbing through Compose's
  `PlatformTextInputMethodRequest` is a separate refactor
  worth its own task if a real use case needs it.

## Future expansion (out of scope for task 49)

- **CJK composition support.** Extend `lang` with
  `start-composition` / `add-key(code-point)` /
  `current-composition() -> string` / `commit-composition()`. The
  IME-side surfaces a candidate strip above the keys. WIT contract
  already leaves room.

- **Stage 2: dynamic discovery.** Arbiter exposes
  `list-installed war.lang.*`; the IME builds its lang list at
  startup without manifest declarations. ~2-3 hours of cleanup on
  top of task 49.

- **Layouts as plugins too.** Numeric / Phone / Email could be
  refactored into `war.layout.numeric` etc. Not done in task 49 —
  built-ins are fine until the install surface grows. The plugin
  contract doesn't change.

- **User picker UI in arbiter.** A long-press on 🌐 could open a
  picker showing all installed languages (via Stage 2 discovery).
  Out of scope.

- **Locale-aware default.** Read `persist.sys.locale` (Android
  system locale sysprop) at IME startup and set `userSelectedLang`
  to match. Currently defaults to English. ~30 min when wanted.

## Related

- `tasks/47-ime-via-guest-app.md` step 3c — the multi-surface
  visibility work. Task 49 reuses the parent-container overlay
  surface model and the `attach-editor` / `detach-editor` socket
  flow. Step 4 of task 47 (focus arbitration) is sequential after
  task 49.
- `tasks/48-panel-dim-query.md` — dynamic panel dimensions
  (unrelated; can run in parallel).
- `tasks/36-cross-app-deps.md` + `tasks/39-generic-dep-wiring.md`
  — the dep-loading infrastructure task 49 builds on. No host
  changes needed because of task 39's generic wiring.
- `feedback_wit_bindgen_no_kotlin_generator` — why
  `LangAdapter.kt` is hand-written.
- `feedback_softkeyboard` — the in-canvas keyboard (now retired
  from wart-app's regular path); the layout patterns came from
  there.

## Resume hints for fresh sessions

1. **Read the revision notes at the top.** v1 of this doc
   proposed a new `interface ime-events` and new
   `ime_outbound.rs` file — both redundant. Use existing
   `war:ime/ime` (from `wit/ime.wit` shipped in task 47 step 1)
   and extend `wart-host/src/ime_inbound.rs`.
2. **Step 1 has two sub-layers.** Don't conflate them: 1a is
   socket-level message parsing (~30 min); 1b is wasmtime
   host→guest binding for `war:ime/ime` (~1.5 h). Pick Option A
   (second `bindgen!`) unless it causes build pain.
3. **Hand-written Kotlin canonical-ABI on the EXPORT side** is
   the trickiest unfamiliar bit — see `feedback_canonical_abi_
   import_export_asymmetry` and the markdown-renderer's EXPORT
   pattern for guidance.
4. **The 🌐 cycle vs editor-type override** is the trickiest
   policy bit. D2 above is the lock — write `pickLayout` per
   step 2's recipe and don't drift.
5. **Don't move English out of built-ins.** It's the universal
   fallback when no plugin loads. Bulgarian moving to a plugin is
   the proof-of-concept; English stays put.
6. **wart-app's Number/Phone TextFields are the device smoke
   drivers.** If the demo doesn't have them, add them as part
   of step 2 — otherwise there's nothing to focus that exercises
   the new path. The MVP-hack KeyboardController plumbing (D7)
   only works because TextFieldCard sets `pendingKeyboardType`
   in `.onFocusChanged` before calling `controller.show()`;
   don't reorder.
