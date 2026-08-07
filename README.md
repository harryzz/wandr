# wandr — a portable UI runtime for WASM apps

Write a UI once — in **any language, with any framework** — compile it to a
single WASM component, and run it natively on any OS wandr has a backend for.

**The one idea:** a guest app imports a fixed set of **OS-agnostic WIT contracts**
(render, input, IME, chrome, device, media, audio, events) and never names an OS.
`wandr-host` — wasmtime + the component model + a Skia rendering core — implements
those contracts and delegates only the physically OS-specific bits (surfaces,
input, native services) to a per-OS **backend**. The contracts are portable; only
the backend is OS-specific — so the same guest `.wasm` runs everywhere.

🎥 **See it running:** [demo video](https://youtube.com/shorts/rR4TG-I5Y58) — a
Pixel 2 XL rendering real Compose/Slint/Swift UIs.

## Status

- **Android** (aarch64, post-ART) — the production backend; replaces ART
  end-to-end, renders on real GPU hardware. A rooted, ART-stripped developer
  target, **not** an end-user install.
- **Linux · macOS · Windows** — a working desktop/dev backend: the same guest
  `.wasm` runs via the desktop loop. **This is how you try wandr.**

Shipped guest frameworks: **Compose, Slint, dioxus, Avalonia, Swift/OpenSwiftUI**.

## Quick start (desktop)

Install the runtime + the `wandr` app manager:

```sh
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/harryzz/wandr-host/main/install.sh | sh
```
```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/harryzz/wandr-host/main/install.ps1 | iex
```

Then, in a new terminal, install and run an app:

```sh
wandr list                    # apps in the registry
wandr install wandr.tetris    # download + install
wandr run wandr.tetris        # run it
```

📖 **Full install + usage guide:** [`docs/install.md`](docs/install.md)
(per-OS steps, the GStreamer video dependency, all `wandr` commands).

## Documentation

| Read | For |
|---|---|
| [`docs/overview.md`](docs/overview.md) | The layer model + honest per-backend/per-framework maturity |
| [`docs/install.md`](docs/install.md) | Install the runtime, install & run apps |
| [`docs/repository-layout.md`](docs/repository-layout.md) | Where things live in the repo |
| [`tasks/STATUS.md`](tasks/STATUS.md) | Per-task ledger — what's done and how |

## Repository layout (brief)

```
apps/{system,user}/   guest apps (system chrome + user apps)
runtime/              native Rust host stack — wandr-host (wasmtime+skia), wandr-arbiter
wit/ contracts/       canonical OS-agnostic WIT contracts + WASI proposals
crates/               shared guest-side Rust libs
tools/scripts/        build + registry tooling
docs/ tasks/ repros/  architecture docs · task narrative · focused reproducers
```

Fresh clone (with submodules):
```sh
git clone --recurse-submodules https://github.com/harryzz/wandr.git
```
