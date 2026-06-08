#!/usr/bin/env bash
# Vendor the minimal AIDL set audioclient-rs needs into ./aidl/, for a
# SELF-CONTAINED / publishable build. Without this, build.rs reads the AOSP AIDLs
# from the wart-host submodule (../../runtime/wart-host/vendor); after running this,
# build.rs prefers the local ./aidl/ copy and the crate no longer depends on the
# wart tree layout.
#
# ‼️ The AOSP version MUST match the TARGET DEVICE's audioserver — the private
# `audio_track_cblk_t` struct and the AAudio AIDL transaction layout are
# version-specific. The default source is the wart project's pinned AOSP submodules
# (already device-matched). To vendor a different/newer AOSP: either re-pin those
# submodules first, or pass a source root as $1 (see below).
#
# Usage:
#   ./vendor-aidl.sh                 # copy from the wart-host submodule (default)
#   ./vendor-aidl.sh /path/to/src    # copy from a custom source root that contains
#                                    #   aosp-frameworks-av/ and
#                                    #   aosp-system-hardware-interfaces/ (+ aidl-stubs/)
#
# For a RAW AOSP checkout (frameworks/av, system/hardware/interfaces) the dir names
# differ; arrange them under a root matching the layout below, or adjust SRC paths.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${1:-$HERE/../../runtime/wart-host/vendor}"
DST="$HERE/aidl"

# (src-relative path, dst-relative path) — must match the subpaths build.rs reads.
PAIRS=(
  "aosp-frameworks-av/media/libaudioclient/aidl|aosp-frameworks-av/media/libaudioclient/aidl"
  "aosp-frameworks-av/media/libshmem/aidl|aosp-frameworks-av/media/libshmem/aidl"
  "aosp-frameworks-av/aidl|aosp-frameworks-av/aidl"
  "aosp-system-hardware-interfaces/media/aidl|aosp-system-hardware-interfaces/media/aidl"
  "aidl-stubs|aidl-stubs"
)

echo "▸ vendoring AIDLs  src=$SRC  ->  $DST"
[ -d "$SRC/aosp-frameworks-av" ] || { echo "✗ no aosp-frameworks-av under $SRC (init the submodules, or pass a source root)"; exit 1; }

rm -rf "$DST"
for pair in "${PAIRS[@]}"; do
  s="$SRC/${pair%%|*}"
  d="$DST/${pair##*|}"
  [ -d "$s" ] || { echo "✗ missing source dir: $s"; exit 1; }
  mkdir -p "$(dirname "$d")"
  cp -a "$s" "$d"
  echo "  copied  ${pair%%|*}"
done

# Reference copy of the private CBLK header we PORT to Rust (not used by codegen,
# but the source of truth for the audio_track_cblk_t struct layout).
CBLK="aosp-frameworks-av/include/private/media/AudioTrackShared.h"
if [ -f "$SRC/$CBLK" ]; then
  mkdir -p "$DST/$(dirname "$CBLK")"
  cp -a "$SRC/$CBLK" "$DST/$CBLK"
  echo "  copied  $CBLK (reference for the CBLK port)"
fi

echo "✓ vendored. build.rs will now prefer ./aidl/ . Commit ./aidl/ for a self-contained crate."
