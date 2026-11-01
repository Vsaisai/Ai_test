#!/usr/bin/env bash
# Сервер в этом терминале (видно журнал). Ctrl+C — остановить.
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/config.sh"
echo "Откройте в браузере: http://127.0.0.1:$VNSHELF_PORT/"
exec "$HERE/vndb-web" --data "$HERE/data" serve --addr "127.0.0.1:$VNSHELF_PORT" --web "$HERE/../client/web"
