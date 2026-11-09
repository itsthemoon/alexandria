#!/usr/bin/env bash
set -euo pipefail

VERSION=v0.2.0
REPO=itsthemoon/alexandria
EDITION=${ALEXANDRIA_EDITION:-lite}
ROOT_KEY=afcbc7a3ff47fac00ed3bcca1b2459d2a6a03053f9a0ed7bb572997af69d2f4a
BASE=${ALEXANDRIA_BASE:-https://github.com/$REPO/releases/download/$VERSION}
MAC_SHA=6120d7e9963081b41154ae7c2397856e04d932a432f7565b6ab3ddef87d70f21
LINUX_SHA=a187672cadae7e96b8f7181974930329f7f1d77a1309185e94fff32e0bffa056
HOSTS='https://github.com/itsthemoon/alexandria/releases/download/ https://download.kiwix.org/zim/ https://huggingface.co/LiquidAI/'

say()  { printf '\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }
ask() {
  local a=""
  if [ -z "${ALEXANDRIA_DIR:-}" ] && (exec </dev/tty) 2>/dev/null; then
    printf '%s [%s]: ' "$1" "$2" > /dev/tty
    read -r a < /dev/tty || true
  fi
  printf '%s' "${a:-$2}"
}
sha() { if command -v sha256sum >/dev/null; then sha256sum < "$1" | cut -d' ' -f1; else shasum -a 256 < "$1" | cut -d' ' -f1; fi; }
size() { wc -c < "$1" | tr -d ' '; }
free_bytes() { df -Pk "$1" | awk 'NR==2 {printf "%.0f\n", $4 * 1024}'; }
PROTO="=https"; [ -n "${ALEXANDRIA_BASE:-}" ] && PROTO="=https,file"
get() { curl -fL -g --proto "$PROTO" --retry 5 --retry-delay 3 --progress-bar "$@"; }
pinned() { [ -n "${ALEXANDRIA_BASE:-}" ] || [ "$(sha "$1")" = "$2" ] || fail "The app download does not match the checksum in this installer. Nothing was installed."; }

main() {

command -v curl >/dev/null || fail "curl is required."
case "$EDITION" in lite|full) ;; *) fail "ALEXANDRIA_EDITION must be lite or full." ;; esac
if pgrep -x Alexandria >/dev/null 2>&1 || pgrep -x alexandria-app >/dev/null 2>&1; then
  say "Alexandria is running. Reopen it after the install to get the new version."
fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

get -s -o "$TMP/files.txt" "$BASE/$EDITION-FILES.txt"
awk -v base="${ALEXANDRIA_BASE:-}" -v hosts="$HOSTS" '
  BEGIN { nh = split(hosts, h, " ") }
  { ok = ($1 ~ /^[A-Za-z0-9][A-Za-z0-9._-]*$/) && ($1 !~ /\.\./) && ($2 ~ /^[0-9]+$/) && length($2) <= 15 && ($3 ~ /^[0-9a-f]+$/) && length($3) == 64 && NF >= 4
    for (i = 4; i <= NF; i++) { good = (base != "" && index($i, base "/") == 1); for (k = 1; k <= nh; k++) if (index($i, h[k]) == 1) good = 1; if (!good) ok = 0 }
    if (!ok) { print "bad line " NR ": " $0 > "/dev/stderr"; bad = 1 } }
  END { exit bad }' "$TMP/files.txt" || fail "The file list from the release is malformed. Nothing was written."
need=$(awk '{s += $2} END {printf "%.0f\n", s}' "$TMP/files.txt")
gb() { awk -v b="$1" 'BEGIN {printf "%.1f GB", b / 1e9}'; }

case "$(uname -s)-$(uname -m)" in
  Darwin-*)
    say "Installing Alexandria for Mac..."
    get -o "$TMP/app.tar.gz" "$BASE/Alexandria-macos.tar.gz"
    pinned "$TMP/app.tar.gz" "$MAC_SHA"
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/Alexandria.app"
    tar -xzf "$TMP/app.tar.gz" -C "$HOME/Applications"
    APP="$HOME/Applications/Alexandria.app"
    ALEX="$APP/Contents/MacOS/alex"
    OPEN="open \"$APP\""
    ;;
  Linux-x86_64)
    say "Installing Alexandria for Linux..."
    get -o "$TMP/app.tar.gz" "$BASE/alexandria-linux-x64.tar.gz"
    pinned "$TMP/app.tar.gz" "$LINUX_SHA"
    DEST="$HOME/.local/share/alexandria-app"
    rm -rf "$DEST"; mkdir -p "$DEST" "$HOME/.local/bin" "$HOME/.local/share/applications"
    tar -xzf "$TMP/app.tar.gz" -C "$DEST" --strip-components=1
    ln -sf "$DEST/alexandria-app" "$HOME/.local/bin/alexandria"
    cat > "$HOME/.local/share/applications/alexandria.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Alexandria
