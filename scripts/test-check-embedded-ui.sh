#!/usr/bin/env bash
# Tests for scripts/check-embedded-ui.sh, with a fake `hairspring` that serves a directory over HTTP (needs python3).
#
#   bash scripts/test-check-embedded-ui.sh
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/check-embedded-ui.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-ui-check-tests.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
FAILED=0

# A fake binary: a plain python3 script, so it can be called by a relative path like the real one.
# `fake --db X serve --addr 127.0.0.1:0 --workers 0` binds a free port, prints the line the real server prints, and then
# acts as FAKE_MODE says: serve $FAKE_UI_DIR (default), accept connections and never answer (hang), or exit (die).
cat >"$TMP/fake-hairspring" <<'FAKE'
#!/usr/bin/env python3
import http.server, os, socket, sys, functools, time
mode = os.environ.get("FAKE_MODE", "serve")
if mode == "die":
    sys.exit(1)
if mode == "hang":
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    sock.listen(5)
    print("hairspring: listening on http://127.0.0.1:%d" % sock.getsockname()[1], flush=True)
    while True:
        conn, _ = sock.accept()  # accepted and never answered
        time.sleep(3600)
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=os.environ["FAKE_UI_DIR"])
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
print("hairspring: listening on http://127.0.0.1:%d" % server.server_address[1], flush=True)
server.serve_forever()
FAKE
chmod +x "$TMP/fake-hairspring"

make_ui() { # DIR TITLE ASSET_BODY
  mkdir -p "$1/assets"
  printf '<!doctype html><title>%s</title><script type="module" src="/assets/app.js"></script>\n' "$2" >"$1/index.html"
  printf '%s\n' "$3" >"$1/assets/app.js"
}

expect() { # NAME EXPECTED_RC ACTUAL_RC [OUTPUT_PATTERN]
  if [ "$2" != "$3" ]; then
    echo "FAIL $1: exit $3, expected $2"
    sed 's/^/    | /' "$TMP/out"
    FAILED=1
  elif [ -n "${4:-}" ] && ! grep -q -- "$4" "$TMP/out"; then
    echo "FAIL $1: output lacks '$4'"
    sed 's/^/    | /' "$TMP/out"
    FAILED=1
  else
    echo "ok   $1"
  fi
}

run() { FAKE_UI_DIR="$1" bash "$SCRIPT" "$TMP/fake-hairspring" "$2" >"$TMP/out" 2>&1; }
run_mode() { FAKE_MODE="$1" FAKE_UI_DIR="$TMP/same" bash "$SCRIPT" "$TMP/fake-hairspring" "$TMP/built" >"$TMP/out" 2>&1; }

make_ui "$TMP/built" "Hairspring" "console.log('new')"
make_ui "$TMP/same" "Hairspring" "console.log('new')"
make_ui "$TMP/stale-page" "Hairspring old" "console.log('new')"
make_ui "$TMP/stale-asset" "Hairspring" "console.log('old')"
mkdir -p "$TMP/fallback"
echo '<h1>Hairspring</h1><p>The server is running, but the web UI has not been built into this binary.</p>' >"$TMP/fallback/index.html"

run "$TMP/same" "$TMP/built";        expect "a binary with the same UI passes" 0 $? "OK:"
run "$TMP/stale-page" "$TMP/built";  expect "an older index.html fails" 1 $? "index.html is not the one"
run "$TMP/stale-asset" "$TMP/built"; expect "an older asset fails" 1 $? "assets/app.js differs"
run "$TMP/fallback" "$TMP/built";    expect "a binary built without the UI fails" 1 $? "UI not built"
FAKE_UI_DIR="$TMP/same" bash "$SCRIPT" "$TMP/fake-hairspring" "$TMP/nowhere" >"$TMP/out" 2>&1
expect "a missing build directory is a usage error" 2 $? "npm run build"
bash "$SCRIPT" "$TMP/missing-binary" "$TMP/built" >"$TMP/out" 2>&1
expect "a missing binary is a usage error" 2 $? "usage:"

# A relative path to the binary, from another directory (the way the release workflow and make call it).
(cd "$TMP" && FAKE_UI_DIR="$TMP/same" bash "$SCRIPT" ./fake-hairspring "$TMP/built") >"$TMP/out" 2>&1
expect "a relative path to the binary works from another directory" 0 $? "OK:"

# A server that accepts and never answers must fail within the request timeout, not hang.
start=$SECONDS
# The script is run under a watchdog of its own: if the request timeouts were removed it would hang, and the watchdog
# turns that into a failure instead of a stuck test run.
FAKE_MODE=hang FAKE_UI_DIR="$TMP/same" bash "$SCRIPT" "$TMP/fake-hairspring" "$TMP/built" >"$TMP/out" 2>&1 &
checked=$!
while kill -0 "$checked" 2>/dev/null && [ $((SECONDS - start)) -le 30 ]; do sleep 1; done
if kill -0 "$checked" 2>/dev/null; then
  kill "$checked" 2>/dev/null
  wait "$checked" 2>/dev/null
  rc=124
else
  wait "$checked"
  rc=$?
fi
took=$((SECONDS - start))
expect "a server that never answers fails" 1 "$rc" "did not serve"
if [ "$took" -gt 20 ]; then echo "FAIL the hanging server took ${took}s to fail"; FAILED=1; else echo "ok   the hanging server failed in ${took}s"; fi

# A server that dies without saying where it listens must fail, even if something else is listening somewhere.
run_mode die
expect "a server that exits at once fails" 1 $? "did not say where it is listening"

# Nothing is left running afterwards.
if pgrep -f "$TMP/fake-hairspring" >/dev/null 2>&1; then echo "FAIL a fake server is still running"; FAILED=1; else echo "ok   no server left running"; fi

exit "$FAILED"
