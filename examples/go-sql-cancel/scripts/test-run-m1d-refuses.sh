#!/bin/sh
# Test: run-m1d.sh refuses an existing output directory, exits non-zero before building or running anything, and
# leaves the directory's contents untouched. Fast: no toolchain is used.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
mkdir "$dir/existing"
echo "keep me" >"$dir/existing/evidence.txt"

status=0
M1D_RESULTS="$dir/existing" sh "$here/scripts/run-m1d.sh" >"$dir/out" 2>"$dir/err" || status=$?

fail() { echo "FAIL: $1" >&2; exit 1; }
[ "$status" -ne 0 ] || fail "run-m1d.sh succeeded on an existing directory"
grep -q "refusing" "$dir/err" || fail "no refusal message: $(cat "$dir/err")"
[ "$(cat "$dir/existing/evidence.txt")" = "keep me" ] || fail "existing file changed"
[ "$(ls "$dir/existing" | wc -l | tr -d ' ')" = 1 ] || fail "files were added to the existing directory"
[ ! -s "$dir/out" ] || fail "something ran: $(cat "$dir/out")"
echo "ok: run-m1d.sh refuses an existing output directory (exit $status)"
