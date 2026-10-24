#!/bin/sh
# Native tests of the Java probe: the line format, the fields that do not depend on the schedule, usage errors and the
# exit status of a stdout that cannot be written. No Hermit, no engine.
#
#   test-probe.sh <lang-at.jar> <test-classes-dir>
#
# Environment: JAVA is the java command (default java), JAVA_FLAGS are extra JVM flags put before -cp (default none, so the
# defaults of the JVM; the flag profiles are checked by scripts/check-java-profiles.sh), RUNS is the number of runs of the
# line checks (default 20). The test classes are only used to list the 24 hashes a thread order can have.
# Exits 0 when every test passed, 1 when one failed, 64 for a usage error.

set -u

if [ $# -ne 2 ]; then
  echo "usage: $0 <lang-at.jar> <test-classes-dir>" >&2
  exit 64
fi
jar=$1
tests=$2
java=${JAVA:-java}
runs=${RUNS:-20}
flags=${JAVA_FLAGS:-}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/probe-java-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0
line_rx='^PROBE v1 lang=java lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'

ok() { echo "ok   $1"; }
bad() {
  echo "FAIL $1"
  failures=$((failures + 1))
}

# probe <stdout-file> <stderr-file> [args...]: one run of the probe; prints the exit status.
probe() {
  out=$1
  err=$2
  shift 2
  # shellcheck disable=SC2086  # flags is a list of words on purpose
  "$java" $flags -cp "$jar" langat.Probe "$@" >"$out" 2>"$err"
  echo $?
}

# Every run: exit 0, empty stderr, exactly one stdout line in the six-field form.
: >"$tmp/lines.txt"
i=0
format_ok=1
while [ "$i" -lt "$runs" ]; do
  rc=$(probe "$tmp/out.txt" "$tmp/err.txt")
  if [ "$rc" -ne 0 ]; then
    echo "run $i: exit status $rc, want 0"
    format_ok=0
  elif [ -s "$tmp/err.txt" ]; then
    echo "run $i: something on stderr:"
    cat "$tmp/err.txt"
    format_ok=0
  elif [ "$(wc -l <"$tmp/out.txt" | tr -d ' ')" -ne 1 ]; then
    echo "run $i: stdout is not exactly one line:"
    cat "$tmp/out.txt"
    format_ok=0
  elif ! grep -Eq "$line_rx" "$tmp/out.txt"; then
    echo "run $i: not the six-field line:"
    cat "$tmp/out.txt"
    format_ok=0
  else
    cat "$tmp/out.txt" >>"$tmp/lines.txt"
  fi
  i=$((i + 1))
done
if [ "$format_ok" -eq 1 ]; then ok "$runs runs: exit 0, empty stderr, one line in the six-field form"; else bad "the line checks"; fi

# Fields that do not depend on the schedule.
if [ "$format_ok" -eq 1 ]; then
  distinct=$(tr ' ' '\n' <"$tmp/lines.txt" | grep '^extra=' | sort -u | wc -l | tr -d ' ')
  if [ "$distinct" -eq 1 ]; then ok "extra (the map order) is the same in every run"; else bad "extra has $distinct distinct values over $runs runs"; fi
  "$java" -cp "$jar:$tests" langat.SelfTest --order-hashes >"$tmp/orders.txt" 2>/dev/null
  if [ "$(wc -l <"$tmp/orders.txt" | tr -d ' ')" -eq 24 ]; then
    stray=0
    for name in lock_order arrival; do
      for value in $(tr ' ' '\n' <"$tmp/lines.txt" | sed -n "s/^$name=//p"); do
        grep -qx "$value" "$tmp/orders.txt" || stray=1
      done
    done
    if [ "$stray" -eq 0 ]; then ok "lock_order and arrival are hashes of a permutation of the four worker numbers"; else bad "a lock_order or arrival is not a hash of a permutation of 0 to 3"; fi
  else
    bad "could not list the 24 order hashes"
  fi
  if [ -z "$(tr ' ' '\n' <"$tmp/lines.txt" | sed -n 's/^vtime_ns=0*//p' | grep -vx '' | head -1)" ]; then bad "vtime_ns is zero in every run"; else ok "vtime_ns is positive"; fi
fi

# Any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout.
usage_ok=1
for arg in x --switch=1.0 buggy ""; do
  rc=$(probe "$tmp/out.txt" "$tmp/err.txt" "$arg")
  if [ "$rc" -ne 64 ] || [ -s "$tmp/out.txt" ] || [ ! -s "$tmp/err.txt" ]; then
    echo "the argument '$arg': exit status $rc, stdout $(wc -c <"$tmp/out.txt" | tr -d ' ') bytes, stderr $(wc -c <"$tmp/err.txt" | tr -d ' ') bytes"
    usage_ok=0
  fi
done
rc=$(probe "$tmp/out.txt" "$tmp/err.txt" a b)
[ "$rc" -eq 64 ] || usage_ok=0
if [ "$usage_ok" -eq 1 ]; then ok "any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout"; else bad "usage errors"; fi

# A stdout that cannot be written is an internal failure: exit 3, a message on stderr.
if [ -w /dev/full ]; then
  # shellcheck disable=SC2086
  "$java" $flags -cp "$jar" langat.Probe >/dev/full 2>"$tmp/err.txt"
  rc=$?
  if [ "$rc" -eq 3 ] && [ -s "$tmp/err.txt" ]; then ok "a full stdout (/dev/full) exits 3 with a message on stderr"; else bad "a full stdout: exit status $rc, want 3 and a message"; fi
else
  echo "skip a full stdout exits 3 (no /dev/full here)"
fi

exit $failures
