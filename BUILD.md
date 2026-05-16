# wart-app — Build & Deploy

End-to-end pipeline: Kotlin source → `.wasm` → WASM Component → AOT
`.cwasm` for `aarch64-linux-android` → push to device → run.

> **TL;DR for an impatient day-to-day build:**
> ```bash
> cd ~/wart/wart-app && ./gradlew wasmWasiProductionExecutable
> bash ~/wart/scripts/build-aot.sh   # if it exists; otherwise see §3
> ```

---

## 0. Prerequisites

| Tool | Path / version |
|------|----------------|
| Kotlin | 2.4.0-RC (declared in `build.gradle.kts`) |
| `wasm-tools` | `~/.cargo/bin/wasm-tools` |
| `wasmtime` CLI | recent (component-model + gc + function-references + exceptions) |
| WASI Preview 2 reactor adapter | `~/skiko/wasi_snapshot_preview1.reactor.wasm` |
| `adb` | connected to a rooted Android device (arm64, API 29+) |
| Compose-MP klibs in `~/.m2` | published from `~/wart/compose-multiplatform-core/` and the 11 sibling dirs |
| skiko-wasm-wasi klib in `~/.m2` | published from `~/skiko/` |

---

## 1. CRITICAL — dependency strategy (READ THIS FIRST)

We have **two** flavours of the same Compose port published to `~/.m2`:

### A. 32 granular klibs (one per upstream module) — DO NOT USE FOR LINKING

```
androidx.compose.ui:ui-wasm-wasi:9999.0.0-SNAPSHOT
androidx.compose.ui:ui-graphics-wasm-wasi:9999.0.0-SNAPSHOT
androidx.compose.ui:ui-text-wasm-wasi:9999.0.0-SNAPSHOT
... (29 more)
```

These come straight out of `~/wart/compose-multiplatform-core/`'s
own publish tasks. They are correct, granular, and reusable as upstream-style
libraries — **but linking against them takes ~2 hours.**

The Kotlin/Wasm linker (`WholeWorldCompilerBase`) performs **O(N³) cross-klib
symbol resolution** during whole-world IR lowering. With N=32 you wait hours.

### B. 11 sibling "fat" klibs — USE THESE

Located in:
```
~/wart/compose-runtime-wasi/
~/wart/compose-ui-base-wasi/
~/wart/compose-ui-graphics-wasi/
~/wart/compose-ui-text-wasi/
~/wart/compose-ui-wasi/
~/wart/compose-foundation-layout-wasi/
~/wart/compose-foundation-wasi/
~/wart/compose-animation-core-wasi/
~/wart/compose-animation-wasi/
~/wart/compose-material-ripple-wasi/
~/wart/compose-material3-wasi/
```

Each of these dirs has a `build.gradle.kts` that bundles **multiple upstream
modules' source dirs** via `srcDirs` pointing back into
`~/wart/compose-multiplatform-core/`. The sources are the same; the
packaging is different. Each one publishes:

```
androidx.compose.<group>:compose-<group>[-base|-layout|-...]-wasi:0.0.0-wasi-local
```

With **N=11**, link time drops to **~5 minutes** ((32/11)³ ≈ 24× faster, which
matches what we measured: 5 min vs 2 h+).

`wart-app/build.gradle.kts` is wired against this flavour. **Do not** add
direct dependencies on the 32 granular klibs.

### How they coexist

If a transitive resolution pulls in a granular klib by coord (which happens
because the JetBrains-published metadata still references them), `build.gradle.kts`
contains a wall of `resolutionStrategy.dependencySubstitution { ... }` blocks
that redirect each one to the appropriate sibling artifact, plus `exclude(...)`
clauses to keep the original androidx maven coords out of the graph entirely.

**If you add a new Compose dependency**, you almost certainly need to add a
matching `substitute(...)` and `exclude(...)` line. Symptoms of forgetting:
- `Could not find org.jetbrains.androidx.X:Y:9999.0.0-SNAPSHOT` (umbrella missing → add substitute to `-wasm-wasi`)
- 2-hour link time (granular klib leaked into the classpath)
- `KLIB loader: The same 'unique_name=...' found in more than one library` (both flavours of same module made it in → tighten exclude)

