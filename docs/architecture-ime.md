# Architecture: the IME (war.ime.keyboard)

This doc explains how soft-keyboard input is delivered in the wart
runtime: which processes are involved, what crosses which boundary,
how the language-plugin system works today, and what's TODO to make
plugin loading fully dynamic.

It exists because the IME has three concurrent processes (the
focused app, the IME guest, wart-arbiter), four control transports
(arbiter↔host UNIX sockets, host↔guest WIT calls, SurfaceFlinger
overlay, InputFlinger input window), and a hand-rolled plugin
contract — non-obvious how it all fits together when you only see
"keystroke lands in TextField".

Companion to [`architecture-host-guest-boundary.md`](architecture-host-guest-boundary.md)
which covers what host↔guest WIT calls are; this doc layers
process / signal / socket transports on top.

## TL;DR

- **war.ime.keyboard** is a regular `.warpkg` guest app — Compose
  Multiplatform on wasmWasi, runs in its own `wart-host` child of
  the zygote, owns a `SurfaceFlinger` overlay surface positioned at
  the bottom of the display.
- **wart-arbiter** is the policy daemon: it tracks which app holds
  editor focus, signals SurfaceFlinger z-order via `SIGUSR1` /
  `SIGUSR2` / `SIGRTMIN+1` to its host children, and routes inbound
  events (attach-editor, key events) over per-host UNIX control
  sockets.
- **Three logical event flows:**
  1. App→arbiter→IME: an app's BasicTextField gains focus →
     `attach-editor` cmd → IME promotes to overlay + receives
     `on-editor-attached(input-type)` so it can pick layout.
  2. IME→arbiter→App: user taps a key → IME calls
     `Keyboard.Import.sendKeyEvent(code-point, key-id, action)` →
     arbiter routes via the focused-app pid's control socket →
     app's render loop drains and dispatches as a Compose KeyEvent.
  3. Layout plugins (currently): IME has hard-coded
     `[dependencies]` for `war.lang.bg` + `war.lang.fr`; the
     wasmtime loader wires them as same-Store deps at IME
     instantiation. TODO: replace with host-mediated dynamic
     loading (task 51).

## Process layout

```
   ┌──────────────────────────────────────────────────────────┐
   │                    Linux kernel                          │
   │                                                          │
   │   wart-host --zygote   wart-arbiter --daemon             │
   │         │                    │                           │
   │         │  fork()             │                          │
   │         ├──────────────────┐  │                          │
   │         │                  │  │                          │
   │  ┌──────▼──────┐    ┌──────▼──┴───┐                      │
   │  │  wart-host  │    │  wart-host  │                      │
   │  │ pid=APP_PID │    │ pid=IME_PID │                      │
   │  │             │    │             │                      │
   │  │ Compose:    │    │ Compose:    │                      │
   │  │ wart-app    │    │ war.ime.kbd │                      │
   │  │             │    │             │                      │
   │  │ owns SF     │    │ owns SF     │                      │
   │  │ fullscreen  │    │ overlay     │                      │
   │  │ surface     │    │ surface     │                      │
   │  │             │    │ (bottom)    │                      │
   │  └─────────────┘    └─────────────┘                      │
   │                                                          │
   │  SurfaceFlinger ◄─── both processes attach BBQ surfaces  │
   │  InputFlinger   ◄─── both register input windows         │
   └──────────────────────────────────────────────────────────┘
```

- `wart-host --zygote` is a long-lived parent that preloads the
  `wasmtime::Engine` (~5 MB of code/JIT setup) and `fork()`s on
  every `LAUNCH` / `LAUNCH_GUI` socket command. Children inherit
  the engine via COW. See `tasks/45-wart-zygote-spike.md`.
- `wart-arbiter` is a sibling daemon at `wart-arbiter --daemon`. It
  listens on `/data/local/tmp/wart-arbiter.sock` for the user's
  CLI commands (`launch` / `set-ime` / `kill` / `attach-editor` /
  …) and on `/data/local/tmp/wart-zygote.sock` to ask the zygote
  to fork.
