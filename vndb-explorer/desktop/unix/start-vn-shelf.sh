#!/usr/bin/env bash
# Starts the VN Shelf server in the background (if needed) and opens the client window.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/server/config.sh"
URL="http://127.0.0.1:$VNSHELF_PORT"
running() { curl -fs -m 2 "$URL/api/status" > /dev/null 2>&1; }

if ! running; then
  echo "Запускаю сервер VN Shelf (порт $VNSHELF_PORT)..."
  mkdir -p "$HERE/server/data"
  nohup "$HERE/server/vndb-web" --data "$HERE/server/data" serve --addr "127.0.0.1:$VNSHELF_PORT" --web "$HERE/client/web" \
    > "$HERE/server/data/server.log" 2>&1 &
  echo $! > "$HERE/server/data/server.pid"
  for _ in $(seq 60); do running && break; sleep 1; done
  if ! running; then
    echo "Сервер не запустился, см. $HERE/server/data/server.log" >&2
    exit 1
  fi
fi
"$HERE/client/open-client.sh"