Comment=Offline library: Wikipedia, medicine, repair guides, maps and a local AI
Exec=$DEST/alexandria-app
Icon=$DEST/icon.png
Categories=Education;Reference;
EOF
    if ldd "$DEST/alexandria-app" | grep -q "not found"; then
      say "Alexandria needs the WebKitGTK library. Install it with:"
      echo "  sudo apt install libwebkit2gtk-4.1-0     (Ubuntu/Debian)"
      echo "  sudo dnf install webkit2gtk4.1           (Fedora)"
    fi
    ALEX="$DEST/alex"
    OPEN="alexandria   (or find Alexandria in your apps menu)"
    ;;
  Linux-*) fail "Linux on $(uname -m) is not supported yet (x86_64 only)." ;;
  *) fail "Unsupported system: $(uname -s). On Windows use install.ps1." ;;
esac


PARENT=$(ask "Where should the library go? (an 'Alexandria' folder is made there)" "${ALEXANDRIA_DIR:-$HOME}")
PARENT=${PARENT/#\~/$HOME}
[ -d "$PARENT" ] || fail "$PARENT does not exist."
LIB="$PARENT/Alexandria"
mkdir -p "$LIB"

have=0
while read -r name bytes _ _; do
  [ -f "$LIB/$name" ] && [ "$(size "$LIB/$name")" = "$bytes" ] && have=$((have + bytes))
done < "$TMP/files.txt"
todo=$((need - have))
index=$((need / 4 + 2000000000))
free=$(free_bytes "$LIB")
say "Library: $(gb "$need") total, $(gb "$todo") to download, plus about $(gb "$index") for search. $(gb "$free") free at $LIB"
[ "$free" -gt $((todo + index + 200000000)) ] || fail "Not enough free space. Free up $(gb $((todo + index + 200000000 - free))) or pick another drive."

n=0; total=$(wc -l < "$TMP/files.txt" | tr -d ' ')
while read -r name bytes want urls; do
  n=$((n + 1))
  f="$LIB/$name"
  if [ -f "$f" ] && [ "$(size "$f")" = "$bytes" ] && [ "$(sha "$f")" = "$want" ]; then
    echo "[$n/$total] $name (already here)"
    continue
  fi
  echo "[$n/$total] $name ($(gb "$bytes"))"
  for ext in part piece done; do [ -L "$f.$ext" ] && rm -f "$f.$ext"; done
  set -f; set -- $urls; set +f
  if [ $# -eq 1 ]; then
    if [ -f "$f.part" ] && [ "$(size "$f.part")" -ge "$bytes" ]; then rm -f "$f.part"; fi
    get -C - -o "$f.part" "$1"
  else
    k=0; got=0
    if [ -f "$f.done" ]; then read -r k got < "$f.done" || { k=0; got=0; }; fi
    if [ -f "$f.part" ]; then
      part_size=$(size "$f.part")
      if [ "$part_size" -gt "$got" ]; then
        dd if=/dev/null of="$f.part" bs=1 seek="$got" 2>/dev/null
      elif [ "$part_size" -lt "$got" ]; then
        k=0; got=0
        : > "$f.part"
      fi
    else
      k=0; got=0
      : > "$f.part"
    fi
    i=0
    for u in "$@"; do
      i=$((i + 1))
      [ "$i" -le "$k" ] && continue
      echo "  part $i of $#"
      rm -f "$f.piece"
      get -C - -o "$f.piece" "$u"
      cat "$f.piece" >> "$f.part"
      echo "$i $(size "$f.part")" > "$f.done"
    done
    rm -f "$f.piece" "$f.done"
  fi
  [ "$(size "$f.part")" = "$bytes" ] || { rm -f "$f.part" "$f.done"; fail "$name downloaded with the wrong size. Run the installer again to resume."; }
  [ "$(sha "$f.part")" = "$want" ] || { rm -f "$f.part" "$f.done"; fail "$name failed its checksum. Run the installer again."; }
  mv "$f.part" "$f"
done < "$TMP/files.txt"

say "Checking every file against the release signature (a minute for Lite, longer for a large library)..."
"$ALEX" verify "$LIB" "$ROOT_KEY" > "$TMP/verify.txt" 2>&1 || { tail -3 "$TMP/verify.txt" >&2; fail "The library did not pass verification."; }
tail -1 "$TMP/verify.txt"

say "Preparing search on this computer (5 to 30 minutes for Lite, a few hours for a large library; it can resume)..."
"$ALEX" index-library "$LIB" "$ROOT_KEY" || fail "Building search failed. Run the installer again to resume."

case "$(uname -s)" in
  Darwin) CONF="$HOME/Library/Application Support/Alexandria" ;;
  *) CONF="${XDG_CONFIG_HOME:-$HOME/.config}/alexandria" ;;
esac
mkdir -p "$CONF" && printf '%s\n' "$LIB" > "$CONF/library-path"

say "Done. Alexandria is installed and works offline from now on."
echo "Open it:  $OPEN"
echo "Library:  $LIB"
}

main "$@"
