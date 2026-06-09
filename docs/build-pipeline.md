# Build pipeline, WIT sync, environment

_The Kotlin→cwasm pipeline, the adapter, the WIT-sync rule, and the dev
environment. Read this for any build / WIT / cwasm / deploy work._

## Key build decisions

- **Hot-reload workflow:**
  ```bash
  adb push skiko-component.cwasm \
    "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
  # then restart the app — no APK rebuild
  ```
  Downloads directory is blocked by scoped storage.
  Use the app-specific external dir above (no permission needed).

- **Build pipeline** (Kotlin → cwasm). Full step-by-step in
  `~/wandr/apps/user/wandr-app/BUILD.md`; minimal form:
  ```bash
  # 1. (only if you changed Skiko itself) republish skiko-wasm-wasi.klib (~1m 40s)
  cd ~/wandr/external/skiko/skiko
  ./gradlew publishWasmWasiPublicationToMavenLocal \
      -Pskiko.wasmWasi.enabled=true \
      -Dorg.gradle.configureondemand=false \
      --console=plain --no-daemon

  # 2. compile the app to .wasm (links against the Compose wasi port — ~2 min)
  cd ~/wandr/apps/user/wandr-app
  ./gradlew compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon

  # 3. embed WIT + adapt P1→P2
  wasm-tools component embed \
      --world wandr:app/wandr-app \
      ~/wandr/apps/user/wandr-app/wit \
      build/compileSync/wasmWasi/main/productionExecutable/kotlin/wandr-app.wasm \
      -o /tmp/embedded.wasm
  # ⚠ Use the wandr-tree fork of the wasi preview1 reactor adapter, NOT
  # ~/wandr/external/skiko/wasi_snapshot_preview1.reactor.wasm. The wandr fork
  # patches `State::new` to place the adapter's 64 KB `State` at the
  # fixed address [0x10000,0x20000) instead of via `cabi_realloc` — the
  # KT-86415 Option B fix (task 34). It must be paired with the
  # `2.4.258-SNAPSHOT` Kotlin stdlib (root ScopedMemoryAllocator starts
  # at RESERVED_BASE=0x20000), wired via the init.d override. Mismatched
  # halves = State corruption / SIGILL. See [[kotlin-wasm-scopedmemory-destroy-bug]].
  # Build once (release profile, ~54 KB stripped):
  #   cd ~/wandr/external/wasmtime && cargo build \
  #     -p wasi-preview1-component-adapter \
  #     --target wasm32-unknown-unknown --release
  wasm-tools component new /tmp/embedded.wasm \
      --adapt ~/wandr/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm \
      -o /tmp/skiko-component.wasm

  # 4. pack + install + run via the Hybrid stack (replaces the old
  #    APK + NativeActivity path — task 35 + 46)
  bash ~/wandr/tools/scripts/build-system-wandrpkgs.sh   # all system wandrpkgs + wandr-app
  bash ~/wandr/tools/scripts/run-hybrid-stack.sh       # wandr-host --zygote + wandr-arbiter
  adb shell "su -c '/data/local/tmp/wandr-arbiter launch com.example.wandr-app'"
  ```

## WIT sync rule

**Whenever `wit/skiko-gfx.wit` changes, sync to the skiko submodule
and any consumer wandrpkg's `wit/deps/skiko-gfx/`:**

```bash
cp ~/wandr/wit/skiko-gfx.wit \
   ~/wandr/external/skiko/skiko/wit/skiko-gfx.wit
# Plus mirror to each wandrpkg that imports skiko-ui:
cp ~/wandr/wit/skiko-gfx.wit ~/wandr/apps/user/wandr-app/wit/deps/skiko-gfx/skiko-gfx.wit
cp ~/wandr/wit/skiko-gfx.wit ~/wandr/apps/system/wandr.ime.keyboard/wit/deps/skiko-gfx/skiko-gfx.wit
```

Then regenerate or hand-edit the Kotlin bindings in
`external/skiko/skiko/src/wasmWasiMain/kotlin/generated/`.

## Environment

- Rust toolchain with `aarch64-linux-android` target
- Android NDK r27 at `~/android-ndk-r27d`
- `adb` connected to rooted Android device (API 29+, arm64)
- `wasmtime` CLI on dev machine
- `wasm-tools` at `~/.cargo/bin/wasm-tools`
- WASI adapter at `~/wandr/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm` (wandr fork — KT-86415 Option B patch)
- Kotlin/Gradle: build skiko + Compose port from `~/wandr/external/skiko/` and `~/wandr/external/compose-multiplatform-core/` (wasmWasi-capable compiler in mavenLocal)
- Java 17+, Gradle 8+
