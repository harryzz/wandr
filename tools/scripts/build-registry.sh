#!/bin/sh
# build-registry.sh — pack curated wandr apps into .wandrpkg archives and emit
# the registry index.json that the `wandr` CLI consumes.
#
#   tools/scripts/build-registry.sh            pack + write docs/registry/index.json
#   tools/scripts/build-registry.sh --publish  also upload the .wandrpkg archives
#                                              to the `apps` GitHub release (gh auth)
#
# The .wandrpkg archives are Release assets (github.com/harryzz/wandr, tag `apps`);
# index.json is committed at docs/registry/index.json and served by GitHub Pages
# (Settings → Pages → Branch main /docs) at
#   https://harryzz.github.io/wandr/registry/index.json
#
# Apps must be BUILT first (their components/*.wasm present on disk) — the bundles
# are gitignored per-toolchain build outputs, so this runs on your machine, not CI.
set -eu

REPO="harryzz/wandr"
TAG="apps"                                   # rolling release holding the assets
BASE_URL="https://github.com/$REPO/releases/download/$TAG"

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
APPS_DIR="$ROOT/apps/user"
LIST="${WANDR_APPS_LIST:-$ROOT/tools/registry/apps.list}"
OUT="$ROOT/dist/registry"
INDEX="$ROOT/docs/registry/index.json"
PAGES_URL="https://harryzz.github.io/wandr/registry/index.json"

publish=0
[ "${1:-}" = "--publish" ] && publish=1

info() { printf '\033[1;32m▸\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

command -v jq  >/dev/null 2>&1 || err "need jq."
command -v zip >/dev/null 2>&1 || err "need zip."

filesize() { stat -c%s "$1" 2>/dev/null || stat -f%z "$1"; }
sha256()   { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
             else shasum -a 256 "$1" | awk '{print $1}'; fi; }
# read a top-level string key from a package.toml
toml_val() {
  awk -F= -v k="$2" '
    $0 ~ "^[[:space:]]*" k "[[:space:]]*=" {
      v = substr($0, index($0, "=") + 1); sub(/#.*/, "", v)
      gsub(/^[[:space:]]*"?|"?[[:space:]]*$/, "", v); print v; exit
    }' "$1"
}

[ -f "$LIST" ] || err "no app list at $LIST"
rm -rf "$OUT"; mkdir -p "$OUT" "$(dirname "$INDEX")"
entries="$OUT/.entries.jsonl"; : > "$entries"

count=0
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in ''|\#*) continue ;; esac
  dir=$(printf '%s' "$line"  | sed 's/|.*//;    s/[[:space:]]*$//; s/^[[:space:]]*//')
  desc=$(printf '%s' "$line" | sed 's/^[^|]*//; s/^|[[:space:]]*//; s/[[:space:]]*$//')
  [ -n "$dir" ] || continue

  appdir="$APPS_DIR/$dir"
  pk="$appdir/package.toml"
  [ -f "$pk" ] || { warn "skip $dir — no package.toml"; continue; }
  ls "$appdir"/components/*.wasm >/dev/null 2>&1 || { warn "skip $dir — not built (no components/*.wasm)"; continue; }

  id=$(toml_val "$pk" app_id)
  ver=$(toml_val "$pk" version)
  name=$(toml_val "$pk" label); [ -n "$name" ] || name="$id"
  [ -n "$id" ] && [ -n "$ver" ] || { warn "skip $dir — missing app_id/version"; continue; }

  asset="$id-$ver.wandrpkg"
  out="$OUT/$asset"
  ( cd "$appdir" && rm -f "$out" && zip -r -q "$out" package.toml components $([ -d assets ] && echo assets) )
  size=$(filesize "$out"); sha=$(sha256 "$out")
  info "packed $id v$ver  ($(du -h "$out" | cut -f1))"

  jq -n --arg id "$id" --arg name "$name" --arg ver "$ver" --arg desc "$desc" \
        --argjson size "$size" --arg sha "$sha" --arg url "$BASE_URL/$asset" \
        '{id:$id,name:$name,version:$ver,description:$desc,size:$size,sha256:$sha,url:$url}' \
        >> "$entries"
  count=$((count+1))
done < "$LIST"

[ "$count" -gt 0 ] || err "no apps packed — build some first (see apps.list)."
jq -s '{apps: .}' "$entries" > "$INDEX"
rm -f "$entries"
info "wrote $INDEX  ($count apps)"

if [ "$publish" = 1 ]; then
  command -v gh >/dev/null 2>&1 || err "--publish needs the gh CLI (and auth)."
  gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 || \
    gh release create "$TAG" --repo "$REPO" --title "wandr apps" \
      --notes "App registry (.wandrpkg archives) consumed by the wandr CLI." --latest=false
  info "uploading $count assets to $REPO release '$TAG' …"
  gh release upload "$TAG" "$OUT"/*.wandrpkg --repo "$REPO" --clobber
  info "assets published to $REPO release '$TAG'."
fi

printf '\nnext:\n'
printf '  git add docs/registry/index.json && git commit -m "registry: refresh" && git push\n'
[ "$publish" = 1 ] || printf '  (assets not uploaded — re-run with --publish to push .wandrpkg to the release)\n'
printf '  Pages one-time: Settings → Pages → Branch main /docs  →  %s\n' "$PAGES_URL"
