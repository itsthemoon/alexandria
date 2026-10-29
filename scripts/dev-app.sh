#!/bin/bash
set -e
cd "$(dirname "$0")/.."
export ALEXANDRIA_PUBKEY="$(tr -d '\n' < release-pubkey.txt)"
[ -n "$ALEXANDRIA_PUBKEY" ] || { echo "release-pubkey.txt is empty" >&2; exit 1; }

if [ "${1:-}" = "--build" ] || [ ! -x target/release/alexandria-app ]; then
  cargo build --release -p alexandria-app
fi
exec env ALEX_LIBRARY="${ALEX_LIBRARY:-$HOME/Alexandria}" target/release/alexandria-app
