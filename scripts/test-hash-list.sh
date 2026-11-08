#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

sha() { shasum -a 256 | cut -d' ' -f1; }

mkdir -p "$T/out"
printf 'AAAA' > "$T/out/good.bin.hashes"
root=$(sha < "$T/out/good.bin.hashes")
printf 'good.bin 4 %s http://example/x\n' "$root" > "$T/list.txt"

mkmanifest() {
  printf 'alexandria-manifest 1\n' > "$T/MANIFEST"
  printf 'release %s %s\n' "$1" "$2" >> "$T/MANIFEST"
  printf 'chunk 1048576\n' >> "$T/MANIFEST"
  printf '%s 4 good.bin\n' "$root" >> "$T/MANIFEST"
}

check() { ALEX=/bin/true bash "$ROOT/scripts/hash-list.sh" "$T/list.txt" "$T/MANIFEST" "$T/out" --check "$1"; }

mkmanifest 'v0.2.0-full' '2026-11-05'
check 'v0.2.0' || { echo "a good Full manifest should pass"; exit 1; }

mkmanifest 'v0.2.0' '2026-11-05'
if check 'v0.2.0'; then echo "a manifest missing -full should fail"; exit 1; fi

mkmanifest 'v0.2.0-full' '2026-11-05'
if check 'v0.3.0'; then echo "a version that differs from the tag should fail"; exit 1; fi

mkmanifest 'v0.2.0-full' 'not-a-date'
if check 'v0.2.0'; then echo "a bad date should fail"; exit 1; fi

echo "hash-list.sh --check enforces the Full release line: ok"
