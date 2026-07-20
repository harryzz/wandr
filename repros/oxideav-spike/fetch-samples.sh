#!/usr/bin/env bash
# Fetch small royalty-free clips, one per codec, for the oxideav spike.
#
# Big Buck Bunny (Blender Foundation, CC-BY 3.0) — same 10 s of content encoded
# four ways, so a decode difference is the CODEC, not the material. ~1 MB each.
#
# Codec coverage is the point:
#   h264  — the one everything has in hardware
#   h265  — HW on the Pixel 2 XL (measured) and most GPUs since ~2015
#   vp9   — what we already decode via libvpx, so it doubles as a cross-check
#   av1   — HW only on Intel Xe/Arc, NVIDIA 30xx+, AMD RDNA2+; software elsewhere
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)/samples"
mkdir -p "$DIR"
BASE="https://test-videos.co.uk/vids/bigbuckbunny"
NAME="Big_Buck_Bunny_720_10s_1MB"

fetch() { # url, out
  if [[ -s "$2" ]]; then echo "have  $(basename "$2")"; return; fi
  echo "get   $(basename "$2")"
  curl -fsSL --retry 3 -o "$2" "$1" || { echo "  FAILED: $1" >&2; rm -f "$2"; }
}

fetch "$BASE/mp4/h264/720/$NAME.mp4"   "$DIR/bbb-h264.mp4"
fetch "$BASE/mp4/h265/720/$NAME.mp4"   "$DIR/bbb-h265.mp4"
fetch "$BASE/webm/vp9/720/$NAME.webm"  "$DIR/bbb-vp9.webm"
fetch "$BASE/webm/av1/720/$NAME.webm"  "$DIR/bbb-av1.webm"

echo
ls -lh "$DIR" | awk 'NR>1 {print "  " $5 "\t" $9}'
echo
echo "Samples in $DIR"
