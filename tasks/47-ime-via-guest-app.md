# Task 47 — IME via dedicated guest app (Path B)

> **Status:** 🔲 scoped 2026-05-27, not started. Picked after a
> design analysis comparing three paths (Gboard-via-our-WMS vs
> first-party guest IME vs in-canvas). The in-canvas keyboard
> (`feedback_softkeyboard`) was retired as a long-term direction by
> user decision; Gboard-via-WMS rejected as 3-6 months of work that
> reintroduces ART (breaks the §9 "no Java" lock). This is the
> §9-aligned middle path.

## Why this task exists

Real IME functionality (multi-language input, voice, emoji
picker, autocorrect, swipe typing) is the next user-facing gap
after the Hybrid runtime stack landed in task 46. The Android-
canonical answer (Gboard + IMMS + WMS) is structurally
incompatible with the §9 model (see
[[project-app-lifecycle-and-packaging]] and the discussion that
led to this task).

The architecture is one wart-native IME app per language /
feature, all sharing one WIT contract, all switchable via the
arbiter. First-party `war.ime.keyboard` ships first; voice and
emoji and CJK come as separate `.warpkg`s sharing the same
contract.

Structurally close to Android — every Android IME component has
a wart counterpart (see "Mapping to Android" below) — but uses
our own arbiter + WIT instead of system_server + binder. No ART,
no WMS, no IMMS.

## Mapping to Android (for orientation)

| Android | wart task-47 deliverable |
|---|---|
| IMMS (Java in system_server) | wart-arbiter gains an IME-routing module |
| InputMethodService (Java base class) | `war.ime.keyboard` warpkg — a real wart guest, just another `.warpkg`. Same shape as any Compose app |
| Gboard | First-party `war.ime.keyboard`. Architecture supports N IMEs as `.warpkg`s (voice, emoji, CJK each one) |
| InputMethodManager (per-app Java facade) | `WasiInputMethod` in skiko-wasi — `imm.showSoftInput(...)` style API exposed to guests via the new WIT interface |
| IInputMethodClient + IInputConnection (binder IPC) | WIT `war:ime/client` interface — `commit-text`, `send-key-event`, `set-selection`, `get-text-before-cursor`, etc. Arbiter routes via the generic dep-wiring proxy (task 39) |
| EditorInfo (input type, hint metadata) | Same record passed through the WIT verb |
| WMS focus gate | Arbiter foreground tracking (task 46 step 4) — already shipped |

## Pre-task design decisions

**D1. IME process model: own zygote-forked guest.** Same as wart-
app. Forked from the wart-host zygote, COW-shares preloaded
engine + skia + system bundles. Distinct process, distinct SF
surface, distinct InputDispatcher focus slot.

**D2. WIT, not binder.** The `war:ime/client` WIT interface
replaces Android's `IInputMethodClient` + `IInputConnection`
binders. Calls routed by the arbiter's generic dep-wiring
proxy (task 39). Net win: capability gating becomes the
component's WIT-imports list (Android's `<uses-permission>`
XML drift is gone).

**D3. IME app is a regular `.warpkg`.** No special install path,
no special launch path. The arbiter picks which installed IME
app is "active" via a `set-ime <app-id>` socket command;
default is `war.ime.keyboard`. Switching is just an arbiter
state change.

**D4. First-party + extensible.** `war.ime.keyboard` is shipped
in the wart project — same status as `war.markdown.renderer`
and the other system bundles. Future IMEs (voice, emoji, CJK)
are additional `.warpkg`s, NOT modifications to
`war.ime.keyboard`. Multiple IMEs can be installed; the user
picks the active one.

**D5. Focus mechanic.** Arbiter tracks `(focused_app_pid,
editor_info)` and `(active_ime_pid)`. When the focused app
calls `attach-editor(EditorInfo)`, the arbiter:
  1. Sends `set-visible(true)` + `set-layer(MAX-1)` to the IME's
     SF surface (via the existing libsf_surface entry points).
  2. Tells the IME via WIT that there's an editor attached, with
     the editor's metadata.
  3. Switches InputFlinger focus to the IME's window.
  4. Routes keystrokes/text back to the focused app via WIT.

When the focused app calls `detach-editor` (or loses
foreground), the reverse runs: IME hidden, focus back to the
foreground app's window.

