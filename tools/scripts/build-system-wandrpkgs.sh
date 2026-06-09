#!/usr/bin/env bash
# Build + package + push + install all four wandrpkgs that the Hybrid runtime
# stack expects on-device (task 46 step 5 helper).
#
# Bundles (3 system + 1 user app):
#   wandr.markdown.renderer   ← markdown-renderer/      (system)
#   wandr.emoji.picker        ← emoji-picker/           (system)
#   wandr.fonts.loader        ← system-fonts/           (system)
#   com.example.wandr-app    ← wandr-app/ (Kotlin/Wasm) (user)
#
# Skips Kotlin build of wandr-app — assumes /tmp/skiko-component.wasm is
# already produced via scripts/standalone-launch.sh's Kotlin/Wasm pipeline
# (Gradle compile → wasm-tools component embed → wasm-tools component new).
# If missing, prints instructions and bails.
#
# Output goes to $APPS_ROOT (default /data/local/tmp/wandr-apps), readable
# by wandr-host --zygote via WANDR_APPS_ROOT.
#
# Usage:
#   scripts/build-system-wandrpkgs.sh
#   APPS_ROOT=/data/wandr scripts/build-system-wandrpkgs.sh   # production root
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APPS_ROOT="${APPS_ROOT:-/data/local/tmp/wandr-apps}"
TMP_BASE="${TMP_BASE:-/tmp}"
WANDR_APP_WASM="${WANDR_APP_WASM:-/tmp/skiko-component.wasm}"

echo "▸ APPS_ROOT (device): $APPS_ROOT"
echo "▸ wandr-host binary: $REPO_ROOT/runtime/wandr-host/target/aarch64-linux-android/release/wasm-android-host"

WANDR_HOST="$REPO_ROOT/runtime/wandr-host/target/aarch64-linux-android/release/wasm-android-host"
if [[ ! -x "$WANDR_HOST" ]]; then
    echo "✗ wandr-host binary missing — run scripts/build-host-android.sh first" >&2
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

MD_WASM=$(build_system_wasm "apps/system/wandr.markdown.renderer" "markdown_renderer.wasm")
EM_WASM=$(build_system_wasm "apps/system/wandr.emoji.picker"      "emoji_picker.wasm")
# Task 57 — the dedicated launcher: a light Rust canvas guest (exports
# my:skiko-gfx/renderer; ~70 KB; no Kotlin/Compose → leak-immune).
LN_WASM=$(build_system_wasm "apps/system/wandr.launcher"          "wandr_launcher.wasm")
# Task 55 — the status bar: light Rust top-overlay canvas guest (~48 KB).
SB_WASM=$(build_system_wasm "apps/system/wandr.statusbar"         "wandr_statusbar.wasm")
# Task 56 — the taskbar: light Rust bottom-nav canvas guest (Back/Home/Recents).
TB_WASM=$(build_system_wasm "apps/system/wandr.taskbar"           "wandr_taskbar.wasm")
# Keyguard/lockscreen — light Rust overlay-lock canvas guest; kind=system so the
# launcher's apps/-only scan skips it; loaded via --standalone-overlay-lock.
KG_WASM=$(build_system_wasm "apps/system/wandr.keyguard"          "wandr_keyguard.wasm")
FT_WASM=$(build_system_wasm "apps/system/wandr.fonts.loader"      "system_fonts.wasm")
BG_WASM=$(build_system_wasm "apps/system/lang/wandr.lang.bg"       "wandr_lang_bg.wasm")
FR_WASM=$(build_system_wasm "apps/system/lang/wandr.lang.fr"       "wandr_lang_fr.wasm")
# Task 59 — the dioxus demo: a reactive Rust guest rendered via the
# dioxus-canvas "tiny Blitz" over the canvas WIT (~516 KB; user app).
DX_WASM=$(build_system_wasm "apps/user/wandr.dioxus.demo"          "wandr_dioxus_demo.wasm")

