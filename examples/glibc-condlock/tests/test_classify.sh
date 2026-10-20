#!/bin/sh
# Tests of scripts/classify.sh: one fixture per class and precedence rule, then (with a build dir) real runs.
# Usage: test_classify.sh [<build dir>]
set -u

here=$(cd "$(dirname "$0")/.." && pwd)
classify=$here/scripts/classify.sh
build=${1:-}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/condlock-classify.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
checks=0
failed=0

PROP=glibc.cond.signal_wakes_one_of_blocked
STALL=condlock.stall_unexplained
LOCKX=condlock.harness.lock_exclusive

# ---- fixture builders (the SDK's assert/end lines, counters right-aligned like the real ones) ---------------------
WIT='{"thread":"wd-0","ord":                   1,"vals":[%s]}'
assert_line() { # id hits trues falses vals [f: put the witness in first_false instead of first_true]
  if [ "${6:-t}" = f ]; then
    printf 'HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"%s","hits":%20s,"trues":%20s,"falses":%20s,"first_false":'"$WIT"',"first_true":null}\n' "$1" "$2" "$3" "$4" "$5"
  else
    printf 'HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"%s","hits":%20s,"trues":%20s,"falses":%20s,"first_false":null,"first_true":'"$WIT"'}\n' "$1" "$2" "$3" "$4" "$5"
  fi
}
end_line() { # dropped
  printf 'HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":9,"evals":5,"dropped":%20s,"threads":3}\n' "$1"
}
GOOD_OUT='condlock libc=none hdr=none rounds=     200 signals=     200 blocked1=       0 maxb=       0 maxstag=       0 ticks=       0 spanned=       0'

# fixture <name> <exit> <stdout text or -> <stderr builder output...>; writes $tmp/<name>.{code,out,err}
healthy_err() { assert_line $PROP 5 5 0 '0,0,0,0'; assert_line $STALL 0 0 0 '0,0,0,0'; assert_line $LOCKX 9 9 0 '0,0,0,0'; end_line 0; }
hit_err() { assert_line $PROP 6 5 1 '41,2,900,17' f;assert_line $STALL 0 0 0 '0,0,0,0'; end_line 0; }
stall_err() { assert_line $PROP 11 11 0 '0,0,0,0'; assert_line $STALL 1 1 0 "41,0,900,$1"; end_line 0; }

fixture() { # fixture <name> <exit> <stdout> ; stderr on stdin
  echo "$2" >"$tmp/$1.code"
  if [ "$3" = "-" ]; then : >"$tmp/$1.out"; else printf '%s\n' "$3" >"$tmp/$1.out"; fi
  cat >"$tmp/$1.err"
}

expect() { # expect <name> <regexp over the classify line>
  checks=$((checks + 1))
  line=$(sh "$classify" "$(cat "$tmp/$1.code")" "$tmp/$1.out" "$tmp/$1.err" 2>&1)
  if printf '%s\n' "$line" | grep -Eq -- "$2"; then echo "ok   $1"; else echo "FAIL $1: got '$line', want /$2/"; failed=$((failed + 1)); fi
}

healthy_err | fixture nohit 0 "$GOOD_OUT"
expect nohit '^class=NOHIT( |$)'

hit_err | fixture hit 3 "$GOOD_OUT"
expect hit '^class=HIT .*subtype=Q\+attributed.* prop_false=1'

stall_err 1 | fixture stall_q 4 "$GOOD_OUT"
expect stall_q '^class=STALL .*subtype=Q( |$)'
stall_err 2 | fixture stall_l 4 "$GOOD_OUT"
expect stall_l '^class=STALL .*subtype=L( |$)'
stall_err 4 | fixture stall_b 4 "$GOOD_OUT"
expect stall_b '^class=STALL .*subtype=B( |$)'
stall_err 8 | fixture stall_m 4 "$GOOD_OUT"
expect stall_m '^class=STALL .*subtype=M( |$)'

healthy_err | fixture timeout_124 124 -
expect timeout_124 '^class=TIMEOUT( |$)'
healthy_err | fixture timeout_137 137 -
expect timeout_137 '^class=TIMEOUT( |$)'
hit_err | fixture timeout_with_hit 124 -
expect timeout_with_hit '^class=TIMEOUT .*hit_also=1'

healthy_err | fixture hermit_125 125 -
expect hermit_125 '^class=HERMIT-FAIL( |$)'
{ healthy_err; echo "thread 'main' panicked at src/x.rs:1"; } | fixture hermit_panic 0 "$GOOD_OUT"
expect hermit_panic '^class=HERMIT-FAIL( |$)'
{ echo "HERMIT_INTERNAL_FAILURE: x"; } | fixture hermit_internal 2 -
expect hermit_internal '^class=HERMIT-FAIL( |$)'

echo 'Deadlock detected: thread(s) waiting on futex, but no runnable threads left.' | fixture deadlock 1 -
expect deadlock '^class=DEADLOCK-REPORT( |$)'
echo 'Deadlock detected: x' | fixture deadlock_exit0 0 "$GOOD_OUT"
expect deadlock_exit0 '^class=DEADLOCK-REPORT( |$)'
{ hit_err; echo 'Deadlock detected: x'; } | fixture deadlock_with_hit 1 -
expect deadlock_with_hit '^class=DEADLOCK-REPORT .*hit_also=1'
{ echo 'Deadlock detected: x'; } | fixture deadlock_beats_crash 2 -
expect deadlock_beats_crash '^class=DEADLOCK-REPORT( |$)'

