#!/usr/bin/env bash
set -euo pipefail

LIB=${1:?library dir}; LIST=${2:?edition list}; shift 2
OUT="$LIB/streets.txt"

if [ "$#" -eq 4 ]; then
  python3 scripts/streets.py --bbox "$1" "$2" "$3" "$4" --out "$OUT"
else
  python3 scripts/streets.py --out "$OUT"
fi

test -s "$OUT" || { echo "street sidecar is empty"; exit 1; }

if grep -qE '^streets\.txt ' "$LIST"; then
  sed -i.bak "/^streets\.txt /d" "$LIST"
  rm -f "$LIST.bak"
fi
printf 'streets.txt %s %s https://github.com/itsthemoon/alexandria/releases/download/library-data/streets.txt\n' \
  "$(wc -c < "$OUT" | tr -d ' ')" \
  "$(if command -v sha256sum >/dev/null; then sha256sum "$OUT" | cut -d' ' -f1; else shasum -a 256 "$OUT" | cut -d' ' -f1; fi)" >> "$LIST"

echo "streets.txt: $(wc -c < "$OUT" | tr -d ' ') bytes, listed in $LIST"
