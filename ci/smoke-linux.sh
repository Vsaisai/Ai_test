#!/usr/bin/env bash
set -euo pipefail
archive="${1:?Pass the Linux ZIP path}"
tmp="$(mktemp -d)"
pid=
cleanup() {
  if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT
unzip -q "$archive" -d "$tmp"
packages=("$tmp"/VNShelf-*-linux-x64)
test "${#packages[@]}" -eq 1
cd "${packages[0]}"
test -x start-vn-shelf.sh
client/cli/vndb-explorer --help >/dev/null
server/vndb-web --data "$tmp/data" serve --addr 127.0.0.1:18080 --web client/web > "$tmp/server.log" 2>&1 &
pid=$!
base=http://127.0.0.1:18080
for _ in {1..20}; do
  if curl -fsS --max-time 2 "$base/api/status" -o "$tmp/status.json" 2>/dev/null; then
    python3 -c 'import json, sys; assert isinstance(json.load(open(sys.argv[1])), dict)' "$tmp/status.json"
    curl -fsS --max-time 5 "$base/" -o "$tmp/index.html"
    cmp "$tmp/index.html" client/web/index.html
    curl -fsS --max-time 5 "$base/js/app.js" -o "$tmp/app.js"
    cmp "$tmp/app.js" client/web/js/app.js
    exit 0
  fi
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done
cat "$tmp/server.log" >&2
exit 1