echo 'segfault' | fixture crash_exit2 2 -
expect crash_exit2 '^class=CRASH( |$)'
healthy_err | fixture crash_signal 139 -
expect crash_signal '^class=CRASH( |$)'
echo 'something' | fixture crash_exit1_no_line 1 -
expect crash_exit1_no_line '^class=CRASH( |$)'

healthy_err | fixture harness_exit5 5 -
expect harness_exit5 '^class=HARNESS-FAIL( |$)'
echo 'condlock: usage: x' | fixture harness_exit64 64 -
expect harness_exit64 '^class=HARNESS-FAIL( |$)'
{ assert_line $PROP 5 5 0 '0,0,0,0'; assert_line $LOCKX 9 8 1 '1,0,0,0'; end_line 0; } | fixture harness_site_false 0 "$GOOD_OUT"
expect harness_site_false '^class=HARNESS-FAIL( |$)'
healthy_err | fixture harness_stdout_missing 0 -
expect harness_stdout_missing '^class=HARNESS-FAIL( |$)'
healthy_err | fixture harness_stdout_malformed 0 'hello'
expect harness_stdout_malformed '^class=HARNESS-FAIL( |$)'
{ assert_line $PROP 5 5 0 '0,0,0,0'; } | fixture harness_report_missing 0 "$GOOD_OUT"
expect harness_report_missing '^class=HARNESS-FAIL( |$)'
{ assert_line $PROP 5 5 0 '0,0,0,0'; end_line 3; } | fixture harness_dropped 0 "$GOOD_OUT"
expect harness_dropped '^class=HARNESS-FAIL( |$)'
hit_err | fixture harness_exit0_with_property_false 0 "$GOOD_OUT"
expect harness_exit0_with_property_false '^class=HARNESS-FAIL( |$)'
healthy_err | fixture harness_exit3_property_true 3 "$GOOD_OUT"
expect harness_exit3_property_true '^class=HARNESS-FAIL( |$)'
healthy_err | fixture harness_exit4_no_stall_hit 4 "$GOOD_OUT"
expect harness_exit4_no_stall_hit '^class=HARNESS-FAIL( |$)'
{ hit_err; } | fixture harness_with_hit 5 -
expect harness_with_hit '^class=HARNESS-FAIL .*hit_also=1'

# violation-only builds: no SDK events, the violation line decides
{ echo "HAIRSPRING_VIOLATION: $PROP"; echo 'condlock: HIT subtype=Q+attributed s_seq=1'; } | fixture vio_hit 3 "$GOOD_OUT"
expect vio_hit '^class=HIT .*subtype=Q\+attributed'
{ echo "HAIRSPRING_VIOLATION: $STALL"; echo 'condlock: STALL subtype=L s_seq=1'; } | fixture vio_stall 4 "$GOOD_OUT"
expect vio_stall '^class=STALL .*subtype=L'
: | fixture vio_nohit 0 "$GOOD_OUT"
expect vio_nohit '^class=NOHIT( |$)'
{ echo "HAIRSPRING_VIOLATION: $LOCKX"; } | fixture vio_harness 0 "$GOOD_OUT"
expect vio_harness '^class=HARNESS-FAIL( |$)'

# the applied flag of planted builds is carried through
healthy_err | fixture applied1 0 "$GOOD_OUT applied=1"
expect applied1 '^class=NOHIT .*applied=1'
healthy_err | fixture applied0 0 "$GOOD_OUT applied=0"
expect applied0 '^class=NOHIT .*applied=0'

# ---- real runs -----------------------------------------------------------------------------------------------------
real() { # real <name> <binary> <args...>
  name=$1; bin=$2; shift 2
  code=0
  "$bin" "$@" >"$tmp/$name.out" 2>"$tmp/$name.err" || code=$?
  echo "$code" >"$tmp/$name.code"
}
if [ -n "$build" ]; then
  real real_healthy "$build/condlock" -n 4 -r 50 -tick-ms 20 -q 10
  expect real_healthy '^class=NOHIT( |$)'
  real real_drop "$build/planted/condlock-drop-k1" -n 4 -r 50 -tick-ms 20 -q 10
  expect real_drop '^class=HIT .*applied=1'
  real real_leak "$build/planted/condlock-leak-k1" -n 4 -r 50 -tick-ms 20 -q 10
  expect real_leak '^class=STALL .*subtype=L .*applied=1'
  real real_barrier "$build/planted/condlock-barrier-k1" -n 4 -r 50 -tick-ms 20 -q 10
  expect real_barrier '^class=STALL .*subtype=B .*applied=1'
  real real_novio "$build/planted/condlock-novio-drop-k1" -n 4 -r 50 -tick-ms 20 -q 10
  expect real_novio '^class=HIT .*applied=1'
fi

echo "$checks checks, $failed failed"
[ "$failed" = 0 ]
