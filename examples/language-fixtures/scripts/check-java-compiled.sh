#!/bin/sh
# Run-time evidence that the JIT never compiled the fixture's two worker loops.
#
#   check-java-compiled.sh <profiles.tsv> <lang-at.jar> [max-k]
#
# The window of the lost-update fixture is the three steps in langat.Race.racyIterations (and lockedIterations): a load of
# the plain counter, a call of spin and a store. A JIT could remove the window by keeping the counter in a register across
# a compiled loop. HotSpot compiles a method (or, by on-stack replacement, a loop inside it) when its invocation and loop
# back-edge counters pass a threshold, and those counters are kept per method and summed over all threads: each worker
# method is called 4 times and its loop passes 4 x 1,000 = 4,000 times in all, well below the thresholds (about 60,000
# back-edges for C1 at tier 3 and about 10,700 with -XX:-TieredCompilation), however many threads run it.
#
# This script does not rely on that arithmetic. For the default flags and for each profile of profiles.tsv it runs buggy,
# fixed and wide under -XX:+PrintCompilation and fails if the output names langat.Race::racyIterations or
# langat.Race::lockedIterations (a normal compile or an OSR compile, which the log marks with %), or any other method of
# langat.Race except spin. Positive controls, so a
# log the check could not read is not taken for a pass: under every run that is not interpreter-only the log must hold at
# least one compile line, and the wide run must show langat.Race::spin compiled (its loop is the hot one).
# With max-k (the fixture's largest --k=N, read from Contract.java by the Makefile and the build record) every arm is run
# a second time with --k=<max-k> appended to each mode: a larger K raises the back-edge count of the worker loops, so the
# guarantee has to hold at the largest K the program accepts, not only at the default.
# JAVA is the java command (default java). Nothing here runs Hermit.
# Exits 0 when every run was clean, 1 when one was not or there is no profile, 64 for a usage error.

set -u

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <profiles.tsv> <lang-at.jar> [max-k]" >&2
  exit 64
fi
profiles=$1
jar=$2
maxk=${3:-}
case "$maxk" in
  *[!0-9]*) echo "usage: max-k must be a number, got \"$maxk\"" >&2; exit 64 ;;
esac
java=${JAVA:-java}
if [ ! -r "$profiles" ]; then
  echo "FAIL $profiles cannot be read" >&2
  exit 1
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/java-compiled.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
tab=$(printf '\t')
status=0
count=0
# Any compiled method of Race but spin: the two worker loops, and the rest of the class (a lambda, execute, joinAll) too.
OTHER_RACE='langat\.Race::([^s]|s[^p]|sp[^i]|spi[^n]|spin[^ ])'
COMPILE_LINE='^ *[0-9]+ +[0-9]+ '

# check_arm <arm> <flags> [k-argument]: the three modes under the flags (and the argument), each with the log in $tmp/log.
check_arm() {
  arm=$1
  flags=$2
  karg=${3:-}
  echo "arm $arm: ${flags:-(default flags)} ${karg:+($karg)}"
  interpreted=0
  case " $flags " in *' -Xint '*) interpreted=1 ;; esac
  for mode in buggy fixed wide; do
    # shellcheck disable=SC2086  # flags is a list of words on purpose
    "$java" $flags -XX:+PrintCompilation -cp "$jar" langat.Main "$mode" $karg >"$tmp/log" 2>"$tmp/err" </dev/null
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL arm $arm, $mode: exit status $rc"; cat "$tmp/err"; return 1; }
    lines=$(grep -Ec "$COMPILE_LINE" "$tmp/log")
    if grep -E "$OTHER_RACE" "$tmp/log"; then
      echo "FAIL arm $arm, $mode: a method of Race other than spin was compiled (the lines above)"
      return 1
    fi
    if [ "$interpreted" -eq 0 ]; then
      [ "$lines" -gt 0 ] || { echo "FAIL arm $arm, $mode: PrintCompilation printed no compile line, so this log proves nothing"; return 1; }
      if [ "$mode" = wide ] && ! grep -Eq 'langat\.Race::spin' "$tmp/log"; then
        echo "FAIL arm $arm, wide: spin was not compiled, so the log is not showing this program's compiles"
        return 1
      fi
    fi
    echo "ok   arm $arm, $mode ${karg:-(default K)}: $lines compile lines, no Race method but spin among them"
  done
}

check_arm default "" || status=1
[ -z "$maxk" ] || check_arm default "" "--k=$maxk" || status=1
count=1
while IFS="$tab" read -r arm flags; do
  case "$arm" in '' | '#'*) continue ;; esac
  count=$((count + 1))
  check_arm "$arm" "$flags" || status=1
  [ -z "$maxk" ] || check_arm "$arm" "$flags" "--k=$maxk" || status=1
done <"$profiles"
if [ "$count" -lt 2 ]; then
  echo "FAIL $profiles holds no profile" >&2
  status=1
fi
exit $status
