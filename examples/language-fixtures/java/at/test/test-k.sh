#!/bin/sh
# Native tests of the fixture's --k=N argument (iterations per worker): the default run is unchanged, --k=50 shortens the
# run and keeps the line's format, and invalid values are usage errors. No Hermit, no engine.
#
#   test-k.sh <lang-at.jar>
#
# Environment: JAVA is the java command (default java). Exits 0 when every test passed, 1 when one failed, 64 for a usage
# error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <lang-at.jar>" >&2
  exit 64
fi
jar=$1
java=${JAVA:-java}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/at-k-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0

ok() { echo "ok   $1"; }
bad() {
  echo "FAIL $1"
  failures=$((failures + 1))
}

# fixture <args...>: one run with the SDK's report off; prints the exit status. stdout in $tmp/out, stderr in $tmp/err.
fixture() {
  HAIRSPRING_EVENTS=off "$java" -cp "$jar" langat.Main "$@" >"$tmp/out" 2>"$tmp/err"
  echo $?
}

# line_matches <mode> <final-regex> <expect-digits>: the one stdout line has the contract's format.
line_matches() {
  [ "$(wc -l <"$tmp/out" | tr -d ' ')" = 1 ] &&
    grep -Eq "^LANGAT v1 lang=java mode=$1 final=$2 expect=$3 order=[0-9a-f]{16} rand=[0-9a-f]{16}\$" "$tmp/out"
}

status=$(fixture fixed)
if [ "$status" = 0 ] && line_matches fixed 00000000000000004000 00000000000000004000; then
  ok "no --k: fixed ends at 4000 with expect=4000 (1,000 iterations per worker)"
else
  bad "no --k: fixed ends at 4000 with expect=4000 (status $status): $(cat "$tmp/out")"
fi

status=$(fixture buggy)
if [ "$status" = 0 ] && line_matches buggy '[0-9]{20}' 00000000000000004000 && [ ! -s "$tmp/err" ]; then
  ok "no --k: buggy keeps the contract line with expect=4000 and nothing on stderr"
else
  bad "no --k: buggy keeps the contract line (status $status): $(cat "$tmp/out") $(cat "$tmp/err")"
fi

status=$(fixture fixed --k=50)
if [ "$status" = 0 ] && line_matches fixed 00000000000000000200 00000000000000000200 && [ ! -s "$tmp/err" ]; then
  ok "--k=50: fixed ends at 200 (4 x 50) with the same line format"
else
  bad "--k=50: fixed ends at 200 (status $status): $(cat "$tmp/out") $(cat "$tmp/err")"
fi

status=$(fixture buggy --k=50)
final=$(sed -n 's/.* final=\([0-9]*\) .*/\1/p' "$tmp/out" | sed 's/^0*//')
if [ "$status" = 0 ] && line_matches buggy '[0-9]{20}' 00000000000000000200 && [ "${final:-0}" -le 200 ]; then
  ok "--k=50: buggy ends at no more than 200"
else
  bad "--k=50: buggy ends at no more than 200 (status $status): $(cat "$tmp/out")"
fi

status=$(fixture wide --k=3)
if [ "$status" = 0 ] && line_matches wide '[0-9]{20}' 00000000000000000012; then
  ok "--k=3: wide runs and expects 12"
else
  bad "--k=3: wide runs and expects 12 (status $status): $(cat "$tmp/out")"
fi

for bad_args in "buggy --k=" "buggy --k=abc" "buggy --k=0" "buggy --k=-1" "buggy --k=1.5" "buggy --k=050" "buggy --k=100001" \
  "buggy --k=99999999999999999999" "fixed --k=50 extra" "fixed --k" "fixed --switch=1.0" "--k=50 fixed" "--k=50"; do
  # shellcheck disable=SC2086  # the words are the arguments on purpose
  status=$(fixture $bad_args)
  if [ "$status" = 64 ] && [ ! -s "$tmp/out" ] && grep -q "usage" "$tmp/err"; then
    ok "refused with 64, nothing on stdout: $bad_args"
  else
    bad "refused with 64, nothing on stdout: $bad_args (status $status): $(cat "$tmp/out") $(cat "$tmp/err")"
  fi
done

fixture buggy --k=0 >/dev/null
if grep -q -- "--k" "$tmp/err"; then
  ok "an invalid --k names the option on stderr"
else
  bad "an invalid --k names the option on stderr: $(cat "$tmp/err")"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures failed"
  exit 1
fi
echo "all passed"
