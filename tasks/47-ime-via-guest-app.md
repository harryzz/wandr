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

The architecture is one wandr-native IME app per language /
feature, all sharing one WIT contract, all switchable via the
arbiter. First-party `wandr.ime.keyboard` ships first; voice and
emoji and CJK come as separate `.wandrpkg`s sharing the same
contract.

Structurally close to Android — every Android IME component has
a wandr counterpart (see "Mapping to Android" below) — but uses
our own arbiter + WIT instead of system_server + binder. No ART,
no WMS, no IMMS.

## Mapping to Android (for orientation)

| Android | wandr task-47 deliverable |
|---|---|
| IMMS (Java in system_server) | wandr-arbiter gains an IME-routing module |
| InputMethodService (Java base class) | `wandr.ime.keyboard` wandrpkg — a real wandr guest, just another `.wandrpkg`. Same shape as any Compose app |
| Gboard | First-party `wandr.ime.keyboard`. Architecture supports N IMEs as `.wandrpkg`s (voice, emoji, CJK each one) |
| InputMethodManager (per-app Java facade) | `WasiInputMethod` in skiko-wasi — `imm.showSoftInput(...)` style API exposed to guests via the new WIT interface |
| IInputMethodClient + IInputConnection (binder IPC) | WIT `wandr:ime/client` interface — `commit-text`, `send-key-event`, `set-selection`, `get-text-before-cursor`, etc. Arbiter routes via the generic dep-wiring proxy (task 39) |
| EditorInfo (input type, hint metadata) | Same record passed through the WIT verb |
| WMS focus gate | Arbiter foreground tracking (task 46 step 4) — already shipped |

## Pre-task design decisions

**D1. IME process model: own zygote-forked guest.** Same as wandr-
app. Forked from the wandr-host zygote, COW-shares preloaded
engine + skia + system bundles. Distinct process, distinct SF
surface, distinct InputDispatcher focus slot.

**D2. WIT, not binder.** The `wandr:ime/client` WIT interface
replaces Android's `IInputMethodClient` + `IInputConnection`
binders. Calls routed by the arbiter's generic dep-wiring
proxy (task 39). Net win: capability gating becomes the
component's WIT-imports list (Android's `<uses-permission>`
XML drift is gone).

**D3. IME app is a regular `.wandrpkg`.** No special install path,
no special launch path. The arbiter picks which installed IME
app is "active" via a `set-ime <app-id>` socket command;
default is `wandr.ime.keyboard`. Switching is just an arbiter
state change.

**D4. First-party + extensible.** `wandr.ime.keyboard` is shipped
in the wandr project — same status as `wandr.markdown.renderer`
and the other system bundles. Future IMEs (voice, emoji, CJK)
are additional `.wandrpkg`s, NOT modifications to
`wandr.ime.keyboard`. Multiple IMEs can be installed; the user
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

- **`wit/ime-client.wit`** (new) — package `wandr:ime`, world
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

- New module `wandr-host/src/ime_impl.rs` (already exists for
  task 40 IMMS probes — REPLACE with a new ime_router_impl.rs
  or rename the old one to ime_imms_probe.rs to free the name).

Success criterion: `wandr-arbiter attach-editor <pid> '<json>'`
from the shell triggers a logged route in the arbiter to a
dummy IME-pid (no UI yet). Pure protocol smoke.

#### Step 1 results (2026-05-27)

**Outcome:** ✅ all eight new socket commands work end-to-end
on device, state is maintained, error cases return structured
ERR responses. Cross-process delivery is the step-2 add-on; step
1 nails the protocol shape + arbiter-side bookkeeping.

**`wit/ime.wit`** (new) — `package wandr:ime@0.1.0`. Defines:

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
  export ime;`. The shape every IME `.wandrpkg` implements.

**`wandr-arbiter/src/state.rs`** — two new globals:

- `Mutex<Option<ActiveIme>>` (`ActiveIme { app_id, pid }`).
- `Mutex<Option<EditorFocus>>` (`EditorFocus { pid, editor_info }`).

Both accessed via `current_*` getters + `set_*` setters
returning the prior value. `remove(app_id)` now also clears
both if the removed app was the active IME or owned the
focused editor.

**`wandr-arbiter/src/main.rs`** — eight new socket-command
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
$ wandr-arbiter list                                → OK count=0

$ wandr-arbiter launch com.example.wandr-app         → pid=5584
$ wandr-arbiter launch com.example.wandr-app2        → pid=5667

$ wandr-arbiter set-ime com.example.wandr-app2       → OK ime=… prev=(none)
$ wandr-arbiter set-ime com.bogus                   → ERR not-running

$ wandr-arbiter attach-editor 5584 text Type-here Hello
  → OK attached editor pid=5584 app=com.example.wandr-app input-type=text
       prev-pid=- route→com.example.wandr-app2 (pid=5667)
$ wandr-arbiter attach-editor 99999                 → ERR attach-editor-unknown-pid

$ wandr-arbiter ime-commit-text hello               → OK route→pid=5584 …
$ wandr-arbiter ime-send-key-event 0 67 down        → OK route→pid=5584 …
$ wandr-arbiter ime-set-composing-text ni           → OK route→pid=5584 …
$ wandr-arbiter ime-finish-composing-text           → OK route→pid=5584 …

$ wandr-arbiter list
  OK count=2
    app=com.example.wandr-app   pid=5584 elapsed=…  [editor:text]
    app=com.example.wandr-app2  pid=5667 elapsed=…  [fg] [ime]

$ wandr-arbiter detach-editor 5584                  → OK detached …
$ wandr-arbiter ime-commit-text orphan              → ERR no-focused-editor
$ wandr-arbiter set-ime -                           → OK cleared prev=com.example.wandr-app2
```

