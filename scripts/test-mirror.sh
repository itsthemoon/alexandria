#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"; [ -n "${SRV:-}" ] && kill "$SRV" 2>/dev/null || true' EXIT

BIN="$T/bin"; mkdir -p "$BIN" "$T/srv" "$T/work" "$T/store"
LOG="$T/log.txt"
: > "$LOG"

cat > "$BIN/alex" <<'EOF'
#!/bin/sh
if [ "$1" = "hash-stream" ]; then
  wc -c >&2
else
  exit 0
fi
EOF
chmod +x "$BIN/alex"

cat > "$BIN/sha256sum" <<'EOF'
#!/bin/sh
exec shasum -a 256 | cut -d' ' -f1
EOF
chmod +x "$BIN/sha256sum"

cat > "$BIN/gh" <<'EOF'
#!/bin/bash
STORE="${GH_STORE:?}"
LOG="${GH_LOG_FILE:?}"
case "$1 $2" in
  "release view")
    [ -d "$STORE/$3" ] || exit 1
    ;;
  "release create")
    mkdir -p "$STORE/$3"
    echo "create $3" >> "$LOG"
    ;;
  "release upload")
    tag=$3; shift 3
    mkdir -p "$STORE/$tag"
    for f in "$@"; do
      case "$f" in -*) continue ;; esac
      cp "$f" "$STORE/$tag/$(basename "$f")"
      echo "upload $tag $(basename "$f")" >> "$LOG"
    done
    ;;
  "release download")
    cp "$STORE/$3/$4" "./$4"
    echo "download $3 $4" >> "$LOG"
    ;;
  "release delete")
    rm -rf "$STORE/$3"
    echo "delete-release $3" >> "$LOG"
    ;;
esac
EOF
chmod +x "$BIN/gh"

cat > "$T/srv.py" <<'EOF'
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        try:
            with open(self.path.lstrip('/'), 'rb') as f:
                data = f.read()
        except OSError:
            self.send_response(404); self.end_headers(); return
        rng = self.headers.get('Range')
        if rng:
            a, b = rng.split('=')[1].split('-')
            a = int(a) if a else 0
            b = int(b) if b else len(data) - 1
            body = data[a:b + 1]
            self.send_response(206)
            self.send_header('Content-Range', 'bytes %d-%d/%d' % (a, b, len(data)))
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(200)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
EOF

printf 'AAAABBBBCCCC' > "$T/srv/good.bin"
printf 'XXXXXXXXXXXX' > "$T/srv/bad.bin"
WANT=$(shasum -a 256 "$T/srv/good.bin" | cut -d' ' -f1)
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
(cd "$T/srv" && python3 "$T/srv.py" "$PORT") >/dev/null 2>&1 &
SRV=$!
for _ in $(seq 50); do curl -fsS "http://127.0.0.1:$PORT/good.bin" >/dev/null 2>&1 && break; sleep 0.2; done

LIST="$T/list.txt"
printf 'file.bin 12 %s http://127.0.0.1:%s/good.bin\n' "$WANT" "$PORT" > "$LIST"

run() {
  MIRROR_PART=4 MIRROR_ALLOW_FILE=1 ALEX="$BIN/alex" PATH="$BIN:$PATH" \
  GH_STORE="$T/store" GH_LOG_FILE="$LOG" \
  bash "$ROOT/scripts/mirror.sh" file.bin "http://127.0.0.1:$PORT/$1" "$LIST" library-data "$T/work"
}

live_matches_good() {
  cat "$T/store/library-data/file.bin.part00" "$T/store/library-data/file.bin.part01" "$T/store/library-data/file.bin.part02" | cmp - "$T/srv/good.bin"
}

staging_leftovers() { ls "$T/store" | grep -v '^library-data$' | wc -l | tr -d ' '; }

: > "$LOG"
run good.bin || { echo "happy-path mirror failed"; exit 1; }
for p in file.bin.part00 file.bin.part01 file.bin.part02; do
  [ -f "$T/store/library-data/$p" ] || { echo "live is missing $p"; exit 1; }
done
grep -q 'upload library-data file.bin.part00' "$LOG" || { echo "parts were not promoted to live"; exit 1; }
grep -q '^delete-release ' "$LOG" || { echo "staging release was not deleted after success"; exit 1; }
[ "$(staging_leftovers)" = "0" ] || { echo "staging leftovers remain after success"; exit 1; }
live_matches_good || { echo "live parts do not reassemble to the good file"; exit 1; }

: > "$LOG"
if run bad.bin; then echo "a mismatched mirror must fail"; exit 1; fi
if grep -q 'upload library-data' "$LOG"; then echo "a mismatched run touched live assets"; exit 1; fi
for p in file.bin.part00 file.bin.part01 file.bin.part02; do
  [ -f "$T/store/library-data/$p" ] || { echo "$p was removed from live by a failed run"; exit 1; }
done
[ "$(staging_leftovers)" = "0" ] || { echo "staging leftovers remain after a failed run"; exit 1; }
grep -q '^delete-release ' "$LOG" || { echo "staging was not cleaned after a failed run"; exit 1; }
live_matches_good || { echo "live parts changed after a failed run"; exit 1; }

: > "$LOG"
run good.bin || { echo "re-run mirror failed"; exit 1; }
grep -q 'upload library-data file.bin.part00' "$LOG" || { echo "re-run did not promote parts"; exit 1; }
if grep -q 'delete library-data' "$LOG"; then echo "a successful re-run deleted live parts"; exit 1; fi
[ "$(staging_leftovers)" = "0" ] || { echo "staging leftovers remain after a successful re-run"; exit 1; }
live_matches_good || { echo "live parts differ after a successful re-run"; exit 1; }
echo "mirror stages, verifies, then promotes; live is never touched by a failed run: ok"
refuse() {
  local url="$1"
  if out=$(MIRROR_ALLOW_FILE= ALEX="$BIN/alex" PATH="$BIN:$PATH" GH_STORE="$T/store" GH_LOG_FILE="$LOG" \
      bash "$ROOT/scripts/mirror.sh" file.bin "$url" "$LIST" library-data "$T/work" 2>&1); then
    echo "an unallowed URL was accepted: $url"; exit 1
  fi
  printf '%s\n' "$out" | grep -qi 'refus' || { echo "no refusal message for: $url"; exit 1; }
}
refuse "http://download.kiwix.org/zim/example.zim"
refuse "https://evil.example/example.zim"
refuse "file:///etc/passwd"
if MIRROR_PART=4 ALEX="$BIN/alex" PATH="$BIN:$PATH" GH_STORE="$T/store" GH_LOG_FILE="$LOG" \
   bash "$ROOT/scripts/mirror.sh" file.bin "http://127.0.0.1:$PORT/good.bin" "$LIST" library-data "$T/work" >/dev/null 2>&1; then
  echo "a test-server URL without MIRROR_ALLOW_FILE was accepted"; exit 1
fi
echo "mirror refuses non-https URLs and hosts outside the allowlist: ok"
