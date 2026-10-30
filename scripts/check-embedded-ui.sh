#!/usr/bin/env bash
# Checks that a hairspring binary serves the web UI that was built from the repository's web/ source.
#
#   scripts/check-embedded-ui.sh BINARY [DIST_DIR]
#
# The UI is compiled into the binary from web/dist when the binary is built, so a binary built after an older (or no)
# `npm run build` carries an older UI than the source. This starts the binary's server on a free loopback port with a
# scratch database, fetches the page and every asset the page names, and compares each with the same file in DIST_DIR
# (default web/dist, which must be a fresh `npm run build`). Exit 0 when every file matches, 1 when one differs or the
# server shows its "UI not built" page, 2 for a usage error.
set -u

BINARY="${1:-}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST="${2:-$HERE/../web/dist}"
[ -n "$BINARY" ] && [ -x "$BINARY" ] || { echo "usage: $0 BINARY [DIST_DIR]  (BINARY must be an executable file)" >&2; exit 2; }
[ -f "$DIST/index.html" ] || { echo "no $DIST/index.html: run 'npm run build' in web/ first" >&2; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-ui-check.XXXXXX")"
SERVER_PID=""
cleanup() {
  [ -z "$SERVER_PID" ] || kill "$SERVER_PID" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

PORT=$(( 20000 + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % 20000 ))
BASE="http://127.0.0.1:$PORT"
(cd "$WORK" && exec "$BINARY" --db "$WORK/ui-check.db" serve --addr "127.0.0.1:$PORT" --workers 0) >"$WORK/server.log" 2>&1 &
SERVER_PID=$!

up=""
for _ in $(seq 1 100); do
  if curl -fsS -o "$WORK/index.html" "$BASE/" 2>/dev/null; then up=1; break; fi
  kill -0 "$SERVER_PID" 2>/dev/null || break
  sleep 0.2
done
[ -n "$up" ] || { echo "FAIL: the server did not answer on $BASE" >&2; sed 's/^/    | /' "$WORK/server.log" >&2; exit 1; }

status=0
differ() { echo "FAIL: $1" >&2; status=1; }

if grep -q "has not been built into this binary" "$WORK/index.html"; then
  differ "the binary was built without the web UI (it serves the 'UI not built' page)"
elif ! cmp -s "$WORK/index.html" "$DIST/index.html"; then
  differ "the binary's index.html is not the one in $DIST: the UI inside it is from a different build"
fi

# Assets are content-hashed by the build, so a file with the same name has the same content; compare the bytes anyway.
for asset in $(grep -o '/assets/[A-Za-z0-9._-]*' "$DIST/index.html" | sort -u); do
  if ! curl -fsS -o "$WORK/asset" "$BASE$asset" 2>/dev/null; then
    differ "the binary does not serve $asset"
  elif ! cmp -s "$WORK/asset" "$DIST$asset" 2>/dev/null; then
    differ "$asset differs from the build in $DIST"
  fi
done

[ "$status" -ne 0 ] || echo "OK: the binary serves the UI built in $DIST"
exit "$status"
