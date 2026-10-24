#!/bin/sh
# Flag validity of the Java probe's JVM profiles, and a clean run under each.
#
#   check-java-profiles.sh <profiles.tsv> <lang-at.jar>
#
# profiles.tsv has one profile per line: the arm name, a tab, the flags (lines starting with # are comments). For each profile
# this runs `java <flags> -version` (the JVM must accept every flag and print no warning: a JVM warning goes to stdout by
# default, which would break the one-line contract), then the probe twice under the flags: exit 0, nothing on stderr and
# exactly one stdout line in the six-field form. It prints each arm with its flags, the `-version` output and the lines.
# JAVA is the java command (default java). Nothing here runs Hermit.
# Exits 0 when every profile passed, 1 when one failed or there is none, 64 for a usage error.

set -u

if [ $# -ne 2 ]; then
  echo "usage: $0 <profiles.tsv> <lang-at.jar>" >&2
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
line_rx='^PROBE v1 lang=java lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'
status=0
count=0

check_arm() { # check_arm <arm> <flags>
  arm=$1
  flags=$2
  echo "arm $arm: $flags"
  # shellcheck disable=SC2086  # flags is a list of words on purpose
  "$java" $flags -version >"$tmp/version.out" 2>&1
  rc=$?
  cat "$tmp/version.out"
  [ "$rc" -eq 0 ] || { echo "FAIL arm $arm: the JVM refused the flags (exit status $rc)"; return 1; }
  if grep -qi 'warning' "$tmp/version.out"; then
    echo "FAIL arm $arm: the JVM printed a warning"
    return 1
  fi
  for run in 1 2; do
    # shellcheck disable=SC2086
    "$java" $flags -cp "$jar" langat.Probe >"$tmp/out.txt" 2>"$tmp/err.txt"
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL arm $arm run $run: exit status $rc, want 0"; return 1; }
    [ ! -s "$tmp/err.txt" ] || { echo "FAIL arm $arm run $run: something on stderr:"; cat "$tmp/err.txt"; return 1; }
    [ "$(wc -l <"$tmp/out.txt" | tr -d ' ')" -eq 1 ] || { echo "FAIL arm $arm run $run: stdout is not exactly one line:"; cat "$tmp/out.txt"; return 1; }
    grep -Eq "$line_rx" "$tmp/out.txt" || { echo "FAIL arm $arm run $run: not the six-field line:"; cat "$tmp/out.txt"; return 1; }
    cat "$tmp/out.txt"
  done
  echo "ok   arm $arm: the flags are valid and a run prints one clean line"
}

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
