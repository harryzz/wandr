#!/usr/bin/env bash
# Build + package + push + install all four warpkgs that the Hybrid runtime
# stack expects on-device (task 46 step 5 helper).
#
# Bundles (3 system + 1 user app):
#   war.markdown.renderer   ← markdown-renderer/      (system)
#   war.emoji.picker        ← emoji-picker/           (system)
#   war.fonts.loader        ← system-fonts/           (system)
#   com.example.wart-app    ← wart-app/ (Kotlin/Wasm) (user)
#
# Skips Kotlin build of wart-app — assumes /tmp/skiko-component.wasm is
# already produced via scripts/standalone-launch.sh's Kotlin/Wasm pipeline
# (Gradle compile → wasm-tools component embed → wasm-tools component new).
# If missing, prints instructions and bails.
#
# Output goes to $APPS_ROOT (default /data/local/tmp/wart-apps), readable
# by wart-host --zygote via WART_APPS_ROOT.
#
# Usage:
#   scripts/build-system-warpkgs.sh
#   APPS_ROOT=/data/wart scripts/build-system-warpkgs.sh   # production root
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APPS_ROOT="${APPS_ROOT:-/data/local/tmp/wart-apps}"
TMP_BASE="${TMP_BASE:-/tmp}"
WART_APP_WASM="${WART_APP_WASM:-/tmp/skiko-component.wasm}"

echo "▸ APPS_ROOT (device): $APPS_ROOT"
echo "▸ wart-host binary: $REPO_ROOT/runtime/wart-host/target/aarch64-linux-android/release/wasm-android-host"

WART_HOST="$REPO_ROOT/runtime/wart-host/target/aarch64-linux-android/release/wasm-android-host"
if [[ ! -x "$WART_HOST" ]]; then
    echo "✗ wart-host binary missing — run scripts/build-host-android.sh first" >&2
    exit 1
fi

# ── 1. Build each system component crate to wasm32-wasip2 ───────────────

build_system_wasm() {
    local crate_dir="$1" wasm_name="$2"
    # Diagnostics → stderr; only the path goes to stdout (so `$(...)`
    # capture in the caller doesn't end up concatenating log lines).
    echo "▸ building system component: $crate_dir" >&2
    ( cd "$REPO_ROOT/$crate_dir" && cargo build --target wasm32-wasip2 --release ) >&2
    local out="$REPO_ROOT/$crate_dir/target/wasm32-wasip2/release/$wasm_name"
    [[ -f "$out" ]] || { echo "✗ build missed: $out" >&2; exit 1; }
    echo "  $(du -h "$out" | cut -f1)  $out" >&2
    printf '%s' "$out"
}

MD_WASM=$(build_system_wasm "apps/system/war.markdown.renderer" "markdown_renderer.wasm")
EM_WASM=$(build_system_wasm "apps/system/war.emoji.picker"      "emoji_picker.wasm")
# Task 57 — the dedicated launcher: a light Rust canvas guest (exports
# my:skiko-gfx/renderer; ~70 KB; no Kotlin/Compose → leak-immune).
LN_WASM=$(build_system_wasm "apps/system/war.launcher"          "war_launcher.wasm")
# Task 55 — the status bar: light Rust top-overlay canvas guest (~48 KB).
SB_WASM=$(build_system_wasm "apps/system/war.statusbar"         "war_statusbar.wasm")
FT_WASM=$(build_system_wasm "apps/system/war.fonts.loader"      "system_fonts.wasm")
BG_WASM=$(build_system_wasm "apps/system/lang/war.lang.bg"       "war_lang_bg.wasm")
FR_WASM=$(build_system_wasm "apps/system/lang/war.lang.fr"       "war_lang_fr.wasm")

# ── 2. Package each warpkg directory ────────────────────────────────────

pack_warpkg() {
    local pkg_dir="$1" wasm_src="$2" comp_name="$3" toml_body="$4"
    rm -rf "$pkg_dir"
    mkdir -p "$pkg_dir/components"
    cp "$wasm_src" "$pkg_dir/components/$comp_name.wasm"
    printf '%s\n' "$toml_body" > "$pkg_dir/package.toml"
    echo "  packaged: $pkg_dir"
}

MD_PKG="$TMP_BASE/markdown.warpkg"
EM_PKG="$TMP_BASE/emoji.warpkg"
FT_PKG="$TMP_BASE/fonts.warpkg"
BG_PKG="$TMP_BASE/lang-bg.warpkg"
FR_PKG="$TMP_BASE/lang-fr.warpkg"
APP_PKG="$TMP_BASE/wart-app.warpkg"
LN_PKG="$TMP_BASE/launcher.warpkg"
SB_PKG="$TMP_BASE/statusbar.warpkg"

pack_warpkg "$MD_PKG" "$MD_WASM" "renderer" "$(cat <<'EOF'
app_id      = "war.markdown.renderer"
version     = "0.1.0"
world       = "war:markdown/renderer-world"
kind        = "system"
composition = "same-store"

[components]
renderer = "components/renderer.wasm"
EOF
)"

pack_warpkg "$EM_PKG" "$EM_WASM" "picker" "$(cat <<'EOF'
app_id      = "war.emoji.picker"
version     = "0.1.0"
world       = "war:emoji/picker-world"
kind        = "system"
composition = "same-store"

[components]
picker = "components/picker.wasm"
EOF
)"

pack_warpkg "$FT_PKG" "$FT_WASM" "loader" "$(cat <<'EOF'
app_id      = "war.fonts.loader"
version     = "0.1.0"
world       = "war:fonts/loader-world"
kind        = "system"
composition = "same-store"