- Each forked `wart-host` child exposes a **per-host UNIX
  control socket** at `/data/local/tmp/wart-host-<pid>.sock` —
  the channel the arbiter uses to push inbound events into a
  running child (attach-editor, key events, etc.). The child's
  `ime_inbound` module drains it once per frame.

## Surface + input ownership

The wart-host child owns a **SurfaceFlinger surface** acquired
through the `libsf_surface.so` shim (`wart-host/cpp/sf_surface.cpp`,
built in-tree on the AOSP a-03 host — see
[[project-boot-model-libgui-build]]).

Two flavors:
- **Fullscreen** (`sf_create_fullscreen_surface`) — for regular
  apps. Frame `(0,0)` to (panel-w, panel-h).
- **Overlay** (`sf_create_overlay_surface`) — for the IME. A
  bottom strip; layer positioned at `(0, panel-h - overlay-h)`.

Each surface also registers an **InputFlinger input window** via
`IInputFlinger::createInputChannel` + `gui::WindowInfoHandle` +
`Transaction::setInputWindowInfo`. The window's `frame` +
`touchableRegion` are passed in **layer-local** coordinates;
SurfaceFlinger adds the layer position to compute display-coord
touchable region. MotionEvents arrive at the input channel in
**window-local** coordinates (no manual offset subtraction
required — this was a step-3c bug: see
`tasks/47-ime-via-guest-app.md` step 3c fix in `wart-host` commit
`0de1da2`).

InputFlinger uses **z-order from the `Windows:` list**, not the
focused-window setting. The IME's overlay sits above the app's
fullscreen surface; taps in the bottom strip route to the IME's
channel, taps above route to the app's.

## Foreground / overlay z policy

`wart-arbiter` signals each host child via UNIX signals to inform it
of its current role:

| Signal      | Role           | Host action                                           |
|-------------|----------------|-------------------------------------------------------|
| `SIGUSR2`   | Foreground     | `sf_set_layer(MAX)` + `sf_set_visible(true)` + lifecycle Resumed |
| `SIGUSR1`   | Background     | `sf_set_layer(0)` + `sf_set_visible(false)` + lifecycle Paused   |
| `SIGRTMIN+1`| OverlayBehind  | `sf_set_layer(0)` + `sf_set_visible(true)` + lifecycle stays Resumed |

When the user taps a TextField:
1. wart-app's Compose layer fires `requestKeyboardController().show()`.
2. wart-app calls `Ime.Import.notifyEditorAttached(input-type, …)`
   — a guest→host WIT call.
3. The host implementation in `wart-host/src/keyboard_host_impl.rs`
   forwards via the **arbiter socket** as `attach-editor <pid>
   <input-type>`.
4. The arbiter looks up which app is "the IME" (set via
   `wart-arbiter set-ime war.ime.keyboard`), promotes the IME's
   host child to `Foreground` (SIGUSR2), demotes the focused-app
   child to `OverlayBehind` (SIGRTMIN+1).
5. The arbiter delivers `editor-attached <input-type>` to the IME
   child over its per-host control socket.
6. The IME child's `ime_inbound` module drains the queue once per
   frame in `wart-host/src/standalone.rs`, then calls the IME
   guest's exported `war:ime/ime.on-editor-attached(input-type)`.
7. The IME guest's `ImeEventsImpl` updates a `MutableState`;
   `pickLayout` recomposes the keyboard with the matching
   editor-driven layout (Numeric / Phone / …).

## Key event delivery