Logcat captures structured `arbiter: ime-<verb> → editor pid=X
app-input-type=Y args=…` lines that step 2 replaces with actual
delivery once per-host control sockets exist.

**Files added/changed (this step):**

- `wit/ime.wit` — new protocol definition
- `wandr-arbiter/src/state.rs` — `ActiveIme`, `EditorInfo`,
  `EditorFocus` types + getters/setters; `remove` clears both
  fields on app death
- `wandr-arbiter/src/main.rs` — eight new socket commands +
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
- New WIT imports on the wandr-app side: the input-connection
  interface (host-driven calls from the IME → guest's editor).
- `compose-multiplatform-core/.../wasmWasiMain/...` actuals for
  the `PlatformTextInputMethodRequest` extension points.
- Replace `WasiSoftKeyboard` registration in wandr-app with the
  new external-IME path; the in-canvas keyboard's code stays in
  tree as the fallback for when no IME app is installed.

Success criterion: tapping a `BasicTextField` in wandr-app logs
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
and `wandr-app/wit/deps/skiko-gfx/skiko-gfx.wit`) and added it
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
to promote `wandr:ime/types` to its own importable package.

**Host impl** — new
`wandr-host/src/ime_host_impl.rs` implements
`my::skiko_gfx::ime::Host for HostState`:

- `notify_editor_attached` opens a one-shot UNIX socket to
  `/data/local/tmp/wandr-arbiter.sock`, writes
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

**Wandr-app integration** — `WasiKeyboardController.show()` /
`.hide()` now ALSO call `Ime.Import.notifyEditorAttached(...)`
/ `notifyEditorDetached()` alongside flipping
`isVisible.value` (which still drives the in-canvas keyboard).
Try/catch defensive — if the arbiter is down, the in-canvas
keyboard still works.

**Build pipeline gotcha caught + fixed during smoke** —
wandr-app's `package.toml` (in `scripts/build-system-wandrpkgs.sh`)
was missing `[dependencies]` declarations for the three system
bundles (markdown/emoji/fonts). The installer auto-detects
component imports for the "missing dep — refuse install" gate
but DOES NOT populate `[dependencies_resolved]` in
cache-key.toml without an explicit `[dependencies]` block — so
the loader's `load_dep_components` walked an empty table and
instantiation failed with "wandr:markdown/renderer.render has
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
$ wandr-arbiter launch com.example.wandr-app
  OK pid=7265 app=com.example.wandr-app
  log: loader: loaded dep `fonts` (wandr:fonts/loader@0.1.0) from …
  log: loader: loaded dep `markdown` (wandr:markdown/renderer@0.1.0) from …
  log: loader: dep `emoji` instantiated; wired 1 fn(s) across 1 interface(s)
  log: loader: dep `fonts` instantiated; wired 2 fn(s) across 1 interface(s)
  log: loader: dep `markdown` instantiated; wired 1 fn(s) across 1 interface(s)
  log: standalone: rendered frame 1, 2, 3, …

# tap on the BasicTextField in TextFieldCard …
  log: ime-host: forwarded attach-editor pid=7265 input-type="text" hint-len=0
       text-len=0 selection=[0..0]
  log: arbiter: attach-editor pid=7265 app=com.example.wandr-app input-type=text
       hint="" initial-text-len=0 → route to (no active IME — set-ime first)
       (step 2 delivers on-editor-attached)

# tap outside …
  log: ime-host: forwarded detach-editor pid=7265
  log: arbiter: detach-editor pid=7265 → route to (no active IME)
       (step 2 delivers on-editor-detached)
