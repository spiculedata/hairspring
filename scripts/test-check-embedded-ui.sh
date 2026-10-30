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

# A fake binary: `fake --db X serve --addr HOST:PORT --workers N` serves $FAKE_UI_DIR on that address.
cat >"$TMP/fake-hairspring" <<'FAKE'
#!/usr/bin/env bash
while [ $# -gt 0 ]; do
  case "$1" in
    --addr) ADDR="$2"; shift 2 ;;
    *) shift ;;
  esac
done
exec python3 -m http.server "${ADDR##*:}" --bind "${ADDR%%:*}" --directory "$FAKE_UI_DIR"
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

exit "$FAILED"
