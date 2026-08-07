# Installing wandr and running apps (desktop)

wandr runs the same WASM app on Linux, macOS, and Windows. This guide covers
installing the runtime and then installing + running apps.

> Desktop only. Android is a rooted, ART-stripped developer target and is not
> installed this way.

---

## 1. Install the runtime

The installer downloads the `wandr-host` runtime **and** the `wandr` app-manager
CLI into `~/.wandr/bin` (Windows: `%LOCALAPPDATA%\wandr\bin`) and adds that
directory to your `PATH`.

### Linux / macOS

```sh
curl -fsSL https://raw.githubusercontent.com/harryzz/wandr-host/main/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/harryzz/wandr-host/main/install.ps1 | iex
```

Then **open a new terminal** so the updated `PATH` takes effect, and check it:

```sh
wandr help
```

<details>
<summary>Install somewhere else / pin a version</summary>

- `WANDR_HOME` — install root (default `~/.wandr`, or `%LOCALAPPDATA%\wandr`).
- `WANDR_VERSION` — pin a host release tag instead of the latest (e.g. `v0.1.1`).

```sh
WANDR_HOME=/opt/wandr WANDR_VERSION=v0.1.1 \
  curl -fsSL https://raw.githubusercontent.com/harryzz/wandr-host/main/install.sh | sh
```
</details>

### Video dependency: GStreamer

Video playback uses **GStreamer** as the decode backend. The installer warns if
it's missing. Install it once:

| OS | Command |
|---|---|
| Debian / Ubuntu | `sudo apt install libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav` |
| macOS | `brew install gstreamer` |
| Windows | `winget install GStreamer.GStreamer` |

Apps that don't play video don't need it.

---

## 2. Install and run apps

List what's available in the registry:

```sh
wandr list
```

```
registry: https://harryzz.github.io/wandr/registry/index.json
    wandr.video.player           0.1.0    Video Player
    wandr.audio.player           0.1.0    Audio Player
    wandr.tetris                 0.1.0    Tetris
    wandr.slint.test             0.1.0    Slint Test
    wandr.avalonia.demo          0.1.0    Avalonia Demo
    wandr.swiftui.demo           0.1.0    SwiftUI 2048
    wandr.jellyfin               0.1.0    Jellyfin
    wandr.navidrome              0.1.0    Navidrome

● = installed. Install:  wandr install <id>
```

Install one (downloads it, verifies its checksum, prepares it for your machine):

```sh
wandr install wandr.tetris
```

Run it:

```sh
wandr run wandr.tetris
```

A `●` next to an app in `wandr list` means it's installed.

---

## 3. Command reference

| Command | What it does |
|---|---|
| `wandr list` | Apps in the registry (`●` = installed) |
| `wandr list --installed` | Only the apps you've installed |
| `wandr install <id> [version]` | Download + install an app |
| `wandr run <id>` | Run an installed app |
| `wandr remove <id> [version]` | Uninstall (all versions, or one) |
| `wandr installed` | List installed apps |
| `wandr help` | Usage |

Installed apps live under `~/.wandr/apps/<id>/<version>/`
(Windows: `%LOCALAPPDATA%\wandr\apps\…`).

---

## 4. Updating

Re-run the same install command — it overwrites the runtime and CLI in place
with the latest release:

```sh
curl -fsSL https://raw.githubusercontent.com/harryzz/wandr-host/main/install.sh | sh   # Linux/macOS
irm https://raw.githubusercontent.com/harryzz/wandr-host/main/install.ps1 | iex        # Windows
```

## 5. Uninstalling

Everything lives under one directory — remove it and drop it from `PATH`:

```sh
rm -rf ~/.wandr          # Linux/macOS
```
```powershell
Remove-Item -Recurse -Force $env:LOCALAPPDATA\wandr   # Windows
```

---

## Troubleshooting

- **`wandr: command not found`** — open a new terminal (the installer updated
  `PATH`), or add the bin dir yourself: `export PATH="$HOME/.wandr/bin:$PATH"`.
- **Video won't play** — install GStreamer (section 1).
- **`install failed` / `Not a directory` when installing an app** — your runtime
  predates app-archive support. Re-run the install command (section 4) to update
  `wandr-host`, then try again.
- **Use a different registry** — set `WANDR_REGISTRY` to another `index.json`
  URL or a local file path.
