#!/usr/bin/env bash
# patch-rtc.sh — apply wandr's local patches to the external/rtc submodule
# (webrtc-rs/rtc). Idempotent: skips a patch that's already applied.
#
# Why: external/rtc tracks pristine upstream webrtc-rs/rtc (pinned). We carry a
# delta in one patch file (`wandr-rtc.patch`) covering three things:
#   1. rtc-ice's mDNS made optional/default-on so the ICE crate builds for
#      wasm32-wasip2 (`--no-default-features`); upstream pulls rtc-mdns →
#      socket2/tokio, which don't build for wasip2. See repros/webrtc-rs-wasip2.
#   2. wandr call-engine support (task 16): Agent::self_select_best_pair() — the
#      answerer self-selects a Succeeded pair when the peer (ringrtc, which uses
#      libwebrtc presume_writable_when_fully_relayed) never sends USE-CANDIDATE —
#      plus connect-path diagnostics (IceDebug counters, wire log, debug_pairs).
#   3. rtc-srtp `external-aead` feature (task 93): AeadProvider/AeadCtx trait
#      injection so the SRTP AES-GCM block can run on the host's hardware AES
#      (wandr:crypto) — off by default, crate unchanged/portable without it.
# (Regenerate after editing the submodule — `add -N` any new files first so the
#  diff includes them:
#    git -C external/rtc add -N . && git -C external/rtc diff > repros/webrtc-rs-wasip2/wandr-rtc.patch && git -C external/rtc reset -q)
#
# Run after a fresh clone / `git submodule update`, before building wandr-call or
# the call-engine repros.
#
#   tools/scripts/patch-rtc.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RTC_DIR="$REPO_ROOT/external/rtc"
PATCH="$REPO_ROOT/repros/webrtc-rs-wasip2/wandr-rtc.patch"

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
    echo "▸ wandr rtc patch already applied — nothing to do"
    exit 0
fi
echo "▸ applying wandr rtc patch …"
git apply "$PATCH"
echo "✓ applied (mDNS-optional + ICE self-select + srtp external-aead)."
