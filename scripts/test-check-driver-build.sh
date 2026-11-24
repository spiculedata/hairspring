#!/usr/bin/env bash
# Tests for scripts/check-driver-build.sh, with a fake cargo that "builds" a driver script printing a fixed answer.
#
#   bash scripts/test-check-driver-build.sh
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/check-driver-build.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-driver-build-tests.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
FAILED=0

# The fake cargo writes $FAKE_DRIVER_BODY as the driver, into $CARGO_TARGET_DIR/debug, unless FAKE_NO_BUILD is set.
cat >"$TMP/cargo" <<'FAKE'
#!/usr/bin/env bash
[ -z "${FAKE_NO_BUILD:-}" ] || { echo "error: build failed" >&2; exit 101; }
mkdir -p "$CARGO_TARGET_DIR/debug"
printf '#!/usr/bin/env bash\n%s\n' "$FAKE_DRIVER_BODY" >"$CARGO_TARGET_DIR/debug/dhyve-session-driver"
chmod +x "$CARGO_TARGET_DIR/debug/dhyve-session-driver"
FAKE
chmod +x "$TMP/cargo"

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

run() { FAKE_DRIVER_BODY="$1" HS_CARGO="$TMP/cargo" bash "$SCRIPT" >"$TMP/out" 2>&1; }

run "echo '{\"fake_vmm\":false}'"; expect "a driver with no stand-in programs passes" 0 $? "OK:"
run "echo '{ \"fake_vmm\": false }'"; expect "spacing does not matter" 0 $? "OK:"
run "echo '{\"fake_vmm\":true}'"; expect "a driver with the stand-in programs fails" 1 $? "FAIL:"
run "echo '{}'"; expect "an answer that does not say fails" 1 $? "FAIL:"
run "exit 3"; expect "a driver that will not answer is an error of its own" 2 $? "would not say"
FAKE_NO_BUILD=1 HS_CARGO="$TMP/cargo" bash "$SCRIPT" >"$TMP/out" 2>&1
expect "a build that fails is an error of its own" 2 $? "did not build"

[ "$FAILED" -eq 0 ] && echo "all passed"
exit "$FAILED"