**D6. SF z-order layout.** Three layers from top to bottom:
  - `i32::MAX`     — reserved for future system overlay
  - `i32::MAX - 1` — IME surface
  - `i32::MAX - 2` — foreground app surface
  - `0`            — background app surfaces

The arbiter sets these explicitly via `sf_set_layer` (task 46
step 5 shipped). The IME app starts at MAX-1 + invisible; only
`attach-editor` flips it to visible.

## Steps

### Step 1 — WIT interface + arbiter routing skeleton (~1-2 days)

The protocol shape, no UI yet.

- **`wit/ime-client.wit`** (new) — package `war:ime`, world
  `ime-client-world` (exported by IME apps), world
  `ime-host-world` (imported by editor-bearing apps).
  - `record editor-info { input-type: input-type, hint: string,
    initial-text: string, initial-selection: tuple<u32, u32> }`
  - IME-side EXPORTS: `interface ime`:
    - `on-editor-attached(editor-info)`
    - `on-editor-detached()`
    - `on-app-config-changed(...)` (font scale etc, future)
  - Host-side EXPORTS (imported by IME):
    - `interface input-connection`:
      - `commit-text(text: string)`
      - `send-key-event(code-point: u32, key-id: u32, action: u8)`
      - `set-selection(start: u32, end: u32)`
      - `get-text-before-cursor(max: u32) -> string`
      - `get-text-after-cursor(max: u32) -> string`
      - `finish-composing-text()`

- **Arbiter routing module**: new state `EditorFocus { pid:
  i32, editor_info: EditorInfo }` + `ActiveIme { app_id:
  String, pid: i32 }`. New socket commands:
  - `set-ime <app-id>` — change which IME app is active. Auto-
    launches it if not running yet.
  - `attach-editor <focused-pid> <editor-info-json>` — called
    by the foreground app's host (skiko-wasi) when a text field
    gains focus. Arbiter relays `on-editor-attached` to the IME
    + flips visibility + focus.
  - `detach-editor <focused-pid>` — reverse.
  - `ime-commit-text <text>` — called by the IME app. Arbiter
    relays to the focused app.

- New module `wart-host/src/ime_impl.rs` (already exists for
  task 40 IMMS probes — REPLACE with a new ime_router_impl.rs
  or rename the old one to ime_imms_probe.rs to free the name).

Success criterion: `wart-arbiter attach-editor <pid> '<json>'`
from the shell triggers a logged route in the arbiter to a
dummy IME-pid (no UI yet). Pure protocol smoke.

#### Step 1 results (2026-05-27)

**Outcome:** ✅ all eight new socket commands work end-to-end
on device, state is maintained, error cases return structured
ERR responses. Cross-process delivery is the step-2 add-on; step
1 nails the protocol shape + arbiter-side bookkeeping.

**`wit/ime.wit`** (new) — `package war:ime@0.1.0`. Defines:

- `enum input-type` (text / number / phone / email / url /
  password / multiline-text)
- `enum key-action` (down / up)
- `record editor-info` — input-type + hint + initial-text +
  selection start/end. utf-16 indices to match Android TextView.
- `interface input-connection` — imported by IME apps. Calls:
  `commit-text`, `send-key-event`, `set-composing-text`,
  `finish-composing-text`, `set-selection`,
  `get-text-before-cursor`, `get-text-after-cursor`. The two
  delivery paths (virtual-keyboard scancode via `send-key-event`
  + smart-IME text via `commit-text` / `set-composing-text`)
  both baked in per the user-flagged design discussion — simple
  English keyboards use mostly `send-key-event`, CJK / voice /
  autocorrect IMEs lean on the composing primitives.
- `interface ime` — exported by IME apps. Two methods:
  `on-editor-attached(info)` + `on-editor-detached()`.
- `world ime-client-world` — `import input-connection;
  export ime;`. The shape every IME `.warpkg` implements.

**`wart-arbiter/src/state.rs`** — two new globals:

- `Mutex<Option<ActiveIme>>` (`ActiveIme { app_id, pid }`).
- `Mutex<Option<EditorFocus>>` (`EditorFocus { pid, editor_info }`).

Both accessed via `current_*` getters + `set_*` setters
returning the prior value. `remove(app_id)` now also clears
both if the removed app was the active IME or owned the
focused editor.

**`wart-arbiter/src/main.rs`** — eight new socket-command
handlers + `run_client_multi` for multi-arg CLI passthrough:

