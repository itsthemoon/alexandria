#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/home" "$T/R" "$T/dest" "$T/app/Alexandria.app/Contents/MacOS" "$T/app/alexandria"
printf '#!/bin/sh\necho "alex $1 ok"\n' > "$T/app/Alexandria.app/Contents/MacOS/alex"
cp "$T/app/Alexandria.app/Contents/MacOS/alex" "$T/app/alexandria/alex"
printf '#!/bin/sh\n' > "$T/app/alexandria/alexandria-app"
chmod +x "$T/app/Alexandria.app/Contents/MacOS/alex" "$T/app/alexandria/alex" "$T/app/alexandria/alexandria-app"
tar -czf "$T/R/Alexandria-macos.tar.gz" -C "$T/app" Alexandria.app
tar -czf "$T/R/alexandria-linux-x64.tar.gz" -C "$T/app" alexandria
sha() { if command -v sha256sum >/dev/null; then sha256sum | cut -d' ' -f1; else shasum -a 256 | cut -d' ' -f1; fi; }
head -c 2500000 /dev/urandom > "$T/big.bin"
split -b 1000000 -a 2 -d "$T/big.bin" "$T/R/big.bin.part"
B="file://$T/R"
echo "big.bin 2500000 $(sha < "$T/big.bin") $B/big.bin.part00 $B/big.bin.part01 $B/big.bin.part02" > "$T/R/full-FILES.txt"
run() { HOME="$T/home" ALEXANDRIA_BASE="$B" ALEXANDRIA_DIR="$T/dest" ALEXANDRIA_EDITION="$1" bash "$ROOT/site/install.sh" < /dev/null > "$T/out" 2>&1; }
LIB="$T/dest/Alexandria"
reset() { rm -f "$LIB/big.bin" "$LIB/big.bin.part" "$LIB/big.bin.piece" "$LIB/big.bin.done"; }

mv "$T/R/big.bin.part02" "$T/hold"
if run full; then echo "a missing part must fail the install"; exit 1; fi
mv "$T/hold" "$T/R/big.bin.part02"
run full || { cat "$T/out"; exit 1; }
grep -q 'part 3 of 3' "$T/out" && ! grep -q 'part 1 of 3' "$T/out" || { echo "did not resume at part 3"; cat "$T/out"; exit 1; }
cmp "$T/big.bin" "$LIB/big.bin"

resume_ok() {
  local desc="$1"
  run full || { cat "$T/out"; echo "$desc: install failed"; exit 1; }
  cmp "$T/big.bin" "$LIB/big.bin" || { echo "$desc: joined bytes differ"; exit 1; }
}

reset
head -c 2200000 "$T/big.bin" > "$LIB/big.bin.part"
tail -c 500000 "$T/big.bin" > "$LIB/big.bin.piece"
printf '2 2000000\n' > "$LIB/big.bin.done"
resume_ok "kill during the append of part 3"
grep -q 'part 3 of 3' "$T/out" && ! grep -qE 'part (1|2) of 3' "$T/out" || { echo "mid-append did not resume at part 3"; cat "$T/out"; exit 1; }

reset
: > "$LIB/big.bin.part"
dd if="$T/big.bin" bs=1000000 skip=1 count=1 2>/dev/null > "$LIB/big.bin.piece"
printf '2 2000000\n' > "$LIB/big.bin.done"
resume_ok "stale piece left after a reset"
grep -q 'part 1 of 3' "$T/out" || { echo "stale-piece state did not restart from part 1"; cat "$T/out"; exit 1; }

reset
head -c 1000000 "$T/big.bin" > "$LIB/big.bin.part"
printf '2 2000000\n' > "$LIB/big.bin.done"
resume_ok "part shorter than recorded"
grep -q 'part 1 of 3' "$T/out" || { echo "short part did not restart from part 1"; cat "$T/out"; exit 1; }

reset
printf '2 2000000\n' > "$LIB/big.bin.done"
resume_ok "recorded progress but part missing"
grep -q 'part 1 of 3' "$T/out" || { echo "missing part did not restart from part 1"; cat "$T/out"; exit 1; }

reset
head -c 2000000 "$T/big.bin" > "$LIB/big.bin.part"
dd if="$T/big.bin" bs=200000 skip=10 count=1 2>/dev/null > "$LIB/big.bin.piece"
printf '2 2000000\n' > "$LIB/big.bin.done"
resume_ok "partial piece of part 3"
grep -q 'part 3 of 3' "$T/out" && ! grep -qE 'part (1|2) of 3' "$T/out" || { echo "partial-piece state did not resume at part 3"; cat "$T/out"; exit 1; }

rm -f "$LIB/big.bin"
printf 'precious' > "$T/outside"
ln -s "$T/outside" "$LIB/big.bin.part"
ln -s "$T/outside" "$LIB/big.bin.piece"
ln -s "$T/outside" "$LIB/big.bin.done"
run full || { cat "$T/out"; exit 1; }
cmp "$T/big.bin" "$LIB/big.bin"
[ "$(cat "$T/outside")" = "precious" ] || { echo "a planted symlink redirected the write outside the library"; exit 1; }
for ext in part piece done; do [ ! -L "$LIB/big.bin.$ext" ] || { echo "planted symlink $ext was not removed"; exit 1; }; done
if run bogus; then echo "bad edition accepted"; exit 1; fi
grep -q 'must be lite or full' "$T/out"

mkdir -p "$T/fakebin"
cat > "$T/fakebin/pgrep" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$T/fakebin/pgrep"
PATH="$T/fakebin:$PATH" HOME="$T/home" ALEXANDRIA_BASE="$B" ALEXANDRIA_DIR="$T/dest" ALEXANDRIA_EDITION=full \
  bash "$ROOT/site/install.sh" < /dev/null > "$T/out2" 2>&1 || { cat "$T/out2"; echo "install with a running app failed"; exit 1; }
grep -q 'Alexandria is running' "$T/out2" || { echo "running-app warning was not shown"; cat "$T/out2"; exit 1; }
head -c 300000 /dev/urandom > "$T/R/small.bin"
echo "small.bin 300000 $(sha < "$T/R/small.bin") $B/small.bin" >> "$T/R/full-FILES.txt"
head -c 400000 /dev/urandom > "$LIB/small.bin.part"
resume_ok "an oversized stale single-file part"
cmp "$T/R/small.bin" "$LIB/small.bin" || { echo "stale part was not replaced"; exit 1; }
echo "installer resumes multi-part downloads from any interrupt point: ok"
