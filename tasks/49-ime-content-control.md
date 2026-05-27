# Task 49 — IME content control: editor-driven layouts + language plugins

> **Status:** 🔲 scoped 2026-05-27, not started. Two follow-ups to task 47
> step 3c that share the same inbound-delivery infrastructure:
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
> both depend on a new arbiter → IME control socket (a mirror of the
> existing editor-side `ime_inbound.rs`).

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

**D6. Inbound IME control socket lives at
`/data/local/tmp/wart-host-ime-<pid>.sock`** (or equivalent under
the active IME's pid). The arbiter writes one-line text messages
the same way it does to wart-host-<editor-pid>.sock today.

**D7. wart-app's hardcoded `input-type="text"` becomes real.** The
`WasiKeyboardController.show()` reads the focused field's actual
`KeyboardOptions.keyboardType` and maps to the WIT enum. This is
the only Compose-side change.

## Steps

### Step 1 — Inbound delivery infrastructure (~1.5 h)

The piece both features need. Mirrors the editor-side
`ime_inbound.rs` shipped in step 3a but pointed at the IME process.

- **`wart-host/src/ime_outbound.rs`** (new) — per-IME-host control
  socket listener.
  - `spawn_listener()` opens `/data/local/tmp/wart-host-ime-<pid>.sock`
    on the IME side, accepts one connection at a time, parses
    one-line messages.
  - Parsed wire format:
    - `editor-attached <input-type> <hint> <initial-text-len> <selection-start> <selection-end>`
    - `editor-detached`
    - (Future: `request-hide`, `config-changed`, …)
  - Queues `InboundImeEvent::EditorAttached { info }` /
    `InboundImeEvent::EditorDetached`.
  - `drain_queue() -> Vec<InboundImeEvent>` for the render loop.

- **`wart-host/src/standalone.rs`** — when running as an overlay,
  call `ime_outbound::spawn_listener()` after EGL setup. Per-frame
  drain calls into the IME guest's new exported `ime-events`
  interface.

- **`wit/skiko-gfx.wit`** — extend `keyboard` (or add new
  `ime-events`) with EXPORTED functions the IME guest implements:
  ```wit
  interface ime-events {
      record editor-info {
          input-type: string,   // "text" / "number" / "phone" / ...
          hint: string,
          initial-text: string,
          selection-start: u32,
          selection-end: u32,
      }
      on-editor-attached: func(info: editor-info);
      on-editor-detached: func();
  }
  ```
  Stringly-typed `input-type` to avoid a cross-package WIT import
  (matches the pattern from step 2's `ime` interface). Mirror to
  the usual 3 sites.

- **`wart-host/src/keyboard_host_impl.rs`** (or new
  `ime_events_host_impl.rs`): no changes needed — the host doesn't
  IMPLEMENT these, it CALLS them. The Host-side call happens from
  `standalone.rs`'s drain via the wasmtime binding.

- **`wart-arbiter/src/main.rs`** — `cmd_attach_editor` now also
  writes `editor-attached ...` to the active IME's per-host socket
  if one exists. `cmd_detach_editor` writes `editor-detached`.

**Success criterion:** `wart-arbiter attach-editor <pid> number` →
`adb logcat` shows the IME's `on-editor-attached` Kotlin handler
fired with `input-type="number"`.

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
  the `show()` call passes the focused field's actual
  `KeyboardOptions.keyboardType`:
  ```kotlin
  fun show(field: WasiTextField) {
      val type = when (field.keyboardOptions.keyboardType) {
          KeyboardType.Number     -> "number"
          KeyboardType.Phone      -> "phone"
          KeyboardType.Email      -> "email"
          KeyboardType.Uri        -> "url"
          KeyboardType.Password   -> "password"
          KeyboardType.NumberPassword -> "password"
          else -> "text"
      }
      Ime.Import.notifyEditorAttached(type, hint, initialText, selStart, selEnd)
  }
  ```
  + add at least one Number TextField to wart-app's demo so the
  path is exercised on the smoke.

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
| **wart** | `wit/keyboard-lang.wit` (new) | Plugin contract |
| **wart** | `wit/skiko-gfx.wit` | New `ime-events` interface (EXPORT from IME) |
| **wart** | `tasks/49-ime-content-control.md` | This file, with results section per step |
| **wart** | `scripts/build-system-warpkgs.sh` | Build / package the new lang warpkgs |
| **wart-host** | `src/ime_outbound.rs` (new) | Per-IME-host control socket |
| **wart-host** | `src/standalone.rs` | spawn_listener + per-frame drain when overlay mode |
| **wart-host** | `src/lib.rs` | `mod ime_outbound;` |
| **wart-arbiter** | `src/main.rs` | `cmd_attach_editor` / `cmd_detach_editor` writes to IME socket |
| **wart-app** | `src/wasmWasiMain/kotlin/WasiKeyboardController.kt` | Read real keyboardType |
| **wart-app** | demo screens get a `Number` and `Phone` TextField | Smoke coverage |
| **wart-app** | `wit/deps/skiko-gfx/skiko-gfx.wit` | Mirror |
| **skiko** | `skiko/wit/skiko-gfx.wit` | Mirror |
| **skiko** | `.../generated/{Internal,}SkikoUi.kt` | `ime-events` Kotlin bindings (hand-added) |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/ImeKeyboard.kt` | New built-in layouts + edit-type pickLayout |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/LangAdapter.kt` (new) | Hand-written WIT bindings + loader |
| **war.ime.keyboard** | `src/wasmWasiMain/kotlin/RealComposeApp.kt` | Wire `on-editor-attached` export |
| **war.ime.keyboard** | `package.toml` | `[dependencies]` for langs |
| **war.ime.keyboard** | `wit/war-ime-keyboard.wit` | Add `import keyboard-lang;` and `export ime-events;` |
| **war.ime.keyboard** | `wit/deps/keyboard-lang/keyboard-lang.wit` (new) | Mirror |
| **war.ime.keyboard** | `wit/deps/skiko-gfx/skiko-gfx.wit` | Mirror |
| **war.lang.bg** (new repo) | full Rust cdylib | Bulgarian, extracted from built-ins |
| **war.lang.fr** (new repo) | full Rust cdylib | French AZERTY |

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

- **Layout-pick order matters when both fire.** If
  `attach-editor(number)` arrives BEFORE the IME's first frame
  composition, the state needs to be set early. Use a top-level
  `LaunchedEffect(Unit)` to install the inbound-event observer
  before the first `pickLayout` call. Risk: race between first
  frame and first event — accept "first frame might show English
  briefly" as MVP; if jarring, gate first composition on a
  Channel.receive.

- **Password mode is mostly TODO.** This task just gives Password
  its own layout entry to make the IME show "no suggestions / no
  autocorrect" visually. Real password handling (no clipboard, no
  swipe trails, etc.) is its own task — call it 49b if it grows.

- **Wart-host needs to know which surface is the IME.** Today
  `--standalone-overlay` is the signal. The new `ime_outbound`
  socket should only spawn when running as an overlay child.

- **Two distinct per-host sockets per IME process.** wart-host
  already opens `/data/local/tmp/wart-host-<pid>.sock` for the
  editor-side inbound (step 3a). The new IME-side inbound is
  `wart-host-ime-<pid>.sock`. Distinct names so an app that's
  both an editor AND an IME (theoretical — no use case yet) can
  open both.

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

1. **Inbound socket is the keystone.** Land step 1 first; both
   features fail without it. Steps 2-5 can land in either order
   afterward.
2. **Hand-written Kotlin WIT bindings for `lang`** — model on
   `wart-app/src/wasmWasiMain/kotlin/MarkdownImports.kt`. Same
   shape (records with strings in lists).
3. **The 🌐 cycle vs editor-type override** is the trickiest
   policy bit. D2 above is the lock — write `pickLayout` per
   step 2's recipe and don't drift.
4. **Don't move English out of built-ins.** It's the universal
   fallback when no plugin loads. Bulgarian moving to a plugin is
   the proof-of-concept; English stays put.
5. **wart-app's number TextField is the device smoke driver.** If
   the demo doesn't have one, add it as part of step 2 — otherwise
   there's nothing to focus that exercises the new path.
