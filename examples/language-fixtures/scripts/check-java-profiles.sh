#!/bin/sh
# Flag validity of the Java probe's JVM profiles, and a clean run under each.
#
#   check-java-profiles.sh [--exact] <profiles.tsv> <lang-at.jar>
#
# profiles.tsv has one profile per line: the arm name, a tab, the flags (lines starting with # are comments). For each profile
# this runs `java <flags> -version` (the JVM must accept every flag and print no warning: a JVM warning goes to stdout by
# default, which would break the one-line contract), then the probe twice under the flags: exit 0, nothing on stderr and a
# stdout that is exactly the one six-field line (check-probe-line.sh: no junk after it). It prints each arm with its flags,
# the `-version` output and the lines. `--exact` also requires the file to hold exactly arms a and b with the experiment's
# flags and nothing else, compared with the text below. The JVM is given no stdin.
# JAVA is the java command (default java). Nothing here runs Hermit.
# Exits 0 when every profile passed, 1 when one failed or there is none, 64 for a usage error.

set -u

here=$(cd "$(dirname "$0")" && pwd)
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
if [ ! -r "$profiles" ]; then
  echo "FAIL $profiles cannot be read" >&2
  exit 1
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/java-profiles.XXXXXX") || exit 1
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

check_arm() { # check_arm <arm> <flags>
  arm=$1
  flags=$2
  echo "arm $arm: $flags"
  # shellcheck disable=SC2086  # flags is a list of words on purpose
  "$java" $flags -version >"$tmp/version.out" 2>&1 </dev/null
  rc=$?
  cat "$tmp/version.out"
  [ "$rc" -eq 0 ] || { echo "FAIL arm $arm: the JVM refused the flags (exit status $rc)"; return 1; }
  if grep -qi 'warning' "$tmp/version.out"; then
    echo "FAIL arm $arm: the JVM printed a warning"
    return 1
  fi
  for run in 1 2; do
    # shellcheck disable=SC2086
    "$java" $flags -cp "$jar" langat.Probe >"$tmp/out.txt" 2>"$tmp/err.txt" </dev/null
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL arm $arm run $run: exit status $rc, want 0"; return 1; }
    [ ! -s "$tmp/err.txt" ] || { echo "FAIL arm $arm run $run: something on stderr:"; cat "$tmp/err.txt"; return 1; }
    sh "$here/check-probe-line.sh" "$tmp/out.txt" || { echo "FAIL arm $arm run $run: stdout is not exactly the probe line"; return 1; }
    cat "$tmp/out.txt"
  done
  echo "ok   arm $arm: the flags are valid and a run prints one clean line"
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
