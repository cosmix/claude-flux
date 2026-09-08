#!/usr/bin/env bash
# Smoke test for `loom status --web`. Usage: smoke-web-dashboard.sh <loom-binary>
set -euo pipefail

bin=${1:?usage: $0 <loom-binary>}
# `loom status --web` resolves its work directory from the current directory and
# exits before binding when there is none. `.loom/work` is gitignored, so a clean
# checkout has no such directory: the server runs from a scratch workspace of its
# own rather than from wherever the caller happens to stand. That makes the
# binary path relative to the caller, so resolve it before the subshell moves.
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
out=$(mktemp "${TMPDIR:-/tmp}/loom-web-smoke.XXXXXX")
work=$(mktemp -d "${TMPDIR:-/tmp}/loom-web-smoke-work.XXXXXX") && [ -n "$work" ] && mkdir -p "$work/.loom/work"
(cd "$work" && exec "$bin" status --web 0) >"$out" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -f "$out"; rm -rf "$work"' EXIT

port=""
for _ in $(seq 1 100); do
  port=$(rg -o 'http://127\.0\.0\.1:[0-9]+' "$out" | head -1 | rg -o '[0-9]+$' || true)
  [ -n "$port" ] && break
  sleep 0.1
done
[ -n "$port" ] || { echo "server did not print its URL:"; cat "$out"; exit 1; }

base="http://127.0.0.1:$port"
page=$(curl -fsS "$base/")
printf '%s' "$page" | rg -q '<div id="root">'
printf '%s' "$page" | rg -qF 'assets/index.js'
curl -fsS "$base/api/status" | jq -e '.status.stages | type == "array"' >/dev/null
curl -fsS "$base/stages/anything" | rg -q '<div id="root">'
curl -fsS -o /dev/null -w '%{content_type}\n' "$base/assets/index.js" | rg -q '^text/javascript'
code=$(curl -s -o /dev/null -w '%{http_code}' "$base/assets/nope.js")
[ "$code" = "404" ] || { echo "expected 404 for a missing asset, got $code"; exit 1; }
code=$(curl -s -o /dev/null -w '%{http_code}' "$base/ws/terminal/x/view")
[ "$code" = "404" ] || { echo "expected 404 for a plain terminal route, got $code"; exit 1; }
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Origin: http://evil.example' "$base/api/status")
[ "$code" = "403" ] || { echo "expected 403 for a foreign origin, got $code"; exit 1; }
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "$base/api/status")
[ "$code" = "403" ] || { echo "expected 403 for a rebound host, got $code"; exit 1; }
echo "smoke-web-dashboard: ok on port $port"