### Why `configurations.matching { ... }` instead of `configurations.all`?

`configurations.all { ... }` eagerly realizes **every** KMP configuration the
plugin lazily creates (~80+ for a wasi target), and turns configure time from
2 s into 6+ minutes of silent CPU burn before any compile starts. The
`matching { it.name.startsWith("wasmWasi") }.configureEach { ... }` form only
touches the configurations we actually consume.

---

## 2. Compile Kotlin → `.wasm`

```bash
cd ~/wart/wart-app
./gradlew wasmWasiProductionExecutable --console=plain --no-daemon
```

Output: `build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm`
(~11 MB).

Cold link: ~55 s. Incremental (single-file change in `Main.kt`): ~15 s.

If it hangs >2 minutes with no log output, you've probably re-introduced one
of the granular klibs. Check the dependency tree:

```bash
./gradlew :dependencies --configuration wasmWasiCompileClasspath | grep -E ' wasm-wasi:9999'
```

If you see lines without a corresponding substitute, that's the leak.

---

## 3. Wrap as WASM Component (WASI Preview 2)

The raw `.wasm` is a WASI Preview 1 core module. We need a **Component** that
exports the `my:skiko-gfx/skiko-ui` world and adapts P1 imports to P2.

```bash
WIT=~/wart/wit/skiko-gfx.wit
WASM_IN=~/wart/wart-app/build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm
ADAPTER=~/skiko/wasi_snapshot_preview1.reactor.wasm
OUT_DIR=/tmp/wart-aot
mkdir -p "$OUT_DIR"

# 1) Embed the WIT into the core module so `component new` knows the world.
wasm-tools component embed \
    --world my:skiko-gfx/skiko-ui \
    "$WIT" \
    "$WASM_IN" \
    -o "$OUT_DIR/embedded.wasm"

# 2) Promote to component, adapting WASI P1 → P2 via the reactor adapter.
wasm-tools component new \
    "$OUT_DIR/embedded.wasm" \
    --adapt "$ADAPTER" \
    -o "$OUT_DIR/skiko-component.wasm"
```

Both outputs are ~11 MB.

> **Don't skip `component embed`** — `component new` will fail without a
> world annotation.

---

## 4. AOT-compile for the device

Wasmtime on Android can't JIT (SELinux W^X without root tricks), so we AOT.

```bash
wasmtime compile \
    --target aarch64-linux-android \
    --wasm component-model \
    --wasm gc \
    --wasm function-references \
    --wasm exceptions \
    -o "$OUT_DIR/skiko-component.cwasm" \
    "$OUT_DIR/skiko-component.wasm"
```

Output: `skiko-component.cwasm` (~63 MB — most is precompiled aarch64 code).

The four `--wasm` flags are **all required**. Kotlin/Wasm output uses GC,
typed function refs, and exception handling proposals; without them you get
validation errors at host load time.

---

## 5. Push to device

The Android app reads from its app-specific external storage. **Do not push
to `/sdcard/Download/`** — scoped storage blocks it.

```bash
adb push "$OUT_DIR/skiko-component.cwasm" \
    "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
```

Then start (or restart) the app:

```bash
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
```

> The activity is `android.app.NativeActivity`, not `MainActivity`.

For a hot-reload cycle during development, **you don't need to rebuild the
APK**. Push the new `.cwasm` and force-stop+start:

```bash
adb shell am force-stop com.example.wasmruntime
adb push "$OUT_DIR/skiko-component.cwasm" \
    "/sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm"
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
```

---

## 6. Verify it's working

Tail logcat with a tag filter:

```bash
adb logcat -c
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
adb logcat | grep -iE "wasm_android|wasmtime|FATAL|AndroidRuntime"
```

A healthy boot looks like:

