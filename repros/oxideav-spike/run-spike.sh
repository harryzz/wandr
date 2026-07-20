#!/usr/bin/env bash
# Decode each codec sample twice — once letting the registry pick (HW may win)
# and once forced to software — and report frames decoded for each.
#
# The two-column output IS the point. A row where HW decodes fewer frames than
# SW is a silent-failure bug, not a performance difference.
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CLI="${OXIDEAV_CLI:-$HERE/ws/target/release/oxideav}"
[[ -x "$CLI" ]] || { echo "no CLI at $CLI — see README (build step)"; exit 1; }
[[ -d "$HERE/samples" ]] || { echo "no samples — run ./fetch-samples.sh"; exit 1; }
mkdir -p "$HERE/out"

echo "host: $(uname -s) $(uname -m)   $(date -u +%FT%TZ)"
echo
echo "== registered video backends (impl / caps / hw / priority) =="
"$CLI" list 2>&1 | grep -E '^oxideav-' || true          # registration failures
"$CLI" list 2>/dev/null | grep -E 'h264|h265|hevc|vp8|vp9|av1' || true
echo
printf '%-6s %-22s %-22s %s\n' "codec" "auto (HW may win)" "--no-hwaccel (SW)" "verdict"
printf '%-6s %-22s %-22s %s\n' "-----" "-----------------" "-----------------" "-------"

frames() { # file, extra-flags -> "N/M"
  local out; out=$("$CLI" $2 transcode "$1" "$HERE/out/tmp.mkv" --codec-video mjpeg 2>&1 \
                   | grep -oE '[0-9]+ pkts in, [0-9]+ frames decoded' | tail -1)
  [[ -z "$out" ]] && { echo "ERR"; return; }
  echo "$(echo "$out" | grep -oE '[0-9]+ frames' | grep -oE '[0-9]+')/$(echo "$out" | grep -oE '^[0-9]+')"
}

for f in "$HERE"/samples/*; do
  b=$(basename "$f"); c="${b#bbb-}"; c="${c%%.*}"
  a=$(frames "$f" ""); s=$(frames "$f" "--no-hwaccel")
  an=${a%%/*}; sn=${s%%/*}
  verdict="ok"
  [[ "$a" == "ERR" || "$an" == "0" ]] && verdict="AUTO DECODED NOTHING"
  [[ "$s" == "ERR" || "$sn" == "0" ]] && verdict="SW DECODED NOTHING"
  [[ "$an" != "$sn" && "$an" != "0" && "$sn" != "0" ]] && verdict="MISMATCH hw!=sw"
  printf '%-6s %-22s %-22s %s\n' "$c" "$a" "$s" "$verdict"
done
rm -f "$HERE/out/tmp.mkv"
echo
echo "Report back: the table above plus the backend list."
