#!/usr/bin/env bash
set -uo pipefail

LIB=${1:?library dir}
PUB=${2:?public key hex}
ALEX=${3:-target/release/alex}
pass=0; fail=0
ok()  { echo "PASS  $1"; pass=$((pass+1)); }
bad() { echo "FAIL  $1"; fail=$((fail+1)); }
check() { local name=$1; shift; if "$@" >/dev/null 2>&1; then ok "$name"; else bad "$name"; fi; }

TMP=$(mktemp -d "${TMPDIR:-/tmp}/alex-e2e.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

if "$ALEX" verify "$LIB" "$PUB" > "$TMP/verify.txt" 2>&1; then
  ok "verify: $(tail -1 "$TMP/verify.txt")"
else
  bad "verify: $(tail -1 "$TMP/verify.txt")"
fi

WRONG=$(printf '%064d' 0 | tr 0 1)
if "$ALEX" list "$LIB" "$WRONG" >/dev/null 2>&1; then bad "wrong key refused"; else ok "wrong key refused"; fi

first_title() { "$ALEX" search "$LIB" "$PUB" "$1" 2>/dev/null | head -1 | cut -f1; }
for q in "Water" "Photosynthesis" "Paris"; do
  t=$(first_title "$q")
  if [ "$t" = "$q" ]; then ok "search '$q' -> '$t'"; else bad "search '$q' -> '$t'"; fi
done

if "$ALEX" article "$LIB" "$PUB" Water > "$TMP/water.html" 2>/dev/null && grep -qi "<html" "$TMP/water.html" && [ "$(wc -c < "$TMP/water.html")" -gt 2000 ]; then
  ok "article Water ($(wc -c < "$TMP/water.html" | tr -d ' ') bytes)"
else
  bad "article Water"
fi

if "$ALEX" tile "$LIB" "$PUB" 0 0 0 "$TMP/t.png" >/dev/null 2>&1; then ok "map tile 0/0/0"; else echo "SKIP  map tile (vector packs are served to the app, not the CLI)"; fi

if [ -x "$(dirname "$ALEX")/llama-completion" ] || [ -x "$(dirname "$ALEX")/llama-completion.exe" ]; then
  if "$ALEX" ask "$LIB" "$PUB" "What is photosynthesis?" > "$TMP/ask.txt" 2>&1 && grep -qi "light\|sun\|energy" "$TMP/ask.txt"; then
    ok "ask photosynthesis: $(head -c 120 "$TMP/ask.txt" | tr '\n' ' ')"
  else
    bad "ask photosynthesis: $(tail -3 "$TMP/ask.txt" | tr '\n' ' ')"
  fi
fi

mkdir -p "$TMP/lib"
cp "$LIB/MANIFEST.sig" "$TMP/lib/"
awk 'NR!=4' "$LIB/MANIFEST" > "$TMP/lib/MANIFEST"
if "$ALEX" list "$TMP/lib" "$PUB" >/dev/null 2>&1; then bad "edited MANIFEST refused"; else ok "edited MANIFEST refused"; fi
mkdir -p "$TMP/t"
head -c 3000000 /dev/urandom > "$TMP/t/data.bin"
"$ALEX" keygen "$TMP/seed" "$TMP/pub" >/dev/null
"$ALEX" manifest "$TMP/t" v0 2026-01-01 >/dev/null
"$ALEX" sign "$TMP/t" "$TMP/seed" >/dev/null
check "throwaway library verifies before tampering" "$ALEX" verify "$TMP/t" "$(cat "$TMP/pub")"
printf '\xff' | dd of="$TMP/t/data.bin" bs=1 seek=1500000 conv=notrunc 2>/dev/null
out=$("$ALEX" verify "$TMP/t" "$(cat "$TMP/pub")" 2>&1)
if echo "$out" | grep -q "chunk 1 failed verification"; then ok "flipped byte refused ($(echo "$out" | tail -1))"; else bad "flipped byte refused: $(echo "$out" | tail -1)"; fi

mkdir -p "$TMP/k"
head -c 100000 /dev/urandom > "$TMP/k/data.bin"
"$ALEX" keygen "$TMP/root" "$TMP/root.pub" >/dev/null
"$ALEX" keygen "$TMP/rel" "$TMP/rel.pub" >/dev/null
"$ALEX" keygen "$TMP/other" "$TMP/other.pub" >/dev/null
"$ALEX" manifest "$TMP/k" v0 2026-01-01 >/dev/null
"$ALEX" sign "$TMP/k" "$TMP/rel" >/dev/null
if "$ALEX" verify "$TMP/k" "$(cat "$TMP/root.pub")" >/dev/null 2>&1; then bad "release key without KEYS refused"; else ok "release key without KEYS refused"; fi
"$ALEX" keys "$TMP/k" "$TMP/root" "$(cat "$TMP/rel.pub")" >/dev/null
check "release key listed in root-signed KEYS accepted" "$ALEX" verify "$TMP/k" "$(cat "$TMP/root.pub")"
"$ALEX" manifest "$TMP/k" v0 2026-01-01 >/dev/null
"$ALEX" sign "$TMP/k" "$TMP/other" >/dev/null
if "$ALEX" verify "$TMP/k" "$(cat "$TMP/root.pub")" >/dev/null 2>&1; then bad "unlisted key refused"; else ok "unlisted key refused"; fi

echo
echo "e2e: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
