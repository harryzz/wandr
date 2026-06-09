# wasi-tls reachability probe

De-risks the **transport layer** for a possible Signal-messenger wandr app: can a
`wasm32-wasip2` guest reach a TLS server using only **host-delegated**
`wasi:sockets` + `wasi:tls@0.2.0-draft` — i.e. with **no TLS/crypto compiled into
the guest**?

The guest (`src/main.rs`) does, per target host: DNS resolve → TCP connect →
`wasi:tls` ClientHandshake → HTTP/1.1 `GET /` → read response. It imports only
`wasi:sockets/*` + `wasi:tls/types` (verify: `wasm-tools component wit`).

## Build + run (desktop, wasmtime 45)

```bash
cargo build --target wasm32-wasip2 --release           # wasip2 emits a component directly
wasmtime run -S inherit-network -S allow-ip-name-lookup -S tls \
    target/wasm32-wasip2/release/wasi-tls-probe.wasm
```

(`wit-bindgen = "0.53"` — the 0.46 macro failed to parse the multi-package
`wit/` dep tree; the 0.53.1 CLI/lib parse it fine.)

## Result (2026-05-30, wasmtime 45.0.0)

```
[OK]   example.com      — resolved 104.20.x.x · 838 bytes · status: HTTP/1.1 200 OK
[FAIL] chat.signal.org  — tls handshake io-error: invalid peer certificate: UnknownIssuer
```

**Transport is proven.** The control host completes the full DNS→TCP→TLS→HTTP
round-trip entirely host-side. Signal reaches certificate verification and fails
only on the trust anchor: `chat.signal.org` is **certificate-pinned** to Signal's
own private CA, and wasmtime's default `RustlsProvider` trusts only the Mozilla
public bundle (`webpki_roots::TLS_SERVER_ROOTS`, `wasi-tls/src/providers/rustls.rs:34`).

## What this means / next step

Not a wandr or wasi-tls limitation — it's Signal's pinning. `wasi:tls`'s
`ClientHandshake::new(server-name, in, out)` has **no** way for the guest to pass
a custom CA; the trust store lives entirely in the host provider. Fix is
**host-side and small**: `wasmtime_wasi_tls::TlsProvider` is a public trait
(`wasi-tls/src/lib.rs:174`) — wandr-host supplies a provider whose `RootCertStore`
= public roots **+ Signal's pinned CA** (the same PEMs presage/libsignal-service-rs
bundle). TLS stays host-delegated; no crypto in the guest.

**Loop closed** by [`../wasi-tls-runner`](../wasi-tls-runner) (desktop, custom
provider) and by **wandr-host itself** (task 66 — Signal's CA wired into the
production host).

## Running it through wandr-host on-device (wandrpkg)

`package.toml` packages this as a `wasi:cli/command` system wandrpkg. Build →
install → launch headless via the zygote:

```bash
cargo build --target wasm32-wasip2 --release
PKG=/tmp/probe.wandrpkg; rm -rf "$PKG"; mkdir -p "$PKG/components"
cp package.toml "$PKG/package.toml"
cp target/wasm32-wasip2/release/wasi-tls-probe.wasm "$PKG/components/probe.wasm"
adb push "$PKG" /data/local/tmp/probe.wandrpkg
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=/data/local/tmp/wandr-apps \
    /data/local/tmp/wandr-host --install /data/local/tmp/probe.wandrpkg'"
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=/data/local/tmp/wandr-apps \
    /data/local/tmp/wandr-host --zygote-launch wandr.probe.wasitls'"
adb logcat -d | grep wasi-tls-probe
```

Device result (2026-05-30, through the production host):

```
signal_tls: trust store = 119 public roots + 1 Signal CA
[wasi-tls-probe] [OK]   example.com     - ... | HTTP/1.1 200 OK
[wasi-tls-probe] [OK]   chat.signal.org - ... | HTTP/1.1 404 Not Found
[wasi-tls-probe] TRANSPORT PROVEN ...
```

`chat.signal.org` handshakes through wandr-host's Signal-aware trust store; the
404 is the wrong path (`GET /`), irrelevant — the trusted handshake is the proof.

Note: the probe writes results to **stderr** as one `write()` per line — wandr's
LogcatStderr sink only surfaces the first `write()` of a multi-write line, so
`eprintln!` with `{}` args truncates after the literal prefix.
