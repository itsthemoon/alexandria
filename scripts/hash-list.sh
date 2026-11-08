#!/usr/bin/env bash
set -euo pipefail

LIST=${1:?usage: hash-list.sh <list> <manifest> <out> [--check] [name...]}
MANIFEST=${2:?manifest}; OUT=${3:?out}
shift 3

ALEX=${ALEX:-target/release/alex}
ALEX=$(cd "$(dirname "$ALEX")" && pwd)/$(basename "$ALEX")
mkdir -p "$OUT"

sha() { if command -v sha256sum >/dev/null; then sha256sum | cut -d' ' -f1; else shasum -a 256 | cut -d' ' -f1; fi; }

hash_one() {
  local name=$1 bytes=$2 want=$3; shift 3
  case "$name" in */*|.*|*..*) echo "unsafe name: $name" >&2; exit 1 ;; esac
  echo "hash $name ($((bytes / 1048576)) MB)"
  for u in "$@"; do curl -fsSL --retry 5 --retry-delay 5 "$u"; done \
    | tee >(sha > "$OUT/$name.sum") \
    | "$ALEX" hash-stream > "$OUT/$name.hashes" 2> "$OUT/$name.size"
  for _ in $(seq 120); do [ -s "$OUT/$name.sum" ] && break; sleep 1; done
  [ "$(tr -d ' \n' < "$OUT/$name.size")" = "$bytes" ] || { echo "size mismatch for $name" >&2; exit 1; }
  [ "$(cat "$OUT/$name.sum")" = "$want" ] || { echo "SHA-256 mismatch for $name" >&2; exit 1; }
  rm -f "$OUT/$name.sum" "$OUT/$name.size"
}

check_manifest() {
  local tag=${1:-} listed entries rel ver
  rel=$(sed -n '2p' "$MANIFEST")
  printf '%s\n' "$rel" | grep -Eq '^release v[0-9]+\.[0-9]+\.[0-9]+-full [0-9]{4}-[0-9]{2}-[0-9]{2}$' \
    || { echo "manifest release line must be a Full release like 'release v0.2.0-full 2026-11-05' (got: $rel)" >&2; exit 1; }
  if [ -n "$tag" ]; then
    ver=$(printf '%s\n' "$rel" | awk '{print $2}')
    [ "$ver" = "${tag}-full" ] || { echo "manifest release version is $ver, tag is $tag" >&2; exit 1; }
  fi
  listed=$(grep -cvE '^\s*(#|$)' "$LIST")
  entries=$(tail -n +4 "$MANIFEST" | grep -c .)
  [ "$listed" = "$entries" ] || { echo "manifest has $entries files, list has $listed" >&2; exit 1; }
  tail -n +4 "$MANIFEST" | while read -r root bytes name; do
    line=$(awk -v n="$name" '$1==n {print; exit}' "$LIST") || { echo "$name is in the manifest but not the list" >&2; exit 1; }
    read -r _ want _ <<< "$line"
    [ "$bytes" = "$want" ] || { echo "manifest size for $name is $bytes, list says $want" >&2; exit 1; }
    [ -f "$OUT/$name.hashes" ] || { echo "missing $name.hashes" >&2; exit 1; }
    [ "$(sha < "$OUT/$name.hashes")" = "$root" ] || { echo "manifest does not match the bytes of $name" >&2; exit 1; }
  done
  cp "$MANIFEST" "$OUT/MANIFEST"
  echo "manifest matches every file as downloaded: $entries files"
}

if [ "${1:-}" = "--check" ]; then
  check_manifest "${2:-}"
  exit 0
fi

if [ $# -gt 0 ]; then
  for name in "$@"; do
    line=$(awk -v n="$name" '$1==n {print; exit}' "$LIST") || { echo "$name is not in $LIST" >&2; exit 1; }
    read -r _ bytes want rest <<< "$line"
    hash_one "$name" "$bytes" "$want" $rest
  done
  exit 0
fi

grep -vE '^\s*(#|$)' "$LIST" | while read -r name bytes want urls; do
  hash_one "$name" "$bytes" "$want" $urls
done

check_manifest