- `set-ime <app-id>` — validates the app is running, swaps the
  ActiveIme pointer, returns `OK ime=… pid=… prev=…`. Special
  form `set-ime -` clears.
- `attach-editor <pid> [input-type] [hint] [initial-text]` —
  validates pid is a tracked app, builds an `EditorInfo`,
  swaps the EditorFocus pointer, logs the route intent toward
  the active IME (or "(no active IME)" if unset).
- `detach-editor <pid>` — clears the focus if it matched.
- `ime-commit-text <text>` / `ime-send-key-event <cp> <key-id>
  <action>` / `ime-set-composing-text <text>` /
  `ime-finish-composing-text` / `ime-set-selection <s> <e>` —
  all dispatched via `cmd_ime_route` which validates an editor
  is focused, then logs the routing intent. Step 2 swaps the
  log call for actual cross-process delivery via per-host
  control sockets.
- `cmd_list` extended to print markers: `[fg]` /  `[ime]` /
  `[editor:<input-type>]` per app row.

**Device-verified on Pixel 2 XL:**

```
$ wart-arbiter list                                → OK count=0

$ wart-arbiter launch com.example.wart-app         → pid=5584
$ wart-arbiter launch com.example.wart-app2        → pid=5667

$ wart-arbiter set-ime com.example.wart-app2       → OK ime=… prev=(none)
$ wart-arbiter set-ime com.bogus                   → ERR not-running

$ wart-arbiter attach-editor 5584 text Type-here Hello
  → OK attached editor pid=5584 app=com.example.wart-app input-type=text
       prev-pid=- route→com.example.wart-app2 (pid=5667)
$ wart-arbiter attach-editor 99999                 → ERR attach-editor-unknown-pid

$ wart-arbiter ime-commit-text hello               → OK route→pid=5584 …
$ wart-arbiter ime-send-key-event 0 67 down        → OK route→pid=5584 …
$ wart-arbiter ime-set-composing-text ni           → OK route→pid=5584 …
$ wart-arbiter ime-finish-composing-text           → OK route→pid=5584 …

$ wart-arbiter list
  OK count=2
    app=com.example.wart-app   pid=5584 elapsed=…  [editor:text]
    app=com.example.wart-app2  pid=5667 elapsed=…  [fg] [ime]

$ wart-arbiter detach-editor 5584                  → OK detached …
$ wart-arbiter ime-commit-text orphan              → ERR no-focused-editor
$ wart-arbiter set-ime -                           → OK cleared prev=com.example.wart-app2
```

Logcat captures structured `arbiter: ime-<verb> → editor pid=X
app-input-type=Y args=…` lines that step 2 replaces with actual
delivery once per-host control sockets exist.

**Files added/changed (this step):**

- `wit/ime.wit` — new protocol definition
- `wart-arbiter/src/state.rs` — `ActiveIme`, `EditorInfo`,
  `EditorFocus` types + getters/setters; `remove` clears both
  fields on app death
- `wart-arbiter/src/main.rs` — eight new socket commands +
  `run_client_multi` CLI passthrough + `cmd_list` markers
- `tasks/47-ime-via-guest-app.md` — this section

**Out of scope for step 1 (lands later):**

- Cross-process delivery — per-host control sockets for the
  zygote + arbiter to push commands INTO running children
  (step 2). Today's "route intent" logging will become an
  actual write to the focused-app's control socket.
- `editor-info` JSON over the wire — current CLI uses
  positional args (space-separated, no embedded spaces in
  hint/text). Step 2's skiko-driven path doesn't go through
  the CLI so it can use a richer serialization.
- Persistence — `EditorFocus` is editor-lifecycle-scoped (gone
  on app exit anyway); `ActiveIme` could survive an arbiter
  restart via the existing crash-marker — left for a follow-up
  once the IME app is real.

### Step 2 — Skiko-wasi WasiInputMethod adapter (~2 days)

Wires the focused-app side. When Compose's `BasicTextField`
gains focus inside a guest, it triggers
`PlatformTextInputMethodRequest.startInputMethod(...)`. The
skiko-wasi actual for this calls the new WIT verb to
`attach-editor` via the arbiter.

- New `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/wasi/WasiInputMethod.kt`
  — Compose `PlatformTextInputMethodRequest` actual.