```
wasm_android_host: resumed (warm) — swapping renderer in existing store
wasmtime::runtime::vm..: FreeList::add_capacity(...): capacity growing from ...
wasm_android_host::ca..: [wasm] tfstate text="..." sel=TextRange(...)
```

The last line means the BasicTextField is receiving keystroke events. Try
`adb shell input keyevent KEYCODE_A` to send a hardware-keyboard 'a' into the
focused field and watch the `tfstate` log line.

---

## 7. End-to-end one-liner

Once you have the prerequisites, the whole cycle is:

```bash
cd ~/wart/wart-app && \
    ./gradlew wasmWasiProductionExecutable --console=plain --no-daemon && \
mkdir -p /tmp/wart-aot && \
wasm-tools component embed \
    --world my:skiko-gfx/skiko-ui \
    ~/wart/wit/skiko-gfx.wit \
    build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm \
    -o /tmp/wart-aot/embedded.wasm && \
wasm-tools component new /tmp/wart-aot/embedded.wasm \
    --adapt ~/skiko/wasi_snapshot_preview1.reactor.wasm \
    -o /tmp/wart-aot/skiko-component.wasm && \
wasmtime compile --target aarch64-linux-android \
    --wasm component-model --wasm gc --wasm function-references --wasm exceptions \
    -o /tmp/wart-aot/skiko-component.cwasm \
    /tmp/wart-aot/skiko-component.wasm && \
adb shell am force-stop com.example.wasmruntime && \
adb push /tmp/wart-aot/skiko-component.cwasm \
    /sdcard/Android/data/com.example.wasmruntime/files/skiko-component.cwasm && \
adb shell am start -n com.example.wasmruntime/android.app.NativeActivity
```

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Link step runs 2+ hours, low CPU | Granular `:9999.0.0-SNAPSHOT` klib leaked into classpath | Run `./gradlew :dependencies --configuration wasmWasiCompileClasspath`; add missing `substitute(...)` or `exclude(...)` |
| `Could not find org.jetbrains.androidx.X:Y:9999.0.0-SNAPSHOT` | Umbrella publication doesn't exist; only `-wasm-wasi` does | Add `substitute(module("X:Y")).using(module("X:Y-wasm-wasi:9999.0.0-SNAPSHOT"))` |
| `KLIB loader: same unique_name found in more than one library` | Two flavours of the same module on classpath | Tighten the relevant `exclude(...)` or remove a duplicate `api(...)` |
| `IrClassSymbolImpl is already bound. Signature: ...` | Two source sets compiled the same upstream `.kt` file | Remove the duplicate `srcDir(...)` from one sibling's build.gradle.kts |
| `wasm-tools component new` fails with "no world" | Skipped `component embed` step | Run `component embed` first |
| `wasmtime compile` errors on validation | Missing one of `--wasm gc / function-references / exceptions` | Pass all four `--wasm` flags |
| App starts but screen stays at splash | Wrong cwasm path or activity name | Confirm push path is `/sdcard/Android/data/com.example.wasmruntime/files/` and activity is `android.app.NativeActivity` |
| Hangs >2 min at "Configuring root project" with no further log | `configurations.all { ... }` eagerly realizing all configurations | Switch to `configurations.matching { it.name.startsWith("wasmWasi") }.configureEach` |
| Gradle daemon crashes mid-build (OOM) | Default heap too small for KMP+Compose | `gradle.properties`: `org.gradle.jvmargs=-Xmx4g -XX:MaxMetaspaceSize=1g` |

---

## When you need to rebuild the sibling klibs

If you change a source file under `~/wart/compose-multiplatform-core/`,
the sibling klib that includes it via `srcDirs` needs to be republished:

```bash
cd ~/wart/compose-ui-wasi   # or whichever sibling owns the changed file
./gradlew publishToMavenLocal -Dorg.gradle.configureondemand=false
```

Then re-run §2 above. (No need to touch the 32 granular klibs unless you're
specifically updating that flavour.)

Build order matters when republishing multiple siblings — runtime first,
ui-base/graphics/text next, ui after them, foundation-layout/animation-core
before foundation/animation, material-ripple before material3.
