#!/usr/bin/env bash
# patch-rtc.sh — apply wart's local patches to the external/rtc submodule
# (webrtc-rs/rtc). Idempotent: skips a patch that's already applied.
#
# Why: external/rtc tracks pristine upstream webrtc-rs/rtc (pinned). We carry a
# small delta — rtc-ice's mDNS made optional/default-on — so the ICE crate builds
# for wasm32-wasip2 (`--no-default-features`); upstream pulls rtc-mdns →
# socket2/tokio, which don't build for wasip2. See repros/webrtc-rs-wasip2.
#
# Run after a fresh clone / `git submodule update`, before building wart-call or
# the call-engine repros.
#
#   tools/scripts/patch-rtc.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RTC_DIR="$REPO_ROOT/external/rtc"
PATCH="$REPO_ROOT/repros/webrtc-rs-wasip2/rtc-ice-mdns-optional.patch"

if [[ ! -d "$RTC_DIR/rtc-ice" ]]; then
    echo "✗ external/rtc not checked out — run: git submodule update --init external/rtc" >&2
    exit 1
fi
if [[ ! -f "$PATCH" ]]; then
    echo "✗ patch not found: $PATCH" >&2
    exit 1
fi

cd "$RTC_DIR"
if git apply --reverse --check "$PATCH" >/dev/null 2>&1; then
    echo "▸ rtc-ice mDNS-optional patch already applied — nothing to do"
    exit 0
fi
echo "▸ applying rtc-ice mDNS-optional patch …"
git apply "$PATCH"
echo "✓ applied. rtc-ice now builds with --no-default-features for wasm32-wasip2."
