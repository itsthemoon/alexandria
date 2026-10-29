#!/usr/bin/env bash
set -euo pipefail

LIB=${1:?library dir}; LIST=${2:?list}; ED=${3:?edition}; BASE=${4:?base url}; OUT=${5:?out dir}
PART=$((1900 * 1024 * 1024))
mkdir -p "$OUT"
PLAN="$OUT/$ED-FILES.txt"
: > "$PLAN"

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
size() { wc -c < "$1" | tr -d ' '; }

add() {
  local name=$1 file=$2; shift 2
  echo "$name $(size "$file") $(sha "$file") $*" >> "$PLAN"
}

host() {
  local name=$1 file="$LIB/$1"
  if [ "$(size "$file")" -le "$PART" ]; then
    cp "$file" "$OUT/$name"
    add "$name" "$file" "$BASE/$name"
  else
    split -b "$PART" -a 2 -d "$file" "$OUT/$name.part"
    local urls=()
    for p in "$OUT/$name".part*; do urls+=("$BASE/$(basename "$p")"); done
    add "$name" "$file" "${urls[@]}"
  fi
}

for f in MANIFEST MANIFEST.sig KEYS KEYS.sig; do
  [ -f "$LIB/$f" ] || continue
  cp "$LIB/$f" "$OUT/$ED-$f"
  add "$f" "$LIB/$f" "$BASE/$ED-$f"
done

tail -n +4 "$LIB/MANIFEST" | while read -r _ _ name; do
  [ -n "$name" ] || continue
  line=$(grep -E "^$name " "$LIST" || true)
  if [ -n "$line" ]; then
    read -r _ bytes want url <<< "$line"
    echo "$name $bytes $want $url" >> "$PLAN"
  else
    host "$name"
  fi
  host "$name.hashes"
done

echo "wrote $PLAN ($(wc -l < "$PLAN" | tr -d ' ') files) and $(ls "$OUT" | wc -l | tr -d ' ') assets in $OUT"
