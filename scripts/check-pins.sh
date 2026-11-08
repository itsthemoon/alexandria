#!/usr/bin/env bash
set -euo pipefail

DIR=${1:?usage: check-pins.sh <dir-with-assets>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

MAC_SHA=$(sed -n 's/^MAC_SHA=//p' "$ROOT/site/install.sh")
LINUX_SHA=$(sed -n 's/^LINUX_SHA=//p' "$ROOT/site/install.sh")
WIN_X64=$(awk -F"'" '/^  x64 = /{print $2}' "$ROOT/site/install.ps1")
WIN_ARM64=$(awk -F"'" '/^  arm64 = /{print $2}' "$ROOT/site/install.ps1")

check() {
  local file=$1 pin=$2 got
  [ -n "$pin" ] || { echo "could not read the pin for $file" >&2; exit 1; }
  [ -f "$DIR/$file" ] || { echo "missing asset: $file" >&2; exit 1; }
  got=$(sha "$DIR/$file")
  printf '%s  %s\n' "$got" "$file"
  [ "$got" = "$pin" ] || { echo "SHA-256 mismatch for $file: built $got, pinned $pin" >&2; exit 1; }
}

check Alexandria-macos.tar.gz "$MAC_SHA"
check alexandria-linux-x64.tar.gz "$LINUX_SHA"
check alexandria-windows-x64.zip "$WIN_X64"
check alexandria-windows-arm64.zip "$WIN_ARM64"
echo "every pinned SHA-256 matches the built assets"
