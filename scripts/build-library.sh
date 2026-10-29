#!/usr/bin/env bash
set -euo pipefail

LIST=${1:?usage: build-library.sh <list> <dir> <release> <date>}
DIR=${2:?usage: build-library.sh <list> <dir> <release> <date>}
RELEASE=${3:?release}
DATE=${4:?date}
ALEX=${ALEX:-target/release/alex}
ALEX=$(cd "$(dirname "$ALEX")" && pwd)/$(basename "$ALEX")
LIST=$(cd "$(dirname "$LIST")" && pwd)/$(basename "$LIST")
mkdir -p "$DIR"
cd "$DIR"

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
size() { wc -c < "$1" | tr -d ' '; }

grep -vE '^\s*(#|$)' "$LIST" | while read -r name bytes want url; do
  case "$name" in */*|.*|*..*) echo "unsafe name: $name" >&2; exit 1 ;; esac
  if [ -f "$name" ] && [ "$(size "$name")" = "$bytes" ] && [ "$(sha "$name")" = "$want" ]; then
    echo "ok   $name"
    continue
  fi
  echo "get  $name ($((bytes / 1048576)) MB)"
  curl -fL --retry 5 --retry-delay 5 -C - -o "$name.part" "$url"
  got=$(size "$name.part")
  [ "$got" = "$bytes" ] || { echo "size mismatch for $name: $got != $bytes" >&2; exit 1; }
  [ "$(sha "$name.part")" = "$want" ] || { echo "SHA-256 mismatch for $name" >&2; exit 1; }
  mv "$name.part" "$name"
done

for f in *; do
  case "$f" in MANIFEST|MANIFEST.sig|KEYS|KEYS.sig|*.hashes|*.search.db|*.part) continue ;; esac
  grep -qE "^$f " "$LIST" || { echo "not in the list, refusing to sign around it: $f" >&2; exit 1; }
done

rm -f ./*.search.db ./*.search.db.hashes

"$ALEX" manifest . "$RELEASE" "$DATE"
echo "library ready (unsigned): $DIR"
