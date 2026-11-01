#!/usr/bin/env bash
# Assemble the PC package: VNShelf-<version>-<platform>/{server,client} + zip.
#   cargo build --release -p vndb-web -p vndb-explorer
#   ./desktop/package.sh windows-x64|linux-x64|macos-arm64 [binary dir]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLATFORM="${1:?platform: windows-x64, linux-x64 or macos-arm64}"
BIN="${2:-$ROOT/target/release}"
VERSION="$(tr -d ' \r\n' < "$ROOT/VERSION")"
EXE=""
[[ "$PLATFORM" == windows* ]] && EXE=".exe"
NAME="VNShelf-$VERSION-$PLATFORM"
DIST="$ROOT/target/dist"
OUT="$DIST/$NAME"

rm -rf "$OUT" "$DIST/$NAME.zip"
mkdir -p "$OUT/server/data" "$OUT/client/web" "$OUT/client/cli"

cp "$BIN/vndb-web$EXE" "$OUT/server/"
cp "$BIN/vndb-explorer$EXE" "$OUT/client/cli/"
cp -R "$ROOT/web/." "$OUT/client/web/"
cp "$ROOT/crates/core/assets/shelves.json" "$OUT/server/shelves.example.json"
cp "$ROOT/crates/core/assets/sources.json" "$OUT/server/sources.example.json"
printf 'Здесь хранятся база VNDB (catalog.sqlite), ваши полки (user.sqlite) и кэш обложек.\n' > "$OUT/server/data/README.txt"
sed "s/{VERSION}/$VERSION/" "$ROOT/desktop/README.txt" > "$OUT/README.txt"

if [[ "$PLATFORM" == windows* ]]; then
  cp -R "$ROOT/desktop/windows/." "$OUT/"
  # cmd.exe wants CRLF line endings; text files too, for Notepad
  find "$OUT" -maxdepth 3 \( -name '*.bat' -o -name 'README.txt' \) -print0 | while IFS= read -r -d '' f; do
    sed -i.bak 's/\r$//; s/$/\r/' "$f" && rm -f "$f.bak"
  done
else
  cp -R "$ROOT/desktop/unix/." "$OUT/"
  if [[ "$PLATFORM" == macos* ]]; then
    cp "$OUT/start-vn-shelf.sh" "$OUT/Start VN Shelf.command"
  fi
  chmod +x "$OUT"/*.sh "$OUT"/server/*.sh "$OUT"/client/*.sh "$OUT/server/vndb-web" "$OUT/client/cli/vndb-explorer"
  [[ -f "$OUT/Start VN Shelf.command" ]] && chmod +x "$OUT/Start VN Shelf.command"
fi

cd "$DIST"
if command -v zip > /dev/null 2>&1; then
  zip -qr -X "$NAME.zip" "$NAME"
elif command -v 7z > /dev/null 2>&1; then
  7z a -tzip -bso0 -bsp0 "$NAME.zip" "$NAME"
else
  powershell -NoProfile -Command "Compress-Archive -Path '$NAME' -DestinationPath '$NAME.zip'"
fi
echo "Package: $DIST/$NAME.zip"