```
   user taps "a" on the IME's bottom-strip surface
       │
       │ (InputFlinger routes to IME's input channel via hit-test)
       ▼
   wart-host child (IME process) — sf_input_poll() drains the
       channel, dispatches as Compose pointer event
       │
       │ (Compose recomposition; KeyButton onClick fires)
       ▼
   IME guest calls Keyboard.Import.sendKeyEvent(
       code-point=97, key-id=29 /* KEYCODE_A */, action=down)
       │
       │ (canonical-ABI lowering; guest→host WIT call)
       ▼
   wart-host/src/keyboard_host_impl.rs::send_key_event
       │
       │ (write "ime-send-key-event 97 29 down" to /tmp/wart-arbiter.sock)
       ▼
   wart-arbiter cmd_ime_route — looks up "focused-app pid"
   from its persistent state, opens
   /data/local/tmp/wart-host-<focused-pid>.sock, writes
   "key-event 97 29 down\n"
       │
       ▼
   wart-host child (focused-app process) — ime_inbound thread
   reads the line, parses, pushes onto a per-frame InboundEvent
   queue
       │
       │ (next render frame)
       ▼
   standalone.rs render loop drains ime_inbound; for each
   KeyEvent, calls dispatch_key_v2 on the skiko WIT export
       │
       │ (host→guest WIT call; lowers code-point+key-id+action)
       ▼
   wart-app guest's skiko renderer thread synthesizes a Compose
   KeyEvent; routes to the focused BasicTextField; "a" appears
```

Two `wart-host` processes, two SF surfaces, two input windows,
two control sockets, three signals — all coordinated by the
arbiter.

## Language plugin system (current)

The IME ships a built-in English QWERTY plus editor-driven layouts
(Numeric / Phone / Email / Url / Password / Symbols / Emoji). All
the other languages — currently Bulgarian + French — are separate
`.warpkg` system components.

### Plugin contract

Each lang plugin is a Rust cdylib targeting `wasm32-wasip2`,
~60 LoC, exporting:

```wit
package war:keyboard-lang-<id>@0.1.0;
interface lang {
    record info {
        name: string, locale: string, is-rtl: bool,
    }
    record key-def {
        display: string, code-point: u32, key-id: u32, width: f32,
    }
    record layout-variant { rows: list<list<key-def>> }
    get-info:   func() -> info;
    get-layout: func(shifted: bool) -> layout-variant;
}
world lang-world { export lang; }
```

The canonical schema lives at `wart/wit/keyboard-lang.wit`; each
plugin has a local copy at `war.lang.<id>/wit/keyboard-lang-<id>.wit`
with a renamed package. Plugins only supply the language's letter
rows — the IME injects digit / shift / utility rows uniformly.

### Why per-plugin packages

The "obvious" design — one shared `war:keyboard-lang/lang@0.1.0`
package exported by every plugin — **doesn't work**. wart-host's
`wire_dep_into_linker` (`wart-host/src/app_loader.rs`) registers
each dep under `linker.instance(interface_name)`. Two deps under
the same name collide: the second `linker.instance(name)` call
returns the existing one and overwrites the funcs; only one plugin
survives.

So each plugin uses its own package name (`war:keyboard-lang-bg`,
`war:keyboard-lang-fr`, …). The downside: the IME hard-codes its
known plugin set.

### Loading + invocation

At IME `package.toml` install time:

```toml
[dependencies]
bg = { system = "war.lang.bg", version = "0.1.0",
       interface = "war:keyboard-lang-bg/lang@0.1.0" }
fr = { system = "war.lang.fr", version = "0.1.0",
       interface = "war:keyboard-lang-fr/lang@0.1.0" }
```

The installer (`wart-host/src/app_installer.rs`) records both deps
in the cache-key. At launch time (`wart-host/src/app_loader.rs`),
`load_dep_components` deserializes each `.cwasm`, and
`wire_dep_into_linker` instantiates each dep in the IME's `Store`
and registers each export as a proxy closure in the IME's linker
under the dep's interface name. Same-Store composition — see
`tasks/36-cross-app-deps.md`.

The IME calls each plugin once at composition time:

```
[ wart-host child for the IME launches ]
  → wasmtime loads + instantiates the IME component
  → load_dep_components loads + instantiates war.lang.bg + war.lang.fr
  → wire_dep_into_linker registers each plugin's get-info /
    get-layout as proxy closures in the IME's linker
  → wasmtime instantiates the IME with the linker
  → IME's main() runs Compose composition
  → ImeKeyboardDefaults.loadAllLayouts() calls
    LangAdapter.loadAllLangPlugins() — hand-written Kotlin/Wasm
    @WasmImport blocks for each known plugin, calls get-info +
    get-layout(false/true), lifts the canonical-ABI returns into
    Kotlin data classes, calls wrapLanguageLayout() to inject the
    digit/shift/utility rows, returns List<KeyboardLayout>
  → loadAllLayouts merges builtins + plugins; the 🌐 cycle becomes
    English → Български → Français
```

