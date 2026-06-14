# signal-link — task-67 Signal client (link + receive), wasm guest over wasi:tls

A `wasm32-wasip2` guest that links as a Signal **secondary device** and **receives +
decrypts** an end-to-end-encrypted 1:1 text — with **no crypto or networking
compiled into the guest**. All transport runs over task-66 host-delegated
`wasi:tls` via our libsignal-service-rs fork (`external/libsignal-service-rs`,
transport swapped to wasi:tls in `wandr-wasi-shims/`).

## What it does
- `do_link` → `provisioning::link_device` over the provisioning websocket; prints
  the `sgnl://linkdevice` URL/QR, then (after you scan) decrypts the
  `ProvisionMessage`, uploads prekeys, registers the device.
- seeds the in-memory store (`store.rs`, a full `ProtocolStore` as `Rc<Inner>`)
  with the provisioned ACI identity.
- `do_receive` → authenticated `/v1/websocket/` (`MessageReceiver::create_message_pipe`)
  → `ServiceCipher::open_envelope` decrypts the first incoming text (a `DataMessage`,
  or your own `Note to Self` sync transcript).

Store is in-memory (v1) ⇒ link + receive happen in one run, and each run orphans a
linked device (prune old "wandr" devices in Signal → Linked Devices).

## Proven live (2026-05-30)
- **Desktop** via the Signal-CA runner (`../wasi-tls-runner`): linked, decrypted a
  real 1:1 text ("How are you wandr") + the typing indicator.
- **On device** (Pixel 2 XL) through the production **wandr-host** as a wandrpkg:
  forked, linked (`LINKED ✓`), opened the message socket, decrypted an incoming
  message (`MESSAGE ✓`) — entirely on aarch64-android, transport over the device's
  network + Signal CA.

## Build + run

Desktop:
```bash
PROTOC=~/tools/protoc/bin/protoc cargo build --target wasm32-wasip2 --release
(cd ../wasi-tls-runner && cargo run --release -- \
    ../signal-link/target/wasm32-wasip2/release/signal-link.wasm)
```

On device (wandrpkg through wandr-host):
```bash
PKG=/tmp/signal-link.wandrpkg; rm -rf "$PKG"; mkdir -p "$PKG/components"
cp package.toml "$PKG/package.toml"
cp target/wasm32-wasip2/release/signal-link.wasm "$PKG/components/signal-link.wasm"
adb push "$PKG" /data/local/tmp/signal-link.wandrpkg
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=/data/local/tmp/wandr-apps /data/local/tmp/wandr-host --install /data/local/tmp/signal-link.wandrpkg'"
adb logcat -c
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=/data/local/tmp/wandr-apps /data/local/tmp/wandr-host --zygote-launch wandr.signal.link'"
adb logcat -d | grep signal-link    # grab the sgnl:// URL
```

## Gotchas (load-bearing)
- **QR:** the terminal unicode QR is NOT phone-scannable — render a PNG from the
  printed `sgnl://` URL: `qrencode -s 12 -m 4 -o /tmp/q.png "<url>"` (on WSL,
  `xdg-open` shows it in Windows). **Scan fast** — the provisioning window is ~30-60s.
- **logcat:** the host's logcat sink emits one line per newline-terminated write;
  `eprintln!` with multiple args splits into several writes and only the first
  surfaces. Build the whole line (incl `\n`) and emit with one `eprint!("{line}")`.
- RNG = ChaCha20 seeded from `getrandom` (wasi); QR via `qrcode`.

See `tasks/67-signal-client.md` + `[[project_signal_wasip2_transport_swap]]`.