- New WIT imports on the wart-app side: the input-connection
  interface (host-driven calls from the IME → guest's editor).
- `compose-multiplatform-core/.../wasmWasiMain/...` actuals for
  the `PlatformTextInputMethodRequest` extension points.
- Replace `WasiSoftKeyboard` registration in wart-app with the
  new external-IME path; the in-canvas keyboard's code stays in
  tree as the fallback for when no IME app is installed.

Success criterion: tapping a `BasicTextField` in wart-app logs
`attach-editor` reaching the arbiter; tapping out logs
`detach-editor`. No UI swap yet — keyboard is still in-canvas.

#### Step 2 results (2026-05-27)

**Outcome:** ✅ end-to-end on Pixel 2 XL. Tapping a `BasicTextField`
forwards `attach-editor pid=N input-type=text ...` to the arbiter
via a fresh UNIX socket connection per call; tapping out forwards
`detach-editor pid=N`. In-canvas keyboard still draws (the
co-existence promise from the scope doc was kept).

**WIT extension** — added an `ime` interface to
`wit/skiko-gfx.wit` (mirrored to `skiko/skiko/wit/skiko-gfx.wit`
and `wart-app/wit/deps/skiko-gfx/skiko-gfx.wit`) and added it
to the `skiko-ui` world's imports:

```wit
interface ime {
    notify-editor-attached: func(
        input-type: string, hint: string, initial-text: string,
        selection-start: u32, selection-end: u32,
    );
    notify-editor-detached: func();
}
```

`input-type` is stringly-typed at this layer (matches the values
in `wit/ime.wit`'s enum) to avoid a cross-package import; if
more typed IME verbs land on this interface, the right move is
to promote `war:ime/types` to its own importable package.

**Host impl** — new
`wart-host/src/ime_host_impl.rs` implements
`my::skiko_gfx::ime::Host for HostState`:

- `notify_editor_attached` opens a one-shot UNIX socket to
  `/data/local/tmp/wart-arbiter.sock`, writes
  `attach-editor <getpid()> <input-type> <hint> <initial-text>\n`
  (spaces in hint/initial-text replaced with `_` for the
  positional CLI parser — step 4's per-host control socket can
  use a richer wire format).
- `notify_editor_detached` writes `detach-editor <pid>\n`.
- Failures log + degrade gracefully — arbiter-down doesn't
  break the guest's focus path.

**Skiko-wasi bindings** — hand-added the `Ime` interface to
`skiko/skiko/src/wasmWasiMain/kotlin/generated/{Internal,}SkikoUi.kt`
following the `logMessage` / `Haptics.perform` pattern: two
`@WasmImport` external declarations
(`__wasm_import_ime_notifyEditorAttached`,
`__wasm_import_ime_notifyEditorDetached`) plus a public
`Ime.Import` companion. Three strings flatten to 6 ints
(ptr, len each); 2 u32 selection bounds → 8 ints total in the
attach signature.

**Wart-app integration** — `WasiKeyboardController.show()` /
`.hide()` now ALSO call `Ime.Import.notifyEditorAttached(...)`
/ `notifyEditorDetached()` alongside flipping
`isVisible.value` (which still drives the in-canvas keyboard).
Try/catch defensive — if the arbiter is down, the in-canvas
keyboard still works.

**Build pipeline gotcha caught + fixed during smoke** —
wart-app's `package.toml` (in `scripts/build-system-warpkgs.sh`)
was missing `[dependencies]` declarations for the three system
bundles (markdown/emoji/fonts). The installer auto-detects
component imports for the "missing dep — refuse install" gate
but DOES NOT populate `[dependencies_resolved]` in
cache-key.toml without an explicit `[dependencies]` block — so
the loader's `load_dep_components` walked an empty table and
instantiation failed with "war:markdown/renderer.render has
the wrong type: function implementation is missing". The fix is
mechanical: declare the three deps in the script's manifest
template (same shape as md-smoke-rust does it).

Also fixed a stdout-pollution bug in the same script: the
`build_system_wasm` function's `echo` lines were going to stdout
and getting captured into `MD_WASM=$(build_system_wasm ...)`
along with the path. Redirected the diagnostics to stderr.

**Device-verified smoke transcript** (Pixel 2 XL, freshly-
rebuilt + reinstalled stack):

```
$ wart-arbiter launch com.example.wart-app
  OK pid=7265 app=com.example.wart-app
  log: loader: loaded dep `fonts` (war:fonts/loader@0.1.0) from …
  log: loader: loaded dep `markdown` (war:markdown/renderer@0.1.0) from …
  log: loader: dep `emoji` instantiated; wired 1 fn(s) across 1 interface(s)
  log: loader: dep `fonts` instantiated; wired 2 fn(s) across 1 interface(s)
  log: loader: dep `markdown` instantiated; wired 1 fn(s) across 1 interface(s)
  log: standalone: rendered frame 1, 2, 3, …

# tap on the BasicTextField in TextFieldCard …
  log: ime-host: forwarded attach-editor pid=7265 input-type="text" hint-len=0
       text-len=0 selection=[0..0]
  log: arbiter: attach-editor pid=7265 app=com.example.wart-app input-type=text
       hint="" initial-text-len=0 → route to (no active IME — set-ime first)
       (step 2 delivers on-editor-attached)

# tap outside …
  log: ime-host: forwarded detach-editor pid=7265
  log: arbiter: detach-editor pid=7265 → route to (no active IME)
       (step 2 delivers on-editor-detached)
```

Step 2 success criterion met by construction:
"tapping a `BasicTextField` in wart-app logs `attach-editor`
reaching the arbiter; tapping out logs `detach-editor`."

**Out of scope** (lands in step 3 — the actual IME app):

- Inbound delivery — IME app's `commit-text` / `send-key-event`
  arriving in the focused guest's editor. Needs a per-host
  control socket (wart-host child opens connection to the
  arbiter at startup; arbiter pushes events down it). Will
  land in step 3 alongside the first-party `war.ime.keyboard`
  warpkg, since the two pieces are needed together to actually
  type a character end-to-end.
- Real `EditorInfo` fields. wart-app's `WasiKeyboardController`
  hard-codes `input-type="text"`, `hint=""`, `initial-text=""`,
  `selection=[0..0]`. Threading the BasicTextField's actual
  `KeyboardOptions.keyboardType` + the textfield's current
  text/selection through Compose's
  `PlatformTextInputMethodRequest` is a separate refactor.

**Files added/changed (this step):**

- `wit/skiko-gfx.wit` + mirrors — new `ime` interface added
  to skiko-ui imports.
- `wart-host/src/ime_host_impl.rs` (new) — WIT impl that
  forwards to the arbiter socket.
- `wart-host/src/lib.rs` — `mod ime_host_impl;`.
- `skiko/skiko/src/wasmWasiMain/kotlin/generated/{Internal,}SkikoUi.kt`
  — hand-added `Ime` bindings.
- `wart-app/src/wasmWasiMain/kotlin/WasiKeyboardController.kt`
  — `show()` / `hide()` also call the new WIT verb.
- `scripts/build-system-warpkgs.sh` — declares wart-app's
  cross-app deps in its package.toml template + redirects
  build_system_wasm's diagnostics to stderr.
- `tasks/47-ime-via-guest-app.md` — this section.

### Step 3 — `war.ime.keyboard` first-party warpkg (~1 week)

The actual keyboard UI. New repo `war.ime.keyboard/` (sibling
to `wart-arbiter` / `markdown-renderer` / etc).

- Compose Material3 UI, fullwidth at the bottom of the screen.
- QWERTY layout, shift/caps lock, basic punctuation, numbers
  via secondary layer, backspace, enter, space.
- WIT imports: `war:ime/input-connection` (calls `commit-text`,
  `send-key-event`, etc).
- WIT exports: `war:ime/ime` (receives `on-editor-attached`,
  shows the keyboard; `on-editor-detached`, hides).
- Build pipeline: same as wart-app (Kotlin/Compose → wasm →
  component embed → component new → .warpkg).
- Package: `app_id = "war.ime.keyboard"`, `kind = "system"`
  (it's the IME-ecosystem equivalent of the markdown/emoji/fonts
  system bundles), `version = "0.1.0"`.

Success criterion: tap text field → keyboard renders + accepts
taps → tapped letters appear in the focused TextField via
`commit-text` round-trip.

#### Step 3a — inbound delivery infra (2026-05-27)

Split step 3 into two sub-steps for tractable commits. **Step 3a
ships the protocol path that delivers events from the arbiter
INTO a running guest** — the missing inbound half of the IME
loop. Step 3b is the actual `war.ime.keyboard` Compose UI, which
sits on top of this infra and can iterate independently.

**Architecture decision**: route IME taps through the existing
**`send-key-event` virtual-hardware-keyboard path** (task 33
step 3's `on-key-event-v2` + `dispatch_key_v2`) instead of
building new WIT exports for `commit-text` /
`set-composing-text`. The user's earlier-this-session intuition
about a virtual-keyboard service was correct for ASCII typing —
that's what step 3a uses. `commit-text` / `set-composing-text`
become future work (needed for autocorrect, CJK, emoji); the
keyboard MVP doesn't need them.

**`wart-host/src/ime_inbound.rs` (new)**:

```
accept thread (background)         render loop (main thread)
───────────────────────             ───────────────────────────
UnixListener::accept()              ── per frame ──
read lines                          drain_queue() → Vec<InboundEvent>
parse `key-event <cp> <kid> <act>`  for each event:
queue.push_back(KeyEvent {...})       dispatch_key_v2(skiko, store, …)
                                    (re-uses task 33 step 3)
```

Per-host socket path: `/data/local/tmp/wart-host-<pid>.sock`.
Bound by the forked child after EGL is up. Mode 666. The
arbiter derives the path from `EditorFocus.pid` — no
registration handshake needed.

Why two threads: wasmtime's `Store` is `!Send`, so only the
render-loop thread can call into the wasm guest. The accept
thread just parses + queues; the render loop drains +
dispatches. Same separation the InputFlinger drain uses.

**`wart-host/src/standalone.rs`**:

- After EGL setup: `crate::ime_inbound::spawn_listener()`.
- Per-frame, alongside the InputFlinger drain:
  `for ev in crate::ime_inbound::drain_queue() { ...
  dispatch_key_v2(skiko, &mut store, action, code_point, key_id) ... }`

**`wart-arbiter/src/main.rs`** — `cmd_ime_route` for the
`send-key-event` verb now actually delivers:

```rust
let host_sock = format!("/data/local/tmp/wart-host-{}.sock", focus.pid);
let line = format!("key-event {cp} {kid} {act}\n");
deliver_to_host(&host_sock, &line)?;
```

`deliver_to_host` is a one-shot connect (open + write + shutdown
+ read-drain + close), matching the existing pattern. Other
ime-* verbs (commit-text / set-composing-text /
finish-composing-text / set-selection) still log only — they
land in step 3b once we have the editor-side WIT exports.

**Device-verified end-to-end** on Pixel 2 XL — full round-trip
from shell-injected `ime-send-key-event` to BasicTextField
state mutation:

```
$ wart-arbiter launch com.example.wart-app  → OK pid=7989
  log: standalone: ime-inbound listening on
       /data/local/tmp/wart-host-7989.sock

# (user taps the TextField in TextFieldCard)
  log: arbiter: attach-editor pid=7989 app=com.example.wart-app
       input-type=text … → route to (no active IME — set-ime first)

# inject 'a' (code-point 97 = 'a', key-id 29 = AKEYCODE_A)
$ wart-arbiter ime-send-key-event 97 29 down  → OK route→pid=7989
$ wart-arbiter ime-send-key-event 97 29 up    → OK route→pid=7989
  log: arbiter: ime-send-key-event → pid=7989 (97 29 down)
       delivered via /data/local/tmp/wart-host-7989.sock
  log: [wasm] tfstate text="hello worlda" sel=TextRange(12, 12)
                              ^^^^^^^^^^^^
                              the 'a' arrived in the BasicTextField

# inject 'b' then 'c'
  log: [wasm] tfstate text="hello worldab" sel=TextRange(13, 13)
  log: [wasm] tfstate text="hello worldabc" sel=TextRange(14, 14)
```

**Out of scope for step 3a** (lands in step 3b):

- `commit-text` / `set-composing-text` / `finish-composing-text`
  /  `set-selection` delivery. Needs new WIT exports on the
  editor-bearing-app side (new `on-commit-text` etc. methods on
  the renderer interface, or a new exported interface) +
  Compose-side handling that mutates the focused TextFieldState.
  Step 3a's send-key-event-only path is sufficient for ASCII
  typing.
- The `war.ime.keyboard` warpkg itself. Now unblocked — the
  IME UI just needs to call `Ime.Import.notifyEditorAttached`
  / `sendKeyEvent` style WIT verbs (which the IME-side adapter
  routes back to the arbiter's `ime-send-key-event` socket
  command).
- Backspace: AKEYCODE_DEL (67) round-tripped at the wire layer
  but didn't mutate the TextField in this smoke. Likely Compose's
  BasicTextField handling of code-point=0 + key-id=67 vs how
  hardware-keyboard backspace is currently delivered — out of
  step-3a scope; will be debugged when the keyboard app needs it.

**Files added/changed (this step):**

- `wart-host/src/ime_inbound.rs` — new module: per-host socket
  listener thread + queue + drain.
- `wart-host/src/lib.rs` — `mod ime_inbound;` (android-only).
- `wart-host/src/standalone.rs` — spawn_listener after EGL +
  per-frame drain calling `dispatch_key_v2`.
- `wart-arbiter/src/main.rs` — `cmd_ime_route` for
  send-key-event actually delivers; added `deliver_to_host`
  one-shot helper.
- `tasks/47-ime-via-guest-app.md` — this section.

### Step 4 — InputFlinger focus arbitration + auto-hide (~2-3 days)

The arbiter coordinates input routing between focused app and
IME. Touches in the keyboard surface dispatch to the IME's
process; touches outside (in the app's surface) dispatch back
to the app. Auto-hide when the user taps outside the keyboard.

- Arbiter calls the IME child's signal-driven SF-focus-request
  routine when `attach-editor`. Detach swaps focus back to the
  focused app's window.
- IME-side render-loop reads its own role (focused/unfocused)
  via the same `app_role` signal mechanism task 46 step 4 uses
  — IME is "foreground" while editor is attached.
- The focused app stays in foreground (not paused) while the
  IME is up — it needs to render the cursor moving as text
  arrives.

Success criterion: type text into wart-app via the new IME;
tap outside the keyboard → keyboard hides + focus returns to
wart-app; tap the text field again → keyboard back. No focus
flapping.

### Step 5 — Polish + future-IME framing (~2-3 days)

- Done/Submit key handling (return key dispatches `keyDone`
  callback to the focused field if the editor-info specifies
  IME action).
- Backspace / arrow keys via `send-key-event`.
- Soft-hide on Compose `clearFocus()`.
- Document the future-IME interface stability + `set-ime
  <app-id>` mechanism in the task doc and a new memory
  `feedback_ime_via_guest_app`.

Success criterion: end-to-end typing experience indistinguishable
from a stock Android device for English ASCII. CJK/voice/emoji
are pluggable but not present yet.

## Future IMEs — voice / emoji / CJK

The `war:ime/client` WIT contract is the stability gate. Each
future IME is:

- **`war.ime.voice`** — Compose UI is a microphone button +
  transcription preview. On press: starts an audio recording
  via our audio HAL (task 21 IAAudioService); transcription is
  *another* future system bundle (speech-recognition WIT
  service). When transcription completes, `commit-text(result)`.
  Implementation cost: low for the IME shell, high for the
  recognition. Recognition could initially be a thin shim over
  Android's `SpeechRecognizer` via rsbinder (uses the same
  no-Java path as our other HAL access), or wasm-side via a
  small whisper.cpp port. Multi-pass effort, but the IME UI
  itself is ~200 lines of Compose.

- **`war.ime.emoji`** — reuses `war.emoji.picker` (task 40,
  already a system bundle). Compose UI is the emoji grid
  itself (`EmojiCard.kt` already implemented in wart-app — the
  rendering code carries straight over). Tap → `commit-text(emoji_codepoint)`.
  Implementation cost: small. Could be ~1 week.

- **`war.ime.zh`** / `war.ime.ja` / `war.ime.ko` / `war.ime.in.*` —
  CJK / Indic IMEs. The WIT side is unchanged; the hard part
  is the input-method dictionary + composition (pinyin →
  candidates, kana → kanji conversion). Could ship as one
  warpkg per language, or one multi-language warpkg with a
  user-selected mode. Per-IME effort is months, but the
  arbiter/protocol side doesn't change.

The crucial property: **each future IME is a vendor-independent
`.warpkg`**, installable via the existing warpkg installer (task
35), discoverable via the arbiter's `list-imes` command (TBD,
introspects `<APPS_ROOT>/apps/*` for app_ids starting with
`war.ime.`), switchable at runtime. The Android-equivalent
distinction between "default keyboard" (Settings → Languages
& Input) and "active IME" carries directly.

## Known unknowns

- **Compose `PlatformTextInputMethodRequest` shape on wasi**.
  The Android actual is heavily JNI-coupled to AndroidInputMethodManager;
  we'd ship a different actual entirely. Investigation in step 2.
- **`send-key-event` semantics for non-letter keys**. Backspace,
  enter, arrows — Compose treats these differently from `commit-text`.
  Probably want a richer enum than just code-point.
- **IME composition state**. CJK input has a "composing region"
  underline that updates as the user types pinyin. WIT `commit-text`
  is too coarse; need a `set-composing-text(text, region)`
  primitive. Step 1 of this task should bake this into the WIT
  even though step 3's English-only keyboard doesn't exercise it.
- **Focus arbitration race**. If the user taps app → IME → app
  fast, do we end up in a consistent state? Need to think about
  the signal ordering. Possibly the arbiter should batch
  transitions through a state machine rather than firing them
  immediately.
- **What happens when the IME app crashes**. Should the foreground
  app keep its editor "attached" and re-attach when a new IME
  app spawns? Or detach? Probably detach + log + maybe
  auto-relaunch the IME after a backoff. Use the crash-marker
  state to track.

## File-touch map

- `wit/ime.wit` (new) — the protocol.
- `wart-arbiter/src/main.rs` — new `cmd_attach_editor`,
  `cmd_detach_editor`, `cmd_set_ime`, `cmd_ime_commit_text`.
- `wart-arbiter/src/state.rs` — `EditorFocus`, `ActiveIme`
  state.
- `wart-host/src/ime_imms_probe.rs` (rename from `ime_impl.rs`)
  — the task 40 probe code, kept for historical reference.
- `wart-host/src/ime_router_impl.rs` (new) — host side of the
  WIT plumbing for the focused-app's `attach-editor` outbound
  + `commit-text` inbound.
- `wart-host/src/lib.rs` — `pub mod ime_router_impl;`.
- `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/wasi/WasiInputMethod.kt`
  (new) — skiko-side actual.
- `compose-multiplatform-core/.../wasmWasiMain/.../PlatformTextInputMethodRequest.wasi.kt`
  (new or update) — Compose actual.
- `war.ime.keyboard/` (new sibling repo) — the IME guest itself.
- `scripts/build-system-warpkgs.sh` — packages `war.ime.keyboard`
  alongside markdown/emoji/fonts.
- `tasks/47-ime-via-guest-app.md` — this doc; update per-step.
- `CLAUDE.md` — status table row.
- `MEMORY.md` →
  [[feedback_ime_via_guest_app]] — new memory capturing the
  resolved design + the path-A/path-B comparison.

## Resume hints for fresh sessions

1. `cat .task-state` — TASK=47 STEP=N tells you where to pick
   up.
2. Read **D1-D6** above before doing anything else — most
   early failures will be due to violating one (especially
   D2: don't add binder).
3. **Step order is load-bearing**: step 1's WIT shape constrains
   everything else. Land it cleanly with composing-text +
   set-composing-text + finish-composing-text primitives even
   though step 3's English keyboard doesn't exercise them.
4. The first-party `war.ime.keyboard` is the GROUND TRUTH for
   the IME-side of the WIT contract. If a contract change
   makes it impossible to write a sensible keyboard, the
   contract is wrong; iterate on the WIT.
5. Voice / emoji / CJK are explicit out-of-scope for this
   task — they're future `.warpkg`s, not modifications to
   `war.ime.keyboard`. The contract should support them
   without changes.

## Related

- `tasks/40-real-ime.md` — the abandoned IMMS-via-rsbinder
  path. Code (commit `09782f5` in wart-host, the AIDL probes
  in `src/ime_impl.rs`) kept in tree for historical reference.
- `tasks/44-wms-window-registration.md` — the abandoned
  vendor-WMS path. Same reason.
- `tasks/46-wart-arbiter-mvp.md` — the arbiter machinery this
  task extends. Specifically step 4 (app_role signaling, SF
  z-order via libsf_surface, oom_score_adj) and the crash-marker
  state persistence are reused.
- `MEMORY.md` → [[project-app-lifecycle-and-packaging]] — the
  §9 lock this task aligns with.
- `MEMORY.md` → [[project-ime-options]] — the original IME-path
  analysis (A: rsbinder→IMMS, B: NativeActivity wrapper, C:
  wasi-guest IME). Path C IS this task; the post-ART north
  star turned out to be reachable directly without C being
  literally last.
- `MEMORY.md` → [[feedback_softkeyboard]] — the in-canvas
  keyboard. Retired as long-term direction by user decision
  2026-05-27; the code stays in tree as the no-installed-IME
  fallback (a guest with no IME app installed still gets to
  type via in-canvas; better than nothing).
