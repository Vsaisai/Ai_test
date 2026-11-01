#!/usr/bin/env bash
HERE="$(cd "$(dirname "$0")" && pwd)"
PID_FILE="$HERE/server/data/server.pid"
if [ -f "$PID_FILE" ] && kill "$(cat "$PID_FILE")" 2> /dev/null; then
  rm -f "$PID_FILE"
  echo "Сервер VN Shelf остановлен."
else
  pkill -x vndb-web && echo "Сервер VN Shelf остановлен." || echo "Сервер VN Shelf не запущен."
fi