`LangAdapter.kt` has hand-rolled canonical-ABI lifts because
wit-bindgen 0.53.1 has no Kotlin generator
([[wit-bindgen-no-kotlin-generator]]). Each lift opens with
`freeAllComponentModelReallocAllocatedMemory()` to avoid the
"can't create new allocators" trap
([[wasi-realloc-allocator-pollution]]).

## TODO — dynamic plugin loading

Spun out as **`tasks/51-dynamic-lang-plugins.md`** (🔲 scoped).

The current design works but the IME owns the plugin registry. To
add a new language (`war.lang.de`) the IME needs:
- an entry in `[dependencies]` of `package.toml`,
- a per-plugin `@WasmImport` block in `LangAdapter.kt`,
- an entry in the static `LangAdapter.plugins` registry,
- a rebuild.

The dynamic design inverts the relationship: **the host owns the
plugin registry**.

```
[ wart-host --zygote startup ]
  → scan <APPS_ROOT>/system-apps/war.lang.*/0.1.0/cache/lang.cwasm
  → for each: deserialize, cache (id, instance, get-info,
    get-layout) in HostState
  → expose new host WIT verbs on a fresh `my:skiko-gfx/lang-plugins`
    interface (or extension of `my:skiko-gfx/keyboard`):
       enumerate-lang-plugins() -> list<lang-plugin-info>
       get-lang-layout(lang-id, shifted) -> list<list<key-def>>
  → host dispatches each get-lang-layout to the right cached Func

[ IME composition ]
  → LangAdapter.kt:
       host.enumerate-lang-plugins() → ids = ["bg","fr","de",...]
       for id in ids:
         info   = host.get-lang-info(id) // also added
         layout = host.get-lang-layout(id, false)
         ...
       LangAdapter shrinks from ~150 LoC of hand-rolled
       canonical-ABI to ~30 LoC of generic enumeration
```

Wins:
- Zero IME rebuild for new languages.
- Plugin distribution decoupled from IME distribution.
- Single source of truth (filesystem scan, not compile-time list).
- Host's typed wasmtime bindgen handles the canonical-ABI lift;
  IME's hand-written Kotlin lifts can be retired.

Costs:
- Host gains plugin lifecycle responsibility — extra `Store` per
  lang plugin at zygote startup (negligible: each lang `.cwasm` is
  ~80 KB).
- New WIT verbs slightly fatten the host contract.
- Doesn't help other plugin-style WIT relationships (each consumer
  needs its own host-mediated bridge).

Trigger conditions documented in `tasks/51-dynamic-lang-plugins.md`:
do this when a 3rd language is concretely needed, when retiring
the cabi_realloc hand-rolled lifts becomes a priority, or when
distributing wart externally where third-party plugins shouldn't
require a custom IME build.

## Related tasks + memories

- `tasks/45-wart-zygote-spike.md` — zygote model and fork
  semantics (Adreno EGL survives fork).
- `tasks/46-wart-arbiter-mvp.md` — arbiter daemon, role
  signalling, control sockets, crash-marker.
- `tasks/47-ime-via-guest-app.md` — IME-as-guest-app design,
  multi-surface visibility, step 3c overlay surface, IME-routing
  socket; **step 4 (focus arbitration + auto-hide) still open**.
- `tasks/49-ime-content-control.md` — this layer: editor-driven
  layouts + language plugins (steps 1–6).
- `tasks/51-dynamic-lang-plugins.md` — the host-mediated TODO.
- [[ime-layout-arbitration]] — pickLayout selector × source
  matrix.
- [[wasi-realloc-allocator-pollution]] — why every Kotlin/Wasm
  WIT lift must lead with `freeAllComponentModelReallocAllocatedMemory()`.
- [[wit-bindgen-no-kotlin-generator]] — why `LangAdapter.kt`'s
  canonical-ABI is hand-written.
- [[wasi-cabi-realloc-export-block]] — why on-editor-attached
  takes primitive params instead of an editor-info record.
