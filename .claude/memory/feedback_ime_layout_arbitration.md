---
name: feedback-ime-layout-arbitration
description: war.ime.keyboard picks layouts from two orthogonal sources — editor-type override (host) and user 🌐 cycle — plus a built-in vs plugin split for what the cycle iterates over
metadata: 
  node_type: memory
  type: feedback
  originSessionId: c7a4384f-c3b0-4cdf-98cb-aa514fa75079
---

The IME's `pickLayout` (war.ime.keyboard/src/wasmWasiMain/kotlin/ImeKeyboard.kt)
arbitrates between **two orthogonal layout selectors** and feeds them
from **two orthogonal layout sources**.

**Two selectors:**
- Editor-type override (host-driven): when the focused field is
  `KeyboardType.Number` / `Phone` / `Email` / `Url` / `Password`, the
  host calls `war:ime/ime.on-editor-attached(input-type)` and
  `pickLayout` returns the matching specialized layout. The 🌐 key
  is disabled in this state (cycle would defeat the override).
- User-cycle (guest-driven): when no editor-type override is active
  (or `Text` / `multiline-text`), the 🌐 button cycles
  `userSelectedLang` across all `isLanguage = true` layouts in
  declaration order.

**Two sources:**
- Built-in layouts (compiled into `ImeKeyboardDefaults`): English
  QWERTY, Numeric, Phone, Email, Url, Password, Symbols, Symbols2,
  Emoji. Live in `ImeKeyboard.kt`.
- Plugin layouts (loaded at composition time): `war.lang.*` warpkgs
  each export `war:keyboard-lang-<id>/lang@0.1.0`. The IME calls
  `lang.get-info` + `lang.get-layout(false/true)` for each declared
  plugin, wraps the returned letter rows with the IME's uniform
  digit/shift/utility rows (`ImeKeyboardDefaults.wrapLanguageLayout`),
  and merges them into the 🌐 cycle.

**Why:** keeps host-mandated keyboard shape (no letters when the
field wants a phone number) separate from user-preferred typography
(English vs Bulgarian vs French). Lets app developers ship language
plugins without touching the IME's core.

**How to apply:** when adding a feature that influences which layout
shows, decide which selector it modifies. Adding a new editor-type
override → extend `pickLayout`'s editor-type branch. Adding a new
input language → ship a war.lang.* warpkg + add an entry to
`LangAdapter.plugins` + an import line in `wit/war-ime-keyboard.wit`.
The two paths don't interact.

**Pitfall (task 49 step 5 design discovery):** two deps cannot share
the same WIT package name — wart-host's `wire_dep_into_linker`
collides on the second `linker.instance(name)` call. Each lang
plugin therefore uses its own WIT package
(`war:keyboard-lang-bg`, `war:keyboard-lang-fr`), and the IME
hard-codes its known plugin set. Future polish: host-mediated
dynamic loading where wart-host scans `system-apps/war.lang.*` and
exposes a single `enumerate-langs` / `get-layout(lang-id, ...)` host
verb — plugins truly zero-touch then. Out of scope for the MVP.

Related: [[wit-bindgen-no-kotlin-generator]],
[[wasi-realloc-allocator-pollution]] (LangAdapter's lift helpers
must lead with `freeAllComponentModelReallocAllocatedMemory()`).