# ── 2. Package each wandrpkg directory ────────────────────────────────────

pack_wandrpkg() {
    local pkg_dir="$1" wasm_src="$2" comp_name="$3" toml_src="$4" assets_src="${5:-}"
    rm -rf "$pkg_dir"
    mkdir -p "$pkg_dir/components"
    cp "$wasm_src" "$pkg_dir/components/$comp_name.wasm"
    # The manifest lives in the app's source dir (apps/.../package.toml) —
    # single source of truth, edited like any file instead of regenerated
    # from a heredoc here.
    cp "$toml_src" "$pkg_dir/package.toml"
    # Task 38 — bundle an optional assets/ dir; the installer copies it
    # verbatim to <install_dir>/assets/ for the assets.read host verb.
    if [[ -n "$assets_src" && -d "$assets_src" ]]; then
        cp -r "$assets_src" "$pkg_dir/assets"
        echo "  + assets: $assets_src → $pkg_dir/assets"
    fi
    echo "  packaged: $pkg_dir"
}

MD_PKG="$TMP_BASE/markdown.wandrpkg"
EM_PKG="$TMP_BASE/emoji.wandrpkg"
FT_PKG="$TMP_BASE/fonts.wandrpkg"
BG_PKG="$TMP_BASE/lang-bg.wandrpkg"
FR_PKG="$TMP_BASE/lang-fr.wandrpkg"
APP_PKG="$TMP_BASE/wandr-app.wandrpkg"
LN_PKG="$TMP_BASE/launcher.wandrpkg"
SB_PKG="$TMP_BASE/statusbar.wandrpkg"
TB_PKG="$TMP_BASE/taskbar.wandrpkg"
KG_PKG="$TMP_BASE/keyguard.wandrpkg"
DX_PKG="$TMP_BASE/dioxus-demo.wandrpkg"

# Each manifest is the app's own apps/.../package.toml (single source of
# truth). pack_wandrpkg copies it into the wandrpkg; the 3rd arg is the
# component name and must match the toml's [components] entry.
pack_wandrpkg "$MD_PKG" "$MD_WASM" "renderer" "$REPO_ROOT/apps/system/wandr.markdown.renderer/package.toml"
pack_wandrpkg "$EM_PKG" "$EM_WASM" "picker"   "$REPO_ROOT/apps/system/wandr.emoji.picker/package.toml"
pack_wandrpkg "$FT_PKG" "$FT_WASM" "loader"   "$REPO_ROOT/apps/system/wandr.fonts.loader/package.toml"
pack_wandrpkg "$LN_PKG" "$LN_WASM" "ui"       "$REPO_ROOT/apps/system/wandr.launcher/package.toml"
pack_wandrpkg "$SB_PKG" "$SB_WASM" "ui"       "$REPO_ROOT/apps/system/wandr.statusbar/package.toml"
pack_wandrpkg "$TB_PKG" "$TB_WASM" "ui"       "$REPO_ROOT/apps/system/wandr.taskbar/package.toml"
pack_wandrpkg "$KG_PKG" "$KG_WASM" "ui"       "$REPO_ROOT/apps/system/wandr.keyguard/package.toml"
pack_wandrpkg "$DX_PKG" "$DX_WASM" "ui"       "$REPO_ROOT/apps/user/wandr.dioxus.demo/package.toml"
pack_wandrpkg "$BG_PKG" "$BG_WASM" "lang"     "$REPO_ROOT/apps/system/lang/wandr.lang.bg/package.toml"
pack_wandrpkg "$FR_PKG" "$FR_WASM" "lang"     "$REPO_ROOT/apps/system/lang/wandr.lang.fr/package.toml"

