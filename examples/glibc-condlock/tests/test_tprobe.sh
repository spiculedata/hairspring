#!/bin/sh
# Native (macOS or Linux) tests of the T-probe program. The fast-forward measurement itself needs the engine and is not
# a Mac result: here only the program's own behaviour is checked, with a 1 second sleep instead of 60.
# Usage: test_tprobe.sh <build dir>
set -u

build=${1:?usage: test_tprobe.sh <build dir>}
tp=$build/tprobe
tmp=$(mktemp -d "${TMPDIR:-/tmp}/condlock-tprobe.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
checks=0
failed=0

check() {
  name=$1; shift
  checks=$((checks + 1))
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; failed=$((failed + 1)); fi
}
run() { code=0; "$tp" "$@" >"$tmp/out" 2>"$tmp/err" || code=$?; echo "$code" >"$tmp/code"; }
code_is() { [ "$(cat "$tmp/code")" = "$1" ]; }
out_has() { grep -q -- "$1" "$tmp/out"; }

run i 1
check "case i: exit 0" code_is 0
check "case i: A woke B after its sleep" out_has '^tprobe i ok slept=1 woke=1$'

run ii 1000
check "case ii: exit 0" code_is 0
check "case ii: 1000 ping-pongs" out_has '^tprobe ii ok pingpongs=1000$'

run iii 1
if [ "$(uname -s)" = Linux ]; then
  check "case iii (Linux): exit 0" code_is 0
  check "case iii (Linux): raw clock_nanosleep woke B" out_has '^tprobe iii ok slept=1 woke=1$'
else
  check "case iii (not Linux): exit 77, NOT-RUN with the reason" code_is 77
  check "case iii (not Linux): says NOT-RUN" out_has '^tprobe iii NOT-RUN: '
fi

run
check "no case: usage, exit 64" code_is 64
run iv 1
check "unknown case: exit 64" code_is 64
run i 0
check "zero seconds: exit 64" code_is 64
run i 99999
check "absurd seconds: exit 64" code_is 64

echo "$checks checks, $failed failed"
[ "$failed" = 0 ]
