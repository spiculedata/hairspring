#!/bin/sh
# Native self-check of a lost-update fixture pair (no Hermit, no engine).
#
#   check-native.sh <lang-id> <racy-binary> <locked-binary> [runs]
#
# Runs each binary `runs` times (default 50) and checks, for every run, the whole contract:
#   - exit status 0 whatever the property says;
#   - one stdout line of the fixed shape `lang=<id> final=<10 digits> order=<16 hex> rand=<16 hex>`;
#   - the protocol report on stderr has the assert line of <lang-id>.counter_exact and the coverage marker;
#   - the detector and the program agree: the property is false exactly when final != 4000.
# Then, across the runs:
#   - the locked twin never fires and always ends at 4000;
#   - the racy program fires at least once. On a host where this process may use only one CPU (nproc respects
#     affinity and cgroup cpusets) four threads cannot overlap, so a racy program that never fires there ends the
#     check with the exit code 77 (skipped), after the locked twin has been checked.
# Each run is limited to $RUN_TIMEOUT seconds (default 60) when `timeout` or `gtimeout` is installed.
# Prints one summary line per binary. Exits 0 when everything holds, 77 when the racy check was skipped, 1 otherwise.

set -u

SKIPPED=77

if [ $# -lt 3 ]; then
  echo "usage: $0 <lang-id> <racy-binary> <locked-binary> [runs]" >&2
  exit 64
fi
lang=$1
racy=$2
locked=$3
runs=${4:-50}
expected=4000
property="$lang.counter_exact"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

limit=""
for t in timeout gtimeout; do
  if command -v "$t" >/dev/null 2>&1; then
    limit="$t ${RUN_TIMEOUT:-60}"
    break
  fi
done

# counted <binary> <name>: runs the binary and prints "<fired> <exact>", the number of runs in which the property
# was false and in which the final value was exact. Prints a reason on stderr and returns 1 on a contract violation.
counted() {
  bin=$1
  name=$2
  fired=0
  exact=0
  i=0
  while [ "$i" -lt "$runs" ]; do
    i=$((i + 1))
    # shellcheck disable=SC2086  # $limit is a command and its argument on purpose
    env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $limit "$bin" >"$tmp/out" 2>"$tmp/err"
    status=$?
    if [ "$status" -eq 124 ] && [ -n "$limit" ]; then
      echo "FAIL $name run $i: no result within the time limit ($limit)" >&2
      return 1
    fi
    if [ "$status" -ne 0 ]; then
      echo "FAIL $name run $i: exit status $status, want 0" >&2
      return 1
    fi
    if ! grep -Eq "^lang=$lang +final=[0-9]{10} order=[0-9a-f]{16} rand=[0-9a-f]{16}$" "$tmp/out" || [ "$(wc -l <"$tmp/out")" -ne 1 ]; then
      echo "FAIL $name run $i: stdout is not the one fixed-width line: $(cat "$tmp/out")" >&2
      return 1
    fi
    final=$(sed -n 's/^lang=[^ ]* *final=\([0-9]*\) .*/\1/p' "$tmp/out" | sed 's/^0*\(.\)/\1/')
    assert_line=$(grep "\"kind\":\"assert\".*\"id\":\"$property\"" "$tmp/err")
    if [ -z "$assert_line" ] || ! grep -q "\"kind\":\"assert\".*\"id\":\"$lang.all_started\"" "$tmp/err"; then
      echo "FAIL $name run $i: the protocol report lacks the assert lines for $property and $lang.all_started" >&2
      return 1
    fi
    falses=$(echo "$assert_line" | sed 's/.*"falses": *\([0-9]*\).*/\1/')
    if [ "$falses" -gt 0 ] && [ "$final" -eq "$expected" ]; then
      echo "FAIL $name run $i: property false but final=$final" >&2
      return 1
    fi
    if [ "$falses" -eq 0 ] && [ "$final" -ne "$expected" ]; then
      echo "FAIL $name run $i: final=$final but the property did not fire" >&2
      return 1
    fi
    [ "$falses" -gt 0 ] && fired=$((fired + 1))
    [ "$final" -eq "$expected" ] && exact=$((exact + 1))
  done
  echo "$fired $exact"
}

racy_counts=$(counted "$racy" racy) || exit 1
locked_counts=$(counted "$locked" locked) || exit 1
racy_fired=${racy_counts% *}
locked_fired=${locked_counts% *}
locked_exact=${locked_counts#* }

echo "$lang racy:   property false in $racy_fired of $runs native runs"
echo "$lang locked: property false in $locked_fired of $runs native runs, final exact in $locked_exact"

status=0
if [ "$locked_fired" -ne 0 ] || [ "$locked_exact" -ne "$runs" ]; then
  echo "FAIL the locked twin lost an update" >&2
  status=1
fi
cpus=${LANG_AT_CPUS:-$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)}  # LANG_AT_CPUS overrides, for the script tests
if [ "$racy_fired" -eq 0 ]; then
  if [ "$cpus" -lt 2 ]; then
    echo "skip: the racy program never fired, and this process may use $cpus CPU (the race needs overlapping threads)"
    [ $status -eq 0 ] && status=$SKIPPED
  else
    echo "FAIL the racy program never fired in $runs runs on $cpus CPUs: the planted race is not reachable" >&2
    status=1
  fi
fi
exit $status
