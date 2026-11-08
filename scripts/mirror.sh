#!/usr/bin/env bash
set -euo pipefail

NAME=${1:?usage: mirror.sh <name> <url> <list> <tag> <work>}
URL=${2:?url}; LIST=${3:?list}; TAG=${4:?tag}; WORK=${5:?work}
ALEX=${ALEX:-target/release/alex}
ALEX=$(cd "$(dirname "$ALEX")" && pwd)/$(basename "$ALEX")
PART=${MIRROR_PART:-$((1900 * 1024 * 1024))}
STAGING="library-staging-$NAME-$(date +%s)-$$"
case "$NAME" in */*|.*|*..*) echo "unsafe name: $NAME" >&2; exit 1 ;; esac
case "$URL" in
  https://download.kiwix.org/*|https://mirrors.dotsrc.org/*|https://ftp.fau.de/*|https://build.protomaps.com/*|https://huggingface.co/*) ;;
  file://*|http://127.0.0.1:*) [ "${MIRROR_ALLOW_FILE:-}" = 1 ] || { echo "refusing test-only URL (MIRROR_ALLOW_FILE=1): $URL" >&2; exit 1; } ;;
  http://*) echo "refusing non-https URL: $URL" >&2; exit 1 ;;
  *) echo "refusing URL from an unallowed host (allowed: download.kiwix.org, mirrors.dotsrc.org, ftp.fau.de, build.protomaps.com, huggingface.co): $URL" >&2; exit 1 ;;
esac
read -r _ BYTES WANT _ < <(awk -v n="$NAME" '$1==n {print; exit}' "$LIST") || { echo "$NAME is not in $LIST" >&2; exit 1; }
mkdir -p "$WORK"
cd "$WORK"

size() { wc -c < "$1" | tr -d ' '; }
sha() { if command -v sha256sum >/dev/null; then sha256sum < "$1" | cut -d' ' -f1; else shasum -a 256 < "$1" | cut -d' ' -f1; fi; }
fail() { echo "$*" >&2; exit 1; }

gh release create "$STAGING" --title "$STAGING" >/dev/null

cleanup() { gh release delete "$STAGING" --yes --cleanup-tag >/dev/null 2>&1 || true; }
trap cleanup INT TERM EXIT

exec 3> >(tee >(sha256sum | cut -d' ' -f1 > sum.txt) | "$ALEX" hash-stream > "$NAME.hashes" 2> size.txt)
n=$(( (BYTES + PART - 1) / PART ))
: > parts.sha
for ((i = 0; i < n; i++)); do
  part=$(printf '%s.part%02d' "$NAME" "$i")
  start=$((i * PART)); end=$((start + PART)); [ "$end" -le "$BYTES" ] || end=$BYTES
  : > "$part"
  tries=0
  while [ "$(size "$part")" -lt $((end - start)) ]; do
    tries=$((tries + 1)); [ "$tries" -le 20 ] || fail "gave up on $part"
    curl -fsL --http1.1 --max-time 900 -r "$((start + $(size "$part")))-$((end - 1))" "$URL" >> "$part" || sleep 5
  done
  [ "$(size "$part")" = $((end - start)) ] || fail "$part has the wrong size"
  printf '%s %s\n' "$part" "$(sha "$part")" >> parts.sha
  cat "$part" >&3
  gh release upload "$STAGING" "$part" --clobber
  rm -f "$part"
done
exec 3>&-
for _ in $(seq 120); do [ -s sum.txt ] && [ -s size.txt ] && break; sleep 1; done

[ "$(tr -d ' \n' < size.txt)" = "$BYTES" ] || fail "size mismatch: $(cat size.txt) != $BYTES"
[ "$(cat sum.txt)" = "$WANT" ] || fail "SHA-256 mismatch for $NAME"

while read -r part want; do
  gh release download "$STAGING" "$part" --clobber >/dev/null 2>&1
  [ "$(sha "$part")" = "$want" ] || fail "staged $part no longer matches its hash"
  gh release upload "$TAG" "$part" --clobber
  rm -f "$part"
done < parts.sha

trap - INT TERM EXIT
gh release delete "$STAGING" --yes --cleanup-tag >/dev/null 2>&1 || true

echo "mirrored $NAME: $n parts, $BYTES bytes, sha256 $WANT"