# wandr-app: use the existing post-adapt .wasm. If missing, the user needs
# to run the Kotlin pipeline (see scripts/standalone-launch.sh fallback
# text + wandr-app/BUILD.md).
if [[ ! -f "$WANDR_APP_WASM" ]]; then
    echo ""
    echo "✗ $WANDR_APP_WASM not found." >&2
    echo "  Build it via scripts/standalone-launch.sh's Kotlin pipeline:" >&2
    echo "    cd wandr-app && ./gradlew compileProductionExecutableKotlinWasmWasi" >&2
    echo "    wasm-tools component embed --world my:skiko-gfx/skiko-ui \\" >&2
    echo "        wit/skiko-gfx.wit \\" >&2
    echo "        wandr-app/build/.../wandr-app.wasm -o /tmp/embedded.wasm" >&2
    echo "    wasm-tools component new /tmp/embedded.wasm \\" >&2
    echo "        --adapt wasmtime-src/.../wasi_snapshot_preview1.wasm \\" >&2
    echo "        -o $WANDR_APP_WASM" >&2
    exit 1
fi

pack_wandrpkg "$APP_PKG" "$WANDR_APP_WASM" "ui" \
    "$REPO_ROOT/apps/user/wandr-app/package.toml" \
    "$REPO_ROOT/apps/user/wandr-app/assets"

# ── 3. Push wandrpkg dirs to device + install ─────────────────────────────

echo ""
echo "▸ pushing wandrpkg dirs to device …"
# adb push of dir-onto-dir nests; rm the device-side copy first.
for pkg in "$MD_PKG" "$EM_PKG" "$FT_PKG" "$BG_PKG" "$FR_PKG" "$LN_PKG" "$SB_PKG" "$TB_PKG" "$KG_PKG" "$DX_PKG" "$APP_PKG"; do
    name="$(basename "$pkg")"
    adb shell "rm -rf /data/local/tmp/$name"
    adb push "$pkg" "/data/local/tmp/$name" >/dev/null
done

echo ""
echo "▸ installing via wandr-host --install (system deps FIRST so app's"
echo "  import-resolution check passes) …"
adb shell "su -c 'rm -rf $APPS_ROOT && mkdir -p $APPS_ROOT'"

WANDR_ENV="LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=$APPS_ROOT"
for pkg in markdown emoji fonts lang-bg lang-fr launcher statusbar taskbar keyguard dioxus-demo wandr-app; do
    echo "  install $pkg.wandrpkg"
    adb shell "su -c '$WANDR_ENV /data/local/tmp/wandr-host --install /data/local/tmp/$pkg.wandrpkg'"
done

# The IME (wandr.ime.keyboard) is packaged by the separate pack-ime-keyboard.sh
# (Kotlin pipeline), so the `rm -rf $APPS_ROOT` clean-install above would
# orphan it — leaving the soft keyboard "missing" until manually reinstalled.
# Reinstall it here if its build output exists (lang plugins are already
# installed just above, which pack-ime-keyboard.sh needs).
IME_WASM="$REPO_ROOT/apps/system/wandr.ime.keyboard/build/compileSync/wasmWasi/main/productionExecutable/kotlin/wandr-ime-keyboard.wasm"
if [[ -f "$IME_WASM" ]]; then
    echo ""
    echo "▸ reinstalling IME (wandr.ime.keyboard) so the wipe didn't orphan it …"
    bash "$REPO_ROOT/tools/scripts/pack-ime-keyboard.sh" >&2 \
        || echo "⚠ IME repack failed — run pack-ime-keyboard.sh manually." >&2
else
    echo ""
    echo "⚠ IME not built ($IME_WASM missing) — the soft keyboard is NOT installed." >&2
    echo "  Build it (wandr-app/BUILD.md Kotlin pipeline) + run pack-ime-keyboard.sh." >&2
fi

echo ""
echo "▸ on-device layout:"
adb shell "ls -1 $APPS_ROOT/system-apps/ 2>/dev/null"
adb shell "ls -1 $APPS_ROOT/apps/ 2>/dev/null"

echo ""
echo "✓ ready. Start the Hybrid stack: scripts/run-hybrid-stack.sh"
