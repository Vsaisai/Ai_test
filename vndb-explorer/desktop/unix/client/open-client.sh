#!/usr/bin/env bash
# Opens VN Shelf as an app window (Chrome/Chromium/Edge --app) or in the default browser.
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/../server/config.sh"
URL="http://127.0.0.1:$VNSHELF_PORT/"
if [ "$(uname)" = "Darwin" ]; then
  for app in "Google Chrome" "Microsoft Edge" "Chromium"; do
    if [ -d "/Applications/$app.app" ]; then open -na "$app" --args --app="$URL"; exit 0; fi
  done
  open "$URL"; exit 0
fi
for b in google-chrome google-chrome-stable chromium chromium-browser microsoft-edge; do
  if command -v "$b" > /dev/null 2>&1; then nohup "$b" --app="$URL" > /dev/null 2>&1 & exit 0; fi
done
if command -v xdg-open > /dev/null 2>&1; then xdg-open "$URL" > /dev/null 2>&1 & exit 0; fi
echo "Откройте в браузере: $URL"
