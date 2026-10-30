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
#
# For release builds only. A debug build of the server reads web/dist from disk when it runs instead of embedding it, so
# it would always pass; the check is about the bytes compiled into a release binary.
set -u

BINARY="${1:-}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST="${2:-$HERE/../web/dist}"
[ -n "$BINARY" ] && [ -x "$BINARY" ] || { echo "usage: $0 BINARY [DIST_DIR]  (BINARY must be an executable file)" >&2; exit 2; }
# The server is started from a scratch directory, so a relative path to the binary must be made absolute first.
case "$BINARY" in /*) ;; *) BINARY="$PWD/$BINARY" ;; esac
[ -f "$DIST/index.html" ] || { echo "no $DIST/index.html: run 'npm run build' in web/ first" >&2; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-ui-check.XXXXXX")"
SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# Port 0 lets the system pick a free port, and the server says which one it got, so the fetches below can only reach
# the server this script started (a fixed or random port could belong to something else already listening).
(cd "$WORK" && exec "$BINARY" --db "$WORK/ui-check.db" serve --addr 127.0.0.1:0 --workers 0) >"$WORK/server.log" 2>&1 &
SERVER_PID=$!

BASE=""
for _ in $(seq 1 100); do
  BASE="$(sed -n 's|^hairspring: listening on \(http://[0-9.]*:[0-9]*\).*|\1|p' "$WORK/server.log" | head -n 1)"
  [ -z "$BASE" ] || break
  kill -0 "$SERVER_PID" 2>/dev/null || break
  sleep 0.2
done
if [ -z "$BASE" ]; then
  echo "FAIL: the server did not say where it is listening" >&2
  sed 's/^/    | /' "$WORK/server.log" >&2
  exit 1
fi

# Every request is bounded, so a server that accepts and never answers fails the check instead of hanging it.
fetch() { curl -fsS --connect-timeout 3 --max-time 10 -o "$2" "$BASE$1" 2>/dev/null; }

if ! fetch / "$WORK/index.html"; then
  echo "FAIL: the server did not serve $BASE/ (within 10 seconds)" >&2
  sed 's/^/    | /' "$WORK/server.log" >&2
  exit 1
fi
kill -0 "$SERVER_PID" 2>/dev/null || { echo "FAIL: the server exited before it could be checked" >&2; exit 1; }

status=0
differ() { echo "FAIL: $1" >&2; status=1; }

if grep -q "has not been built into this binary" "$WORK/index.html"; then
  differ "the binary was built without the web UI (it serves the 'UI not built' page)"
elif ! cmp -s "$WORK/index.html" "$DIST/index.html"; then
  differ "the binary's index.html is not the one in $DIST: the UI inside it is from a different build"
fi

# Assets are content-hashed by the build, so a file with the same name has the same content; compare the bytes anyway.
for asset in $(grep -o '/assets/[A-Za-z0-9._-]*' "$DIST/index.html" | sort -u); do
  if ! fetch "$asset" "$WORK/asset"; then
    differ "the binary does not serve $asset"
  elif ! cmp -s "$WORK/asset" "$DIST$asset" 2>/dev/null; then
    differ "$asset differs from the build in $DIST"
  fi
done

[ "$status" -ne 0 ] || echo "OK: the binary serves the UI built in $DIST"
exit "$status"