[components]
loader = "components/loader.wasm"
EOF
)"

# Task 57 — the launcher installs as a (launchable) app under apps/ so
# the arbiter's set-home/launch path finds it; it filters its own id out
# of its grid. `ui` component name matches the GUI-app convention.
pack_warpkg "$LN_PKG" "$LN_WASM" "ui" "$(cat <<'EOF'
app_id      = "war.launcher"
version     = "0.1.0"
world       = "my:skiko-gfx/skiko-ui"
composition = "same-store"
label       = "Launcher"

[components]
ui = "components/ui.wasm"
EOF
)"

# Task 55 — the status bar installs under system-apps/ (kind=system) so
# the launcher's apps/-only scan doesn't list it as a tile; the loader's
# AppRef::Installed search falls back to system-apps/ for the
# --standalone-overlay-top load.
pack_warpkg "$SB_PKG" "$SB_WASM" "ui" "$(cat <<'EOF'
app_id      = "war.statusbar"
version     = "0.1.0"
world       = "my:skiko-gfx/skiko-ui"
kind        = "system"
composition = "same-store"
label       = "Status Bar"

[components]
ui = "components/ui.wasm"
EOF
)"

pack_warpkg "$BG_PKG" "$BG_WASM" "lang" "$(cat <<'EOF'
app_id      = "war.lang.bg"
version     = "0.1.0"
world       = "war:keyboard-lang-bg/lang-world"
kind        = "system"
composition = "same-store"

[components]
lang = "components/lang.wasm"
EOF
)"

pack_warpkg "$FR_PKG" "$FR_WASM" "lang" "$(cat <<'EOF'
app_id      = "war.lang.fr"
version     = "0.1.0"
world       = "war:keyboard-lang-fr/lang-world"
kind        = "system"
composition = "same-store"

[components]
lang = "components/lang.wasm"
EOF
)"

# wart-app: use the existing post-adapt .wasm. If missing, the user needs
# to run the Kotlin pipeline (see scripts/standalone-launch.sh fallback
# text + wart-app/BUILD.md).
if [[ ! -f "$WART_APP_WASM" ]]; then
    echo ""
    echo "✗ $WART_APP_WASM not found." >&2
    echo "  Build it via scripts/standalone-launch.sh's Kotlin pipeline:" >&2
    echo "    cd wart-app && ./gradlew compileProductionExecutableKotlinWasmWasi" >&2
    echo "    wasm-tools component embed --world my:skiko-gfx/skiko-ui \\" >&2
    echo "        wit/skiko-gfx.wit \\" >&2
    echo "        wart-app/build/.../wart-app.wasm -o /tmp/embedded.wasm" >&2
    echo "    wasm-tools component new /tmp/embedded.wasm \\" >&2
    echo "        --adapt wasmtime-src/.../wasi_snapshot_preview1.wasm \\" >&2
    echo "        -o $WART_APP_WASM" >&2
    exit 1
fi

pack_warpkg "$APP_PKG" "$WART_APP_WASM" "ui" "$(cat <<'EOF'
app_id      = "com.example.wart-app"
version     = "0.1.0"
world       = "my:skiko-gfx/skiko-ui"
composition = "same-store"
# Task 57 — human-readable label shown by the launcher (`list-apps`).
label       = "Demo"

[components]
ui = "components/ui.wasm"

# Cross-app deps wart-app actually imports — task 36 / 39 / 41. The
# installer reads these to populate dependencies_resolved in the
# cache-key, which load_dep_components walks at launch time to wire
# each dep into the consumer's linker.
[dependencies]
markdown = { system = "war.markdown.renderer", version = "0.1.0", interface = "war:markdown/renderer@0.1.0" }
emoji    = { system = "war.emoji.picker",      version = "0.1.0", interface = "war:emoji/picker@0.1.0" }
fonts    = { system = "war.fonts.loader",      version = "0.1.0", interface = "war:fonts/loader@0.1.0" }
EOF
)"

# ── 3. Push warpkg dirs to device + install ─────────────────────────────

echo ""
echo "▸ pushing warpkg dirs to device …"
# adb push of dir-onto-dir nests; rm the device-side copy first.
for pkg in "$MD_PKG" "$EM_PKG" "$FT_PKG" "$BG_PKG" "$FR_PKG" "$LN_PKG" "$SB_PKG" "$APP_PKG"; do
    name="$(basename "$pkg")"
    adb shell "rm -rf /data/local/tmp/$name"
    adb push "$pkg" "/data/local/tmp/$name" >/dev/null
done

echo ""
echo "▸ installing via wart-host --install (system deps FIRST so app's"
echo "  import-resolution check passes) …"
adb shell "su -c 'rm -rf $APPS_ROOT && mkdir -p $APPS_ROOT'"

WART_ENV="LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT"
for pkg in markdown emoji fonts lang-bg lang-fr launcher statusbar wart-app; do
    echo "  install $pkg.warpkg"
    adb shell "su -c '$WART_ENV /data/local/tmp/wart-host --install /data/local/tmp/$pkg.warpkg'"
done

echo ""
echo "▸ on-device layout:"
adb shell "ls -1 $APPS_ROOT/system-apps/ 2>/dev/null"
adb shell "ls -1 $APPS_ROOT/apps/ 2>/dev/null"

echo ""
echo "✓ ready. Start the Hybrid stack: scripts/run-hybrid-stack.sh"