```

Step 2 success criterion met by construction:
"tapping a `BasicTextField` in wandr-app logs `attach-editor`
reaching the arbiter; tapping out logs `detach-editor`."

**Out of scope** (lands in step 3 — the actual IME app):

- Inbound delivery — IME app's `commit-text` / `send-key-event`
  arriving in the focused guest's editor. Needs a per-host
  control socket (wandr-host child opens connection to the
  arbiter at startup; arbiter pushes events down it). Will
  land in step 3 alongside the first-party `wandr.ime.keyboard`
  wandrpkg, since the two pieces are needed together to actually
  type a character end-to-end.
- Real `EditorInfo` fields. wandr-app's `WasiKeyboardController`
  hard-codes `input-type="text"`, `hint=""`, `initial-text=""`,
  `selection=[0..0]`. Threading the BasicTextField's actual
  `KeyboardOptions.keyboardType` + the textfield's current
  text/selection through Compose's
  `PlatformTextInputMethodRequest` is a separate refactor.

**Files added/changed (this step):**

- `wit/skiko-gfx.wit` + mirrors — new `ime` interface added
  to skiko-ui imports.
- `wandr-host/src/ime_host_impl.rs` (new) — WIT impl that
  forwards to the arbiter socket.
- `wandr-host/src/lib.rs` — `mod ime_host_impl;`.
- `skiko/skiko/src/wasmWasiMain/kotlin/generated/{Internal,}SkikoUi.kt`
  — hand-added `Ime` bindings.
- `wandr-app/src/wasmWasiMain/kotlin/WasiKeyboardController.kt`
  — `show()` / `hide()` also call the new WIT verb.
- `scripts/build-system-wandrpkgs.sh` — declares wandr-app's
  cross-app deps in its package.toml template + redirects
  build_system_wasm's diagnostics to stderr.
- `tasks/47-ime-via-guest-app.md` — this section.

### Step 3 — `wandr.ime.keyboard` first-party wandrpkg (~1 week)

The actual keyboard UI. New repo `wandr.ime.keyboard/` (sibling
to `wandr-arbiter` / `markdown-renderer` / etc).

- Compose Material3 UI, fullwidth at the bottom of the screen.
- QWERTY layout, shift/caps lock, basic punctuation, numbers
  via secondary layer, backspace, enter, space.
- WIT imports: `wandr:ime/input-connection` (calls `commit-text`,
  `send-key-event`, etc).
- WIT exports: `wandr:ime/ime` (receives `on-editor-attached`,
  shows the keyboard; `on-editor-detached`, hides).
- Build pipeline: same as wandr-app (Kotlin/Compose → wasm →
  component embed → component new → .wandrpkg).
- Package: `app_id = "wandr.ime.keyboard"`, `kind = "system"`
  (it's the IME-ecosystem equivalent of the markdown/emoji/fonts
  system bundles), `version = "0.1.0"`.

Success criterion: tap text field → keyboard renders + accepts
taps → tapped letters appear in the focused TextField via
`commit-text` round-trip.

#### Step 3a — inbound delivery infra (2026-05-27)

Split step 3 into two sub-steps for tractable commits. **Step 3a
ships the protocol path that delivers events from the arbiter
INTO a running guest** — the missing inbound half of the IME
loop. Step 3b is the actual `wandr.ime.keyboard` Compose UI, which
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

**`wandr-host/src/ime_inbound.rs` (new)**:

```
accept thread (background)         render loop (main thread)
───────────────────────             ───────────────────────────
UnixListener::accept()              ── per frame ──
read lines                          drain_queue() → Vec<InboundEvent>
parse `key-event <cp> <kid> <act>`  for each event:
queue.push_back(KeyEvent {...})       dispatch_key_v2(skiko, store, …)
                                    (re-uses task 33 step 3)
```

Per-host socket path: `/data/local/tmp/wandr-host-<pid>.sock`.
Bound by the forked child after EGL is up. Mode 666. The
arbiter derives the path from `EditorFocus.pid` — no
registration handshake needed.

Why two threads: wasmtime's `Store` is `!Send`, so only the
render-loop thread can call into the wasm guest. The accept
thread just parses + queues; the render loop drains +
dispatches. Same separation the InputFlinger drain uses.

**`wandr-host/src/standalone.rs`**:

- After EGL setup: `crate::ime_inbound::spawn_listener()`.
- Per-frame, alongside the InputFlinger drain:
  `for ev in crate::ime_inbound::drain_queue() { ...
  dispatch_key_v2(skiko, &mut store, action, code_point, key_id) ... }`

**`wandr-arbiter/src/main.rs`** — `cmd_ime_route` for the
`send-key-event` verb now actually delivers:

```rust
let host_sock = format!("/data/local/tmp/wandr-host-{}.sock", focus.pid);
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
$ wandr-arbiter launch com.example.wandr-app  → OK pid=7989
  log: standalone: ime-inbound listening on
       /data/local/tmp/wandr-host-7989.sock

# (user taps the TextField in TextFieldCard)
  log: arbiter: attach-editor pid=7989 app=com.example.wandr-app
       input-type=text … → route to (no active IME — set-ime first)

# inject 'a' (code-point 97 = 'a', key-id 29 = AKEYCODE_A)
$ wandr-arbiter ime-send-key-event 97 29 down  → OK route→pid=7989
$ wandr-arbiter ime-send-key-event 97 29 up    → OK route→pid=7989
  log: arbiter: ime-send-key-event → pid=7989 (97 29 down)
       delivered via /data/local/tmp/wandr-host-7989.sock
  log: [wasm] tfstate text="hello worlda" sel=TextRange(12, 12)
                              ^^^^^^^^^^^^
                              the 'a' arrived in the BasicTextField

# inject 'b' then 'c'
  log: [wasm] tfstate text="hello worldab" sel=TextRange(13, 13)
  log: [wasm] tfstate text="hello worldabc" sel=TextRange(14, 14)
