#!/bin/sh
# classify.sh <exit status> <stdout file> <stderr file>
#
# Classifies one run of condlock from its own outputs, never from an engine verdict. Prints one line:
#
#   class=<C> subtype=<S|-> prop_false=<0|1> stall_hit=<0|1> applied=<0|1|-> hit_also=<0|1> detail=<text>
#
# Precedence (first match wins): TIMEOUT, HERMIT-FAIL, DEADLOCK-REPORT, CRASH, HARNESS-FAIL, STALL, HIT, NOHIT.
# A run that had a property hit but fell in an earlier class carries hit_also=1 (the hit is detail, not the class).
# The workload never exits 1, so exit 1 with Hermit's "Deadlock detected" line is Hermit's own report (DEADLOCK-REPORT);
# the same line anywhere is that class too. Both SDK builds (assert lines) and violation-only builds (HAIRSPRING_VIOLATION
# lines) are understood.
set -u

code=${1:?usage: classify.sh <exit status> <stdout file> <stderr file>}
out=${2:?}
err=${3:?}

PROP=glibc.cond.signal_wakes_one_of_blocked
STALL=condlock.stall_unexplained

site_line() { grep '"kind":"assert"' "$err" | grep "\"id\":\"$1\"" | head -n 1; }
site_num() { # site_num <id> <field>: the number, 0 if the site has no line
  v=$(site_line "$1" | grep -Eo "\"$2\": *[0-9]+" | grep -Eo '[0-9]+$')
  echo "${v:-0}"
}
violation_for() { grep -q "^HAIRSPRING_VIOLATION: $1\$" "$err"; }

# witness_vals <id> <first_true|first_false>: the four values of that witness, comma separated, or empty
witness_vals() {
  site_line "$1" | grep -Eo "\"$2\":\{[^}]*\}" | grep -Eo '\[[-0-9, ]*\]' | tr -d '[] '
}

subtype_name() { # subtype_name <bits>
  n=$1
  attr=""
  if [ $((n & 16)) -ne 0 ]; then attr="+attributed"; fi
  case $((n & 15)) in
    1) echo "Q$attr" ;;
    2) echo "L$attr" ;;
    4) echo "B$attr" ;;
    8) echo "M$attr" ;;
    *) echo "NONE$attr" ;;
  esac
}

dump_subtype() { # the human dump line is the fallback when no SDK witness exists
  grep -Eo "^condlock: $1 subtype=[A-Za-z+]+" "$err" | head -n 1 | sed 's/.*subtype=//'
}

has_sdk=0
if grep -q '^HAIRSPRING_EVENT ' "$err"; then has_sdk=1; fi

prop_false=0
if [ "$(site_num $PROP falses)" -gt 0 ] || violation_for $PROP; then prop_false=1; fi
stall_hit=0
if [ "$(site_num $STALL hits)" -gt 0 ] || violation_for $STALL; then stall_hit=1; fi

harness_false=0
# shellcheck disable=SC2013 # ids use the protocol alphabet [A-Za-z0-9._:/-]: no whitespace to split on
for id in $(grep '"kind":"assert"' "$err" | grep -Eo '"id":"condlock\.harness\.[A-Za-z0-9._:/-]*"' | cut -d'"' -f4); do
  if [ "$(site_num "$id" falses)" -gt 0 ]; then harness_false=1; fi
done
if grep -q '^HAIRSPRING_VIOLATION: condlock\.harness\.' "$err"; then harness_false=1; fi

applied="-"
a=$(grep -Eo ' applied=[01]$' "$out" | head -n 1 | grep -Eo '[01]$')
if [ -n "$a" ]; then applied=$a; fi

subtype="-"
if [ "$stall_hit" = 1 ]; then
  bits=$(witness_vals $STALL first_true | awk -F, '{ print $4 }')
  if [ -n "$bits" ]; then subtype=$(subtype_name "$bits"); else subtype=$(dump_subtype STALL); fi
elif [ "$prop_false" = 1 ]; then
  bits=$(witness_vals $PROP first_false | awk -F, '{ print $4 }')
  if [ -n "$bits" ]; then subtype=$(subtype_name "$bits"); else subtype=$(dump_subtype HIT); fi
fi
[ -n "$subtype" ] || subtype="-"

stdout_ok=0
if [ "$(wc -l <"$out" | tr -d ' ')" = 1 ] && grep -q '^condlock libc=' "$out"; then stdout_ok=1; fi

report_ok=1 # an SDK build must have an `end` line and no dropped data
if [ "$has_sdk" = 1 ]; then
  grep -q '"kind":"end"' "$err" || report_ok=0
  d=$(grep '"kind":"end"' "$err" | head -n 1 | grep -Eo '"dropped": *[0-9]+' | grep -Eo '[0-9]+$')
  if [ "${d:-0}" -gt 0 ]; then report_ok=0; fi
fi

deadlock=0
if grep -q 'Deadlock detected' "$err" "$out"; then deadlock=1; fi

emit() { # emit <class> <detail>
  hit_also=0
  if [ "$prop_false" = 1 ] && [ "$1" != HIT ]; then hit_also=1; fi
  echo "class=$1 subtype=$subtype prop_false=$prop_false stall_hit=$stall_hit applied=$applied hit_also=$hit_also detail=$2"
  exit 0
}

case $code in 124 | 137) emit TIMEOUT "exit $code" ;; esac
case $code in 125 | 101) emit HERMIT-FAIL "exit $code" ;; esac
if grep -Eq 'panicked at|HERMIT_INTERNAL_FAILURE|SKID_OVERSHOOT|Usage: hermit|HERMIT_CLEANUP_UNCONFIRMED' "$err"; then
  emit HERMIT-FAIL "hermit message on stderr"
fi
if [ "$deadlock" = 1 ]; then emit DEADLOCK-REPORT "exit $code, Deadlock detected"; fi
case $code in 0 | 3 | 4 | 5 | 64) ;; *) emit CRASH "exit $code" ;; esac

case $code in 5 | 64) emit HARNESS-FAIL "exit $code" ;; esac
if [ "$harness_false" = 1 ]; then emit HARNESS-FAIL "a condlock.harness site was false"; fi
if [ "$stdout_ok" != 1 ]; then emit HARNESS-FAIL "stdout is not the single report line"; fi
if [ "$report_ok" != 1 ]; then emit HARNESS-FAIL "incomplete SDK report (no end line, or dropped events)"; fi
if [ "$code" = 0 ] && { [ "$prop_false" = 1 ] || [ "$stall_hit" = 1 ]; }; then
  emit HARNESS-FAIL "exit 0 although a detector site fired"
fi
if [ "$code" = 3 ] && [ "$prop_false" != 1 ]; then emit HARNESS-FAIL "exit 3 without the property site false"; fi
if [ "$code" = 4 ] && [ "$stall_hit" != 1 ]; then emit HARNESS-FAIL "exit 4 without the stall site hit"; fi

if [ "$code" = 4 ]; then emit STALL "exit 4"; fi
if [ "$code" = 3 ]; then emit HIT "exit 3"; fi
emit NOHIT "exit 0"
