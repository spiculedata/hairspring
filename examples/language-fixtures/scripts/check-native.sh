#!/bin/sh
# Native self-check of a lost-update fixture artifact (no Hermit, no engine).
#
#   check-native.sh <lang-id> <artifact> [runs]
#
# The artifact is one program that takes its mode as argv[1]: buggy, fixed or wide. This script checks the whole
# fixture contract, natively:
#   - argv: no argument, an unknown mode, a wrong-case mode and an extra argument each print a usage line on stderr,
#     print nothing on stdout, and exit 64;
#   - every run of buggy, fixed and wide (`runs` of buggy and fixed, $WIDE_RUNS of wide, default 10) exits 0 whatever
#     the property says and prints exactly one stdout line,
#     `LANGAT v1 lang=<id> mode=<mode> final=<20 digits> expect=0000000000000000<4000> order=<16 hex> rand=<16 hex>`;
#   - the protocol report on stderr has one evaluation each of <id>.counter_exact (witness values
#     [final, 4000, 4, 1000]) and <id>.all_started, and no incomplete-catalog notice from the SDK's audit;
#   - the detector and the program agree: the property is false exactly when final != 4000, and final never exceeds it.
# Then, across the runs:
#   - fixed never fires and always ends at 4000;
#   - buggy fires at least once. On a host where this process may use only one CPU (nproc respects affinity and
#     cgroup cpusets) four threads cannot overlap, so a buggy mode that never fires there ends the check with the
#     exit code 77 (skipped), after the other checks. LANG_AT_REQUIRE_PARALLEL=1 turns that skip into a failure (the
#     build record's check must run on at least two CPUs);
#   - wide is only checked against the contract; how often it fires is printed.
# Each run is limited to $RUN_TIMEOUT seconds (default 60) when `timeout` or `gtimeout` is installed.
# Prints one summary line per mode. Exits 0 when everything holds, 77 when the buggy check was skipped, 1 otherwise,
# 64 for a usage error.

set -u

SKIPPED=77
USAGE_STATUS=64
EXPECTED=4000

