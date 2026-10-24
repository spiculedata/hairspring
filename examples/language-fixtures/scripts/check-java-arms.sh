#!/bin/sh
# Flag validity of the Java fixture's JVM profiles, and a clean run under each.
#
#   check-java-arms.sh [--exact] <profiles.tsv> <lang-at.jar>
#
# profiles.tsv has one profile per line: the arm name, a tab, the flags (lines starting with # are comments). For each
# profile this runs `java <flags> -version` (the JVM must accept every flag and print no warning: a JVM warning goes to
# stdout by default, which would break the one-line contract), then `langat.Main fixed` under the flags: exit 0, stdout
# exactly one result line with final == 4000 (the locked mode never loses an update), and nothing on stderr but protocol
# lines. Then ARM_RUNS runs of `buggy` (default 5): each exits 0 with one clean result line, and how many of them lost an
# update is printed as information only (the native self-check is what requires the race to fire; a profile with one
# processor may legitimately hide it natively). `--exact` also requires the file to hold exactly arms a and b with the
# experiment's flags and nothing else. The JVM is given no stdin.
# JAVA is the java command (default java). Nothing here runs Hermit.
# Exits 0 when every profile passed, 1 when one failed or there is none, 64 for a usage error.

set -u

exact=0
if [ "${1:-}" = "--exact" ]; then
  exact=1
  shift
fi
if [ $# -ne 2 ]; then
  echo "usage: $0 [--exact] <profiles.tsv> <lang-at.jar>" >&2
  exit 64
fi
profiles=$1
jar=$2
java=${JAVA:-java}
arm_runs=${ARM_RUNS:-5}
EXPECTED=4000
if [ ! -r "$profiles" ]; then
  echo "FAIL $profiles cannot be read" >&2
  exit 1
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/java-arms.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
tab=$(printf '\t')
status=0
count=0

# The two profiles of the experiment, as profiles.tsv must hold them under --exact.
expected_profiles() {
  printf 'a\t-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off\n'
  printf 'b\t-XX:TieredStopAtLevel=1 -XX:-BackgroundCompilation -XX:CICompilerCount=1 -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off\n'
}

check_exact() {
  grep -v '^#' "$profiles" | grep -v '^$' >"$tmp/actual.tsv"
  expected_profiles >"$tmp/expected.tsv"
  if cmp -s "$tmp/actual.tsv" "$tmp/expected.tsv"; then
    echo "ok   $profiles is exactly arms a and b with the experiment's flags"
    return 0
  fi
  echo "FAIL $profiles is not exactly arms a and b with the experiment's flags:"
  diff "$tmp/expected.tsv" "$tmp/actual.tsv"
  return 1
}

# fixture_run <flags> <mode>: one run, stdout in $tmp/out, stderr in $tmp/err; the exit status is the result.
fixture_run() {
  # shellcheck disable=SC2086  # flags is a list of words on purpose
  env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE \
    "$java" $1 -cp "$jar" langat.Main "$2" >"$tmp/out" 2>"$tmp/err" </dev/null
}

# result_line_ok <mode>: stdout is exactly one result line of this mode, and stderr holds protocol lines only.
result_line_ok() {
  shape="^LANGAT v1 lang=java mode=$1 final=[0-9]{20} expect=0{16}$EXPECTED order=[0-9a-f]{16} rand=[0-9a-f]{16}$"
  [ "$(wc -l <"$tmp/out" | tr -d ' ')" -eq 1 ] && grep -Eq "$shape" "$tmp/out" || return 1
  [ -z "$(grep -v '^HAIRSPRING_EVENT' "$tmp/err")" ]
}

check_arm() { # check_arm <arm> <flags>
  arm=$1
  flags=$2
  echo "arm $arm: $flags"
  # shellcheck disable=SC2086
  "$java" $flags -version >"$tmp/version.out" 2>&1 </dev/null
  rc=$?
  cat "$tmp/version.out"
  [ "$rc" -eq 0 ] || { echo "FAIL arm $arm: the JVM refused the flags (exit status $rc)"; return 1; }
  if grep -qi 'warning' "$tmp/version.out"; then
    echo "FAIL arm $arm: the JVM printed a warning"
    return 1
  fi
  fixture_run "$flags" fixed
  rc=$?
  [ "$rc" -eq 0 ] || { echo "FAIL arm $arm: fixed exited $rc, want 0"; cat "$tmp/err"; return 1; }
  result_line_ok fixed || { echo "FAIL arm $arm: fixed did not print one clean result line:"; cat "$tmp/out" "$tmp/err"; return 1; }
  grep -q "final=0*$EXPECTED expect=" "$tmp/out" || { echo "FAIL arm $arm: the locked mode lost an update:"; cat "$tmp/out"; return 1; }
  cat "$tmp/out"
  fired=0
  i=0
  while [ "$i" -lt "$arm_runs" ]; do
    i=$((i + 1))
    fixture_run "$flags" buggy
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL arm $arm: buggy exited $rc, want 0"; cat "$tmp/err"; return 1; }
    result_line_ok buggy || { echo "FAIL arm $arm: buggy did not print one clean result line:"; cat "$tmp/out" "$tmp/err"; return 1; }
    grep -q "final=0*$EXPECTED expect=" "$tmp/out" || fired=$((fired + 1))
  done
  echo "information: arm $arm, buggy lost an update in $fired of $arm_runs runs"
  echo "ok   arm $arm: the flags are valid, fixed is exact and the result lines are clean"
}

if [ "$exact" -eq 1 ]; then
  check_exact || status=1
fi
while IFS="$tab" read -r arm flags; do
  case "$arm" in '' | '#'*) continue ;; esac
  count=$((count + 1))
  check_arm "$arm" "$flags" || status=1
done <"$profiles"
if [ "$count" -eq 0 ]; then
  echo "FAIL $profiles holds no profile" >&2
  status=1
fi
exit $status