```

**Out of scope for step 3a** (lands in step 3b → some done, some
deferred — see step 3b results below):

- `commit-text` / `set-composing-text` / `finish-composing-text`
  /  `set-selection` delivery. Needs new WIT exports on the
  editor-bearing-app side (new `on-commit-text` etc. methods on
  the renderer interface, or a new exported interface) +
  Compose-side handling that mutates the focused TextFieldState.
  Step 3a's send-key-event-only path is sufficient for ASCII
  typing.
- The `wandr.ime.keyboard` wandrpkg itself. Now unblocked — the
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

- `wandr-host/src/ime_inbound.rs` — new module: per-host socket
  listener thread + queue + drain.
- `wandr-host/src/lib.rs` — `mod ime_inbound;` (android-only).
- `wandr-host/src/standalone.rs` — spawn_listener after EGL +
  per-frame drain calling `dispatch_key_v2`.
- `wandr-arbiter/src/main.rs` — `cmd_ime_route` for
  send-key-event actually delivers; added `deliver_to_host`
  one-shot helper.
- `tasks/47-ime-via-guest-app.md` — this section.

#### Step 3b — first-party `wandr.ime.keyboard` wandrpkg (2026-05-27)

**Outcome:** ✅ end-to-end on Pixel 2 XL across two concurrent
processes. Tapping a key in the IME app delivers a synthetic
`KeyEvent` to the focused `BasicTextField` in wandr-app via the
new IME-side WIT verb + the step-3a inbound delivery path.

**The full loop, verified live:**

```
[user taps "a" button in wandr.ime.keyboard's Compose UI]
       │
       ▼
[IME pid=12105] ime-key tap: label=a codePoint=97 keyId=29
[IME] keyboard-host: forwarded ime-send-key-event 97 29 down
       │  (UNIX socket: wandr.ime.keyboard → arbiter)
       ▼
[arbiter] ime-send-key-event → pid=9819 (97 29 down) delivered
          via /data/local/tmp/wandr-host-9819.sock
       │  (UNIX socket: arbiter → wandr-app's per-host control socket)
       ▼
[wandr-app pid=9819] ime_inbound accept thread → queue → render-
                    loop drain → dispatch_key_v2
       │
       ▼
[wandr-app] tfstate text="hello worldabcd lowa" sel=TextRange(20, 20)
                              ↑↑↑↑↑↑↑↑↑↑↑↑
                              keys from the IME accumulated here
```

**New `my:skiko-gfx/keyboard` WIT interface** — IME-side outbound
half (mirror of `ime` interface from step 2, which is editor-side
outbound):

```wit
interface keyboard {
    send-key-event: func(code-point: u32, key-id: u32, action: u8);
}
```

Added to `world skiko-ui` imports. The wandr-host impl
(`keyboard_host_impl.rs`) forwards each call to the arbiter as
`ime-send-key-event <cp> <kid> <down|up>`; the arbiter then
routes via the focused-pid's per-host control socket (step 3a).
Step 3b ships `send-key-event` only — `commit-text` etc. need
new editor-side WIT exports + Compose-side handling, deferred.

**`wandr.ime.keyboard` repo (new sibling)** — Kotlin/Compose
first-party IME, bootstrapped from `wandr-app/` template:

```
wandr.ime.keyboard/
  build.gradle.kts       (copied from wandr-app, renamed artifact)
  settings.gradle.kts    rootProject.name = "wandr-ime-keyboard"
  src/wasmWasiMain/
    kotlin/
      Main.kt            (kept as-is from wandr-app — same
                          @WasmExport renderer surface, smoke
                          test calls stripped)
      RealComposeApp.kt  (rewritten — 200 LoC Compose UI: 13
                          ImeKey buttons in 3 rows, bottom-
                          anchored, dark theme. Each tap calls
                          Keyboard.Import.sendKeyEvent.)
      WasiHapticFeedback.kt
      WasiLifecycleOwnerBridge.kt
      compose/ generated/  (boilerplate from wandr-app — minor
                            unused-symbol drag, will trim if it
                            becomes annoying)
  wit/
    wandr-ime-keyboard.wit
      package wandr:ime-keyboard@0.1.0;
      world ime-keyboard { include my:skiko-gfx/skiko-ui@0.1.0; }
    deps/skiko-gfx/skiko-gfx.wit   (mirror)
```

The IME app's only outbound dep is `my:skiko-gfx/keyboard` (the
new interface). No cross-app deps needed (markdown / emoji /
fonts dropped because the keyboard doesn't render rich content).

**Layout**: 3 rows × bottom-anchored. Top row `a b c d e`; second
row `h l o w r` (chosen for typing "hello world" without a full
QWERTY); bottom row `space ⌫ ⏎`. Bottom-anchor via
`Box(fillMaxSize, contentAlignment = BottomCenter)`. The upper
~75% of the IME's surface is the dark panel background — once
step 3c lifts the libgui shim's `eLayerOpaque` flag, the upper
area can be transparent and wandr-app will show through above
the keyboard (the proper Android-IME UX).

**Package**:

```toml
app_id      = "wandr.ime.keyboard"
version     = "0.1.0"
world       = "wandr:ime-keyboard/ime-keyboard"
composition = "same-store"

[components]
ui = "components/ui.wasm"
```

Installed under `<APPS_ROOT>/apps/wandr.ime.keyboard/0.1.0/` —
treated as a regular user app at MVP. (`kind = "system"` is a
future polish for when multiple IMEs ship and the user picks
one — for now there's just the one.)

**Smoke transcript** (Pixel 2 XL, with wandr-app + wandr.ime.keyboard
concurrent):

```
$ wandr-arbiter launch com.example.wandr-app  → pid=9819
# user taps a BasicTextField in wandr-app → attach-editor fires
$ wandr-arbiter list
  com.example.wandr-app  pid=9819 [fg] [editor:text]

$ wandr-arbiter launch wandr.ime.keyboard      → pid=12105
$ wandr-arbiter foreground wandr.ime.keyboard  → IME surface on top
$ wandr-arbiter set-ime wandr.ime.keyboard
$ wandr-arbiter list
  com.example.wandr-app  pid=9819            [editor:text]
  wandr.ime.keyboard      pid=12105 [fg] [ime]

# user taps the IME's "a" button:
  log: [ime] ime-key tap: label=a codePoint=97 keyId=29
  log: [arbiter] ime-send-key-event → pid=9819 (97 29 down)
       delivered via /data/local/tmp/wandr-host-9819.sock
  log: [wandr-app] tfstate text="hello worlda"

# multiple keys accumulated through the smoke:
  tfstate text="hello worldabcd lowa"
```

**Architecture proven**: 13 keys × 2 events (down + up) = 26
round-trips across two processes via 3 UNIX sockets, no
dropped events, no crashes.

**Step 3 success criterion**: "tap text field → keyboard renders
+ accepts taps → tapped letters appear in the focused TextField
via commit-text round-trip." ✅ met — except the round-trip is
via `send-key-event` (virtual-hardware-keyboard path), not
`commit-text`. The user-observable behavior is identical for
ASCII typing; commit-text matters for autocorrect / CJK /
emoji which are future-IME work.

**Bug found + fixed during smoke**: stale skiko in mavenLocal —
needed `./gradlew publishWasmWasiPublicationToMavenLocal` first
so the new `Keyboard` symbol was resolvable from the new IME
app's compile. Standard skiko-changes workflow, just easy to
forget when bootstrapping a new sibling project.

**Files added/changed (this step):**

- `wit/skiko-gfx.wit` + mirrors — new `keyboard` interface;
  added to `world skiko-ui` imports.
- `wandr-host/src/keyboard_host_impl.rs` (new) — Host trait
  forwards `send_key_event` to arbiter's
  `ime-send-key-event` socket cmd.
- `wandr-host/src/lib.rs` — `mod keyboard_host_impl;`.
- `skiko/.../generated/SkikoUi.kt` + `InternalSkikoUi.kt`
  — hand-added `Keyboard` interface + Import companion.
- `wandr.ime.keyboard/` — new sibling repo (full Kotlin/Compose
  project bootstrapped from `wandr-app/`, stripped to a
  single-purpose IME).
- `tasks/47-ime-via-guest-app.md` — this section.

**Step 3 deferred items** (carry to step 3c):

- `commit-text` / `set-composing-text` / `finish-composing-text`
  / `set-selection` outbound from IME. Need new editor-side WIT
  exports + Compose-side `PlatformTextInputMethodRequest`
  handling to actually mutate `TextFieldState`. Until then, the
  IME uses `send-key-event` exclusively (which works for ASCII;
  blocks CJK / autocorrect / emoji codepoints).
- Multi-surface visibility: IME's surface is opaque (libgui
  shim hardcodes `eLayerOpaque`), so when the IME is foreground
  wandr-app is hidden entirely. Real Android-IME UX has the IME
  as a partial-screen overlay with the focused app visible
  above. Needs both shim changes (lift `eLayerOpaque`,
  potentially set per-surface) and arbiter changes (visible-
  but-not-foreground app state).
- Full QWERTY UI + shift/caps + secondary number layer + a
  proper layout with all 26 letters. The single-letter MVP
  proves the architecture; full keyboard is iteration on the
  `KeyboardScreen()` composable.
- `kind = "system"` for the IME package — currently `kind`
  defaults to `app`. The IME is conceptually system-shaped;
  upgrade when a real "pick-your-IME" UI ships.
- Editor-attached/detached events delivered to the IME (so the
  IME can show/hide itself based on whether an editor is
  focused, instead of always being visible). Needs new
  WIT exports on the IME-side.

#### Step 3c plan — multi-surface visibility (the eLayerOpaque fix)

**Symptom** (verified post-rebuild 2026-05-27): when the IME is
promoted to foreground, its SurfaceControl covers the screen
opaquely and wandr-app is invisible — even though wandr-app's
process is alive and rendering frames. Confirmed by screenshot
+ logcat: both `begin_frame: logical=1440x2880` lines present;
arbiter promoted IME via SIGUSR2 (→ `set_layer(MAX)`,
`set_visible(true)`) and demoted wandr-app via SIGUSR1 (→
`set_layer(0)`, **`set_visible(false)`**, lifecycle Paused).
This is the user-visible "keyboard hides the real app" issue.

**Root cause** — two pieces stacked:

1. `cpp/sf_surface.cpp` hardcodes `t.setFlags(g_control,
   eLayerOpaque, eLayerOpaque)` at lines 224-225. Even if both
   surfaces were visible, the IME's would opaquely occlude
   wandr-app's because SF doesn't blend opaque layers.
2. `standalone.rs:240-242` calls `sf.set_visible(false)` on
   any process that receives SIGUSR1 (Background role). The
   arbiter's `promote_to_foreground` SIGUSR1's the previous fg
   whenever a new fg comes up — including when the new fg is
   the IME (which conceptually should be an *overlay*, not a
   replacement).

**Decision — Approach A (partial-surface overlay, no alpha)**:
the IME's SurfaceControl is sized to just the bottom strip of
the screen (e.g. 1440×1100) and positioned at y=PH-1100.
SurfaceFlinger composites IT and wandr-app as two opaque
non-overlapping rects. No transparency, no `eLayerOpaque`
lift, no per-frame transparent-clear — both surfaces stay
opaque, the IME just doesn't span the full screen. This
parallels how real Android IMEs work (the IME window is
sized to its content height).

The chosen alternative (Approach B — full-screen surface with
`eLayerOpaque` lifted + transparent-clear) was rejected
because (a) it makes every frame more expensive (SF must
alpha-blend) and (b) it requires Skia to clear to
`Color::TRANSPARENT` and the IME's Compose layout to draw
strict-only on the keyboard area — fragile under recompose.

**Concrete API**:

1. **Shim** (`cpp/sf_surface.cpp`, requires a-03 build):

   ```c
   // New entry point. Same shape as sf_create_fullscreen_surface
   // but the SC is created at (PW, height_px), positioned at
   // (0, PH - height_px). All other behavior identical
   // (eLayerOpaque kept, transform hint per panel, BBQ
   // attached directly to g_control, input window registered
   // for the smaller rect).
   ANativeWindow* sf_create_overlay_surface(
       int32_t height_px,
       int32_t* out_w,
       int32_t* out_h,
       uint32_t* out_transform);
   ```

   Implementation: parameterize the existing function's
   `createSurface(name, W, H, ..., 0)` + `setSize` /
   `setPosition` transaction. Keep `eLayerOpaque` (the IME
   panel IS fully opaque within its bounds — wandr-app shows
   ABOVE the keyboard, not THROUGH it). Input window must
   register at (0, PH-H) → (PW, PH) so InputFlinger routes
   only taps inside the keyboard's rect to the IME process.

2. **Wandr-host** (`wandr-host/src/sf_surface.rs` +
   `standalone.rs`):

   - `sf_surface::create_overlay(height_px) -> Result<...>`
     dlsyms `sf_create_overlay_surface`.
   - New CLI flag `--standalone-overlay <height_px>`. When
     present, `standalone::run` calls `create_overlay` instead
     of `create_fullscreen`.
   - On SIGUSR1 (Background role) we should still call
     `set_layer(0)` but **must NOT** `set_visible(false)` when
     this child is the underlying-editor process and the new
     fg is an overlay. Cleanest plumbing: introduce a third
     `AppRole::OverlayBehind` and signal it via SIGRTMIN+1 (or
     via the arbiter writing the previous fg's pid to a state
     file the wandr-host watches). Action under
     `OverlayBehind`: keep visible, demote layer, lifecycle
     stays `Resumed` (the editor must keep rendering so the
     user sees the cursor blink).

3. **Arbiter** (`wandr-arbiter/src/main.rs`):

   - New cmd `overlay <app-id>`: promotes `<app-id>` as an
     OVERLAY foreground. Signals the IME with SIGUSR2 (normal
     fg). Signals the previous fg with **SIGRTMIN+1**
     (OverlayBehind) instead of SIGUSR1.
   - When the overlay app exits / is killed / loses its IME
     status, restore previous fg by SIGUSR2-promoting it
     again. Track this via a new `overlay_pid` field next to
     `foreground_pid` in `state`.
   - Set-ime should be split from overlay — the IME being the
     **routing target** for `ime-send-key-event` doesn't
     necessarily mean it's the **visible overlay**. The
     keyboard SHOULD only become visible when an editor is
     attached. Tie overlay show/hide to `attach-editor` /
     `detach-editor` in the IME's render loop (overlay = on
     while editor focused).

4. **Compose** (`wandr.ime.keyboard/.../RealComposeApp.kt`):

   The current `Box(fillMaxSize)` keeps working as-is —
   `fillMaxSize` adapts to whatever surface size we give the
   guest at composition. The dark background continues to be
   the IME panel; with the smaller surface it occupies the
   bottom strip only, so wandr-app shows above it naturally.
   The `BottomCenter` anchor becomes redundant but is
   harmless.

   Size handoff: `WindowInfo.containerSize` reads from
   `WitCanvas.Import.surfaceWidth/Height` which the host
   already returns based on the SurfaceControl's logical
   size — should plumb through correctly once the host opens
   the smaller surface.

**Build dependencies**:

- The shim change (item 1) requires building `libsf_surface.so`
  on the AOSP a-03 host (per `project_boot_model_libgui_build`)
  — same machine task 33 / task 46 step 4 used.
- Items 2-4 build on the regular dev machine. Items 3-4 can
  land before item 1 and self-test by passing
  `--standalone-overlay 1100` with the OLD shim — the call
  will fail with `dlsym` returning null, and we'll log + bail.
  Catches any plumbing typos early.

**Smoke**: `wandr-arbiter overlay wandr.ime.keyboard` while
wandr-app is fg → wandr-app visible above the keyboard,
keyboard visible at the bottom, taps in the keyboard area
route to the IME process via InputFlinger (its input window
registered for the bottom rect), taps above route to wandr-app.

#### Step 3c results (2026-05-27)

**Outcome:** ✅ device-verified end-to-end on Pixel 2 XL. wandr-app
renders fullscreen at the top half of the panel; the IME (wandr.ime.keyboard)
renders as a 1100-px overlay at the bottom. Both surfaces opaque,
SurfaceFlinger composes them as non-overlapping rects. Auto-tie via
`attach-editor` / `detach-editor` works; manual `overlay` /
`overlay-clear` socket cmds work; the IME's `LaunchedEffect` at
composition root sets its own height via the new WIT verb.

**Architectural surprise:** setting position directly on a single
BBQ-backed `SurfaceControl` does NOT stick on this device — the
layer kept ending up at `displayFrame=(0,0,1440,1100)` no matter
what we tried (setPosition pre-BBQ-create, post-BBQ-create,
setDestinationFrame, both X/Y orderings for landscape-native
panel, defensive re-application on show + setLayer). The fix
came from `frameworks/native/libs/gui/tests/EndToEndNativeInputTest.cpp`
`BlastInputSurface` class (line 344): **BBQ-backed surfaces need a
PARENT container surface that carries geometry**. The pattern:

```cpp
// Container parent (no buffer, just geometry holder)
g_overlay_parent = createSurface(name, 0, 0, fmt, eFXSurfaceContainer);
// Buffer-state child, parented to container
g_control = createSurface(name, PW, H, fmt,
                          eFXSurfaceBufferState,
                          g_overlay_parent->getHandle());

// Position/layer/crop on the PARENT.
t.setPosition(g_overlay_parent, 0, Y);
t.setLayer   (g_overlay_parent, MAX-1);
t.setCrop    (g_overlay_parent, Rect(0, 0, PW, H));

// Crop + show + flags on the CHILD; BBQ + input window also attach
// to the child.
t.setCrop(g_control, Rect(0, 0, PW, H));
t.show   (g_control);
t.setFlags(g_control, eLayerOpaque, eLayerOpaque);
```

The fullscreen path stays as-is (single-surface, no parent). Only
the overlay path needs the indirection.

**Eleven artifacts shipped (across 5 repos):**

| Repo | File(s) |
|---|---|
| wandr (top) | `wit/skiko-gfx.wit` — new `request-overlay-height` verb on `keyboard` interface |
| wandr-host | `cpp/sf_surface.{cpp,h}` (parent-container + `sf_create_overlay_surface` + `sf_resize_overlay`), `src/sf_surface.rs` (create_overlay + resize_overlay + `ANativeWindow_setBuffersGeometry` FFI), `src/app_role.rs` (`OverlayBehind=2` + SIGRTMIN+1 handler), `src/standalone.rs` (`--standalone-overlay` branch + OverlayBehind arm + per-frame overlay-resize drain), `src/keyboard_host_impl.rs` (`request_overlay_height` via static-atomic bridge), `src/zygote.rs` (`LAUNCH_GUI_OVERLAY` + `ChildAction::Gui{overlay}`), `src/main.rs` (`--standalone-overlay` CLI flag) |
| wandr-arbiter | `src/state.rs` (`OverlayState`), `src/main.rs` (`cmd_overlay` / `overlay-clear` / `launch-overlay` + `promote_to_overlay` / `demote_from_overlay` + auto-tie in `cmd_attach_editor` / `cmd_detach_editor`), `src/zygote_client.rs` (`launch_gui_overlay`) |
| skiko | `skiko/wit/skiko-gfx.wit` (mirror), `skiko/src/wasmWasiMain/kotlin/generated/{Internal,}SkikoUi.kt` (hand-added `Keyboard.Import.requestOverlayHeight`) |
| wandr.ime.keyboard | `src/wasmWasiMain/kotlin/RealComposeApp.kt` (`LaunchedEffect { requestOverlayHeight(1100u) }` at composition root) |
| wandr-app | `wit/deps/skiko-gfx/skiko-gfx.wit` (mirror) |

**New socket commands** (`wandr-arbiter`):
- `launch-overlay <app-id>` — like `launch` but the child acquires a
  bottom-strip overlay surface via the new `LAUNCH_GUI_OVERLAY`
  zygote cmd.
- `overlay <app-id>` — manually engage the overlay split: IME →
  Foreground (`SIGUSR2`), prior fg → `OverlayBehind` (`SIGRTMIN+1`).
- `overlay-clear` — tear down the split: IME → Background
  (`SIGUSR1`), behind-app → Foreground (`SIGUSR2`).
- `attach-editor` / `detach-editor` — now auto-fire
  `promote_to_overlay` / `demote_from_overlay` when an `ActiveIme` is
  set, so the IME visibility is driven by editor focus rather than
  needing an explicit `overlay` command.

**Device-verified smoke transcript:**

```
$ wandr-arbiter launch com.example.wandr-app           → pid=21228
$ wandr-arbiter launch-overlay wandr.ime.keyboard       → pid=21235
                                                       (created at
                                                        Disp Frame
                                                        0 1780 1440 2880)
$ wandr-arbiter overlay wandr.ime.keyboard
  OK overlay=wandr.ime.keyboard pid=21235 prev-fg=com.example.wandr-app
     behind-pid=21228
  # SurfaceFlinger now shows two layers composed as non-overlapping
  # rects:
  #   wandr#... at        Disp Frame=0    0 1440 1780
  #   wandr-ime-overlay#... at Disp Frame=0 1780 1440 2880
  #   wandr-ime-overlay-parent#... — child of which the buffer surface
  #                                hangs (the BlastInputSurface pattern)
$ wandr-arbiter overlay-clear
$ wandr-arbiter set-ime wandr.ime.keyboard
$ wandr-arbiter attach-editor 21228 text
  OK ... overlay=engaged    # ← auto-tie fires
$ wandr-arbiter detach-editor 21228
  OK ... overlay=cleared    # ← auto-tie reverses
```

**Out of scope for step 3c, lands later:**

- **Real focus arbitration between two surfaces** — step 4. Touches
  in the bottom 1100 px route to the IME via InputFlinger's
  per-window touchableRegion match (confirmed in dump:
  `touchableRegion={0,1780,2880,1440}` for the IME's input window);
  taps above route to wandr-app's fullscreen window. Whether key events
  go to the focused window vs the IME's input channel is step-4
  business. Auto-hide on tap-outside is also step 4.

- **Live resize without re-attaching EGL** — the host calls
  `ANativeWindow_setBuffersGeometry` after `sf_resize_overlay` to
  flush producer-side geometry. The IME's initial 1200→1100 resize
  during first composition works visually but Skia caches buffers at
  the old size; minor visual cost. Revisit if a real shift/secondary-
  layer changes the IME's preferred height during a session.

- **Dynamic panel dimensions** — `cpp/sf_surface.cpp` still has
  `constexpr PANEL_W=1440 / PANEL_H=2880` (taimen-specific). Spun
  out as `tasks/48-panel-dim-query.md`.

**Files added/changed (this step):**

- All listed in the "Eleven artifacts shipped" table above.
- `tasks/47-ime-via-guest-app.md` — this section.
- `tasks/48-panel-dim-query.md` — new (follow-up).
- `MEMORY.md` → no new entry; the BlastInputSurface pattern is
  documented here for now (revisit if a second overlay shape
  ever appears).

### Step 4 — InputFlinger focus arbitration + auto-hide (~2-3 days)

**Forward reference (task 49):** task 49's inbound socket
(`/data/local/tmp/wandr-host-<pid>.sock`, written + drained by the
host's `ime_inbound` queue) is the same per-host control channel
step 4 will reuse for `request-hide` / tap-outside-to-hide / focus
changes. Step 4 doesn't need new transport, just new message
shapes — extend `ime_inbound.rs::InboundEvent` with `RequestHide`
etc. and route from a new `Keyboard.Import.requestHide` WIT verb
the IME calls on its ⌄ key or on tap-outside detection.

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

Success criterion: type text into wandr-app via the new IME;
tap outside the keyboard → keyboard hides + focus returns to
wandr-app; tap the text field again → keyboard back. No focus
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

The `wandr:ime/client` WIT contract is the stability gate. Each
future IME is:

- **`wandr.ime.voice`** — Compose UI is a microphone button +
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

- **`wandr.ime.emoji`** — reuses `wandr.emoji.picker` (task 40,
  already a system bundle). Compose UI is the emoji grid
  itself (`EmojiCard.kt` already implemented in wandr-app — the
  rendering code carries straight over). Tap → `commit-text(emoji_codepoint)`.
  Implementation cost: small. Could be ~1 week.

- **`wandr.ime.zh`** / `wandr.ime.ja` / `wandr.ime.ko` / `wandr.ime.in.*` —
  CJK / Indic IMEs. The WIT side is unchanged; the hard part
  is the input-method dictionary + composition (pinyin →
  candidates, kana → kanji conversion). Could ship as one
  wandrpkg per language, or one multi-language wandrpkg with a
  user-selected mode. Per-IME effort is months, but the
  arbiter/protocol side doesn't change.

The crucial property: **each future IME is a vendor-independent
`.wandrpkg`**, installable via the existing wandrpkg installer (task
35), discoverable via the arbiter's `list-imes` command (TBD,
introspects `<APPS_ROOT>/apps/*` for app_ids starting with
`wandr.ime.`), switchable at runtime. The Android-equivalent
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
- `wandr-arbiter/src/main.rs` — new `cmd_attach_editor`,
  `cmd_detach_editor`, `cmd_set_ime`, `cmd_ime_commit_text`.
- `wandr-arbiter/src/state.rs` — `EditorFocus`, `ActiveIme`
  state.
- `wandr-host/src/ime_imms_probe.rs` (rename from `ime_impl.rs`)
  — the task 40 probe code, kept for historical reference.
- `wandr-host/src/ime_router_impl.rs` (new) — host side of the
  WIT plumbing for the focused-app's `attach-editor` outbound
  + `commit-text` inbound.
- `wandr-host/src/lib.rs` — `pub mod ime_router_impl;`.
- `skiko/src/wasmWasiMain/kotlin/org/jetbrains/skiko/wasi/WasiInputMethod.kt`
  (new) — skiko-side actual.
- `compose-multiplatform-core/.../wasmWasiMain/.../PlatformTextInputMethodRequest.wasi.kt`
  (new or update) — Compose actual.
- `wandr.ime.keyboard/` (new sibling repo) — the IME guest itself.
- `scripts/build-system-wandrpkgs.sh` — packages `wandr.ime.keyboard`
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
4. The first-party `wandr.ime.keyboard` is the GROUND TRUTH for
   the IME-side of the WIT contract. If a contract change
   makes it impossible to write a sensible keyboard, the
   contract is wrong; iterate on the WIT.
5. Voice / emoji / CJK are explicit out-of-scope for this
   task — they're future `.wandrpkg`s, not modifications to
   `wandr.ime.keyboard`. The contract should support them
   without changes.

## Related

- `tasks/40-real-ime.md` — the abandoned IMMS-via-rsbinder
  path. Code (commit `09782f5` in wandr-host, the AIDL probes
  in `src/ime_impl.rs`) kept in tree for historical reference.
- `tasks/44-wms-window-registration.md` — the abandoned
  vendor-WMS path. Same reason.
- `tasks/46-wandr-arbiter-mvp.md` — the arbiter machinery this
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