if [ $# -lt 2 ]; then
  echo "usage: $0 <lang-id> <artifact> [runs]" >&2
  exit 64
fi
lang=$1
artifact=$2
runs=${3:-50}
wide_runs=${WIDE_RUNS:-10}
property="$lang.counter_exact"
coverage="$lang.all_started"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

limit=""
for t in timeout gtimeout; do
  if command -v "$t" >/dev/null 2>&1; then
    limit="$t ${RUN_TIMEOUT:-60}"
    break
  fi
done

# run_artifact <args...>: runs the artifact with the SDK's environment removed; stdout in $tmp/out, stderr in $tmp/err.
run_artifact() {
  # shellcheck disable=SC2086  # $limit is a command and its argument on purpose
  env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $limit "$artifact" "$@" >"$tmp/out" 2>"$tmp/err"
}

# usage_checked <description> <args...>: the artifact must reject these arguments with the usage status.
usage_checked() {
  what=$1
  shift
  run_artifact "$@"
  status=$?
  if [ "$status" -ne "$USAGE_STATUS" ]; then
    echo "FAIL usage ($what): exit status $status, want $USAGE_STATUS" >&2
    return 1
  fi
  if [ -s "$tmp/out" ] || [ ! -s "$tmp/err" ]; then
    echo "FAIL usage ($what): want a usage line on stderr and nothing on stdout" >&2
    return 1
  fi
}

check_usage() {
  usage_checked "no argument" || return 1
  usage_checked "unknown mode" racy || return 1
  usage_checked "wrong case" Buggy || return 1
  usage_checked "extra argument" buggy extra || return 1
}

# one_run <mode> <index>: the whole contract for one run; prints "<fired> <exact>" on success, a reason and 1 if not.
one_run() {
  mode=$1
  i=$2
  run_artifact "$mode"
  status=$?
  if [ "$status" -eq 124 ] && [ -n "$limit" ]; then
    echo "FAIL $mode run $i: no result within the time limit ($limit)" >&2
    return 1
  fi
  if [ "$status" -ne 0 ]; then
    echo "FAIL $mode run $i: exit status $status, want 0" >&2
    return 1
  fi
  shape="^LANGAT v1 lang=$lang mode=$mode final=[0-9]{20} expect=0{16}$EXPECTED order=[0-9a-f]{16} rand=[0-9a-f]{16}$"
  if ! grep -Eq "$shape" "$tmp/out" || [ "$(wc -l <"$tmp/out")" -ne 1 ]; then
    echo "FAIL $mode run $i: stdout is not the one contract line: $(cat "$tmp/out")" >&2
    return 1
  fi
  final=$(sed -n 's/^.* final=\([0-9]*\) expect=.*/\1/p' "$tmp/out" | sed 's/^0*\(.\)/\1/')
  assert_line=$(grep "\"kind\":\"assert\".*\"id\":\"$property\"" "$tmp/err")
  start_line=$(grep "\"kind\":\"assert\".*\"id\":\"$coverage\"" "$tmp/err")
  if [ -z "$assert_line" ] || [ -z "$start_line" ]; then
    echo "FAIL $mode run $i: the protocol report lacks the assert lines for $property and $coverage" >&2
    return 1
  fi
  for line in "$assert_line" "$start_line"; do
    if ! echo "$line" | grep -Eq '"hits": *1,'; then
      echo "FAIL $mode run $i: a property was not evaluated exactly once: $line" >&2
      return 1
    fi
  done
  if ! echo "$assert_line" | grep -Eq "\"vals\":\[$final,$EXPECTED,4,1000\]"; then
    echo "FAIL $mode run $i: the witness values of $property are not [$final,$EXPECTED,4,1000]: $assert_line" >&2
    return 1
  fi
  if grep -q 'incomplete catalog' "$tmp/err"; then
    echo "FAIL $mode run $i: the SDK's audit reports call sites missing from the catalog" >&2
    return 1
  fi
  falses=$(echo "$assert_line" | sed 's/.*"falses": *\([0-9]*\).*/\1/')
  if [ "$falses" -gt 0 ] && [ "$final" -eq "$EXPECTED" ]; then
    echo "FAIL $mode run $i: property false but final=$final" >&2
    return 1
  fi
  if [ "$falses" -eq 0 ] && [ "$final" -ne "$EXPECTED" ]; then
    echo "FAIL $mode run $i: final=$final but the property did not fire" >&2
    return 1
  fi
  if [ "$final" -gt "$EXPECTED" ]; then
    echo "FAIL $mode run $i: final=$final is above $EXPECTED: the counter cannot gain updates" >&2
    return 1
  fi
  fired=0
  exact=0
  [ "$falses" -gt 0 ] && fired=1
  [ "$final" -eq "$EXPECTED" ] && exact=1
  echo "$fired $exact"
}

# counted <mode> <runs>: runs the mode and prints "<fired> <exact>", the number of runs in which the property was
# false and in which the final value was exact. Returns 1 on a contract violation (already reported on stderr).
counted() {
  mode=$1
  total=$2
  fired_total=0
  exact_total=0
  i=0
  while [ "$i" -lt "$total" ]; do
    i=$((i + 1))
    result=$(one_run "$mode" "$i") || return 1
    fired_total=$((fired_total + ${result% *}))
    exact_total=$((exact_total + ${result#* }))
  done
  echo "$fired_total $exact_total"
}

check_usage || exit 1
buggy_counts=$(counted buggy "$runs") || exit 1
fixed_counts=$(counted fixed "$runs") || exit 1
wide_counts=$(counted wide "$wide_runs") || exit 1
buggy_fired=${buggy_counts% *}
fixed_fired=${fixed_counts% *}
fixed_exact=${fixed_counts#* }
wide_fired=${wide_counts% *}

echo "$lang buggy: property false in $buggy_fired of $runs native runs"
echo "$lang fixed: property false in $fixed_fired of $runs native runs, final exact in $fixed_exact"
echo "$lang wide:  property false in $wide_fired of $wide_runs native runs"

status=0
if [ "$fixed_fired" -ne 0 ] || [ "$fixed_exact" -ne "$runs" ]; then
  echo "FAIL the fixed mode lost an update" >&2
  status=1
fi
cpus=${LANG_AT_CPUS:-$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)}  # LANG_AT_CPUS overrides, for the script tests
if [ "$buggy_fired" -eq 0 ]; then
  if [ "$cpus" -lt 2 ] && [ "${LANG_AT_REQUIRE_PARALLEL:-0}" != 1 ]; then
    echo "skip: the buggy mode never fired, and this process may use $cpus CPU (the race needs overlapping threads)"
    [ $status -eq 0 ] && status=$SKIPPED
  else
    echo "FAIL the buggy mode never fired in $runs runs on $cpus CPUs: the planted race is not reachable" >&2
    status=1
  fi
fi
exit $status
