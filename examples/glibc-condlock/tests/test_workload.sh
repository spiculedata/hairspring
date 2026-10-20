#!/bin/sh
# Behaviour tests of the condlock binaries (native, informational): exit codes, the stdout line, the catalog, and one
# smoke run of every planted build. Usage: test_workload.sh <build dir>
set -u

build=${1:?usage: test_workload.sh <build dir>}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/condlock-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
checks=0
failed=0

check() { # check <name> <command...>
  name=$1; shift
  checks=$((checks + 1))
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; failed=$((failed + 1)); fi
}

# run <binary> <args...>: stdout -> $tmp/out, stderr -> $tmp/err, status -> $tmp/code
run() {
  bin=$1; shift
  code=0
  "$bin" "$@" >"$tmp/out" 2>"$tmp/err" || code=$?
  echo "$code" >"$tmp/code"
}
code() { cat "$tmp/code"; }
code_is() { [ "$(code)" = "$1" ]; }
out_has() { grep -q -- "$1" "$tmp/out"; }
err_has() { grep -q -- "$1" "$tmp/err"; }
err_lacks() { ! grep -q -- "$1" "$tmp/err"; }
out_lines_is() { [ "$(wc -l <"$tmp/out" | tr -d ' ')" = "$1" ]; }
assert_line() { # assert_line <id>: the SDK assert line of <id>
  grep '"kind":"assert"' "$tmp/err" | grep "\"id\":\"$1\""
}
assert_field_is() { # assert_field_is <id> <field> <value>
  assert_line "$1" | grep -Eq "\"$2\": *$3[,}]"
}
assert_field_positive() { # assert_field_positive <id> <field>
  assert_line "$1" | grep -Eq "\"$2\": *[1-9][0-9]*[,}]"
}

ids="glibc.cond.signal_wakes_one_of_blocked condlock.stall_unexplained condlock.contended_signal
condlock.wait_spanned_two_signals condlock.watchdog_tick condlock.stagnation_seen condlock.harness.lock_exclusive
condlock.harness.pthread_ok condlock.harness.all_rounds_ran"
catalog_ok() { for id in $ids; do grep '"kind":"catalog"' "$tmp/err" | grep -q "\"id\":\"$id\"" || return 1; done; }
catalog_count_is_nine() { [ "$(grep -c '"kind":"catalog"' "$tmp/err" | tr -d ' ')" = "9" ]; }

c=$build/condlock

# --- healthy run -------------------------------------------------------------------------------------------------
run "$c" -n 4 -r 50 -tick-ms 20 -q 10
check "healthy: exit 0" code_is 0
check "healthy: exactly one stdout line" out_lines_is 1
check "healthy: stdout line starts with the fixed prefix" out_has '^condlock libc=none hdr=none rounds='
check "healthy: rounds done = R*N" out_has 'rounds=     200 '
check "healthy: no planted-build field" sh -c "! grep -q applied '$tmp/out'"
check "healthy: nine catalog lines" catalog_count_is_nine
check "healthy: every catalogued id is the documented one" catalog_ok
check "healthy: SDK end line present" err_has '"kind":"end"'
check "healthy: no state dump" err_lacks '^condlock: '
check "healthy: property site never false" assert_field_is glibc.cond.signal_wakes_one_of_blocked falses 0
check "healthy: all rounds ran site true" assert_field_positive condlock.harness.all_rounds_ran trues
check "healthy: pthread_ok site evaluated and never false" assert_field_is condlock.harness.pthread_ok falses 0
check "healthy: lock_exclusive never false" assert_field_is condlock.harness.lock_exclusive falses 0
check "healthy: stall site never hit" assert_field_is condlock.stall_unexplained hits 0

# --- usage -------------------------------------------------------------------------------------------------------
run "$c" -bogus 1
check "usage: bad flag exits 64" code_is 64
check "usage: reason on stderr" err_has 'condlock: usage'
run "$c" -n 0
check "usage: n = 0 exits 64" code_is 64

# --- one thread config the Mac self-tests do not use ----------------------------------------------------------------
run "$c" -n 1 -r 5 -tick-ms 20 -q 10
check "single worker: exit 0" code_is 0

# --- variants ------------------------------------------------------------------------------------------------------
run "$build/condlock-novio" -n 4 -r 30 -tick-ms 20 -q 10
check "violation-only build: healthy exit 0" code_is 0
check "violation-only build: no violation line" err_lacks 'HAIRSPRING_VIOLATION'
check "violation-only build: no SDK events" err_lacks 'HAIRSPRING_EVENT'
run "$build/condlock-nowd" -n 4 -r 30 -tick-ms 20 -q 10
check "no-watchdog build: healthy exit 0" code_is 0
check "no-watchdog build: no watchdog tick" assert_field_is condlock.watchdog_tick hits 0

# --- planted builds: one smoke run each (the full counts are the self-tests) ----------------------------------------
run "$build/planted/condlock-drop-k1" -n 4 -r 50 -tick-ms 20 -q 10
check "drop k=1: exit 3 (property fired)" code_is 3
check "drop k=1: applied flag printed as 1" out_has ' applied=1$'
check "drop k=1: property site false" assert_field_positive glibc.cond.signal_wakes_one_of_blocked falses
check "drop k=1: a HIT dump line on stderr" err_has '^condlock: HIT '
check "drop k=1: stall site not hit" assert_field_is condlock.stall_unexplained hits 0

run "$build/planted/condlock-leak-k1" -n 4 -r 50 -tick-ms 20 -q 10
check "leak k=1: exit 4 (STALL)" code_is 4
check "leak k=1: subtype L" err_has '^condlock: STALL subtype=L '
check "leak k=1: property site true" assert_field_is glibc.cond.signal_wakes_one_of_blocked falses 0
check "leak k=1: stall site hit once" assert_field_is condlock.stall_unexplained hits 1
check "leak k=1: applied=1" out_has ' applied=1$'

run "$build/planted/condlock-barrier-k1" -n 4 -r 50 -tick-ms 20 -q 10
check "barrier k=1: exit 4 (STALL)" code_is 4
check "barrier k=1: property site true" assert_field_is glibc.cond.signal_wakes_one_of_blocked falses 0
check "barrier k=1: applied=1" out_has ' applied=1$'

run "$build/planted/condlock-holdg-k1" -n 4 -r 50 -tick-ms 20 -q 10
check "holdg k=1: exit 4 (STALL)" code_is 4
check "holdg k=1: subtype M (G could not be taken for a whole tick)" err_has '^condlock: STALL subtype=M '
check "holdg k=1: property site true" assert_field_is glibc.cond.signal_wakes_one_of_blocked falses 0
check "holdg k=1: stall site hit once" assert_field_is condlock.stall_unexplained hits 1
check "holdg k=1: applied=1" out_has ' applied=1$'

run "$build/planted/condlock-novio-drop-k1" -n 4 -r 50 -tick-ms 20 -q 10
check "violation-only drop k=1: exit 3" code_is 3
check "violation-only drop k=1: violation line names the property" err_has 'HAIRSPRING_VIOLATION: glibc.cond.signal_wakes_one_of_blocked'

# a planted k beyond what R produces is not applied, and the run is healthy (applied=0)
run "$build/planted/condlock-drop-k50" -n 2 -r 2 -tick-ms 20 -q 10
check "drop k=50 with tiny R: exit 0, applied=0" sh -c "[ \"\$(cat '$tmp/code')\" = 0 ] && grep -q ' applied=0\$' '$tmp/out'"

echo "$checks checks, $failed failed"
[ "$failed" = 0 ]
