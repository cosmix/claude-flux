#!/usr/bin/env bash
# Smoke test for the opt-in browser terminal lane. Usage: smoke-web-terminal.sh <loom-binary>
set -euo pipefail

bin=${1:?usage: $0 <loom-binary>}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
out=$(mktemp "${TMPDIR:-/tmp}/loom-web-terminal-smoke.XXXXXX")
work=$(mktemp -d "${TMPDIR:-/tmp}/loom-web-terminal-smoke-work.XXXXXX") && [ -n "$work" ] && mkdir -p "$work/.loom/work"
(cd "$work" && exec "$bin" status --web 0 --terminals) >"$out" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -f "$out"; rm -rf "$work"' EXIT

url=""
for _ in $(seq 1 100); do
  url=$(rg -o 'http://127\.0\.0\.1:[0-9]+/\?token=[0-9a-f]+' "$out" | head -1 || true)
  [ -n "$url" ] && break
  sleep 0.1
done
[ -n "$url" ] || { echo "server did not print its URL:"; cat "$out"; exit 1; }

port=${url#http://127.0.0.1:}
port=${port%%/*}
token=${url#*token=}
base="http://127.0.0.1:$port"
cookie="loom_dashboard_${port}=${token}"
origin="http://127.0.0.1:$port"

headers=$(mktemp "${TMPDIR:-/tmp}/loom-web-terminal-headers.XXXXXX")
trap 'kill "$pid" 2>/dev/null || true; rm -f "$out" "$headers"; rm -rf "$work"' EXIT
code=$(curl -s -D "$headers" -o /dev/null -w '%{http_code}' "$base/?token=$token")
[ "$code" = "302" ] || { echo "expected token bootstrap 302, got $code"; exit 1; }
rg -qi "^set-cookie: loom_dashboard_${port}=" "$headers"
code=$(curl -s -o /dev/null -w '%{http_code}' "$base/?token=wrong")
[ "$code" = "403" ] || { echo "expected wrong token 403, got $code"; exit 1; }

websocket_code() {
  curl -s -o /dev/null -w '%{http_code}' --max-time 2 \
    -H 'Upgrade: websocket' \
    -H 'Connection: Upgrade' \
    -H 'Sec-WebSocket-Version: 13' \
    -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    "$@"
}

code=$(websocket_code -H "Origin: $origin" "$base/ws/terminal/x/view")
[ "$code" = "401" ] || { echo "expected missing cookie 401, got $code"; exit 1; }
code=$(websocket_code -H "Cookie: $cookie" -H 'Origin: http://evil.example' "$base/ws/terminal/x/view")
[ "$code" = "403" ] || { echo "expected foreign origin 403, got $code"; exit 1; }
code=$(websocket_code -H "Cookie: $cookie" -H "Origin: http://127.0.0.1:$((port + 1))" "$base/ws/terminal/x/view")
[ "$code" = "403" ] || { echo "expected wrong loopback port 403, got $code"; exit 1; }

code=$(websocket_code -H "Cookie: $cookie" -H "Origin: $origin" "$base/ws/terminal/x/view" || true)
if [ "$code" = "000" ]; then
  { curl -si --max-time 2 \
      -H 'Upgrade: websocket' \
      -H 'Connection: Upgrade' \
      -H 'Sec-WebSocket-Version: 13' \
      -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
      -H "Cookie: $cookie" \
      -H "Origin: $origin" \
      "$base/ws/terminal/x/view" || true; } \
    | rg -q '^HTTP/1.1 101' || { echo "valid upgrade did not return 101"; exit 1; }
elif [ "$code" != "101" ]; then
  echo "expected valid upgrade 101, got $code"
  exit 1
fi

python3 - "$port" "$cookie" "$origin" <<'PY'
import socket
import sys

port, cookie, origin = sys.argv[1:]
request = (
    "GET /ws/terminal/definitely-not-a-stage/view HTTP/1.1\r\n"
    f"Host: 127.0.0.1:{port}\r\n"
    "Upgrade: websocket\r\n"
    "Connection: Upgrade\r\n"
    "Sec-WebSocket-Version: 13\r\n"
    "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    f"Cookie: {cookie}\r\n"
    f"Origin: {origin}\r\n\r\n"
).encode()
sock = socket.create_connection(("127.0.0.1", int(port)), timeout=2)
sock.sendall(request)
data = b""
while b"\r\n\r\n" not in data:
    data += sock.recv(4096)
head, frame = data.split(b"\r\n\r\n", 1)
assert head.startswith(b"HTTP/1.1 101"), head
while len(frame) < 2:
    frame += sock.recv(4096)
assert frame[0] & 0x0f == 0x8, frame
length = frame[1] & 0x7f
if length == 126:
    while len(frame) < 4:
        frame += sock.recv(4096)
    length = int.from_bytes(frame[2:4], "big")
    offset = 4
else:
    offset = 2
while len(frame) < offset + length:
    frame += sock.recv(4096)
assert int.from_bytes(frame[offset:offset + 2], "big") == 4004, frame
sock.close()
PY

curl -fsS "$base/api/status" | jq -e '.terminals == true' >/dev/null
page=$(curl -fsS "$base/")
printf '%s' "$page" | rg -qF 'assets/index.js'
found=false
for asset in web/dist/assets/*.js; do
  name=${asset##*/}
  [ "$name" = "index.js" ] && continue
  if curl -fsS "$base/assets/$name" | rg -qF 'xterm-screen'; then
    found=true
    break
  fi
done
[ "$found" = true ] || { echo "embedded terminal bundle lacks xterm-screen"; exit 1; }

echo "smoke-web-terminal: ok"
