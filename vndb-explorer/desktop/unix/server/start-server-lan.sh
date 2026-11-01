#!/usr/bin/env bash
# Сервер, доступный с телефона в той же сети Wi-Fi. Пароля нет: только для домашней сети.
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/config.sh"
echo "Откройте на телефоне: http://IP-этого-компьютера:$VNSHELF_PORT/"
(hostname -I 2> /dev/null || ipconfig getifaddr en0 2> /dev/null) | sed 's/^/Адрес: /'
exec "$HERE/vndb-web" --data "$HERE/data" serve --addr "0.0.0.0:$VNSHELF_PORT" --web "$HERE/../client/web"
