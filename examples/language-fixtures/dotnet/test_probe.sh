#!/bin/sh
# Native tests of the built probe (LangAtProbe.dll) and of its environment profiles.
#
#   test_probe.sh <dotnet> <LangAtProbe.dll> [runs]
#
# Checks, with the real program: every run exits 0 with an empty stderr and exactly one stdout line in the six-field form;
# any argument is a usage error (exit 64, a usage line on stderr, nothing on stdout); an unwritable stdout is an internal
# failure (exit 3, only where /dev/full exists); both environment profiles of profiles.manifest list exactly the variables
# the experiment names, and the program runs and prints a valid line under each of them. Nothing here runs Hermit.
# Exits 0 when everything holds, 1 when something does not, 64 for a usage error.

set -u

if [ $# -lt 2 ]; then
  echo "usage: $0 <dotnet> <LangAtProbe.dll> [runs]" >&2
  exit 64
fi
dotnet=$1
dll=$2
runs=${3:-5}
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/probe-dotnet-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0
line_rx='^PROBE v1 lang=dotnet lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'
# The runtime directory: the one that holds the dotnet executable (resolved through links).
root=$(dirname "$(readlink -f "$dotnet" 2>/dev/null || echo "$dotnet")")

ok() { echo "ok   $1"; }
bad() {
  echo "FAIL $1"
  failures=$((failures + 1))
}

# run_probe <NAME=value>... [-- <program argument>...]: the probe in an otherwise empty environment (PATH only), so the
# variables named are the only ones the runtime sees, as under the experiment's profiles.
run_probe() {
  words=""
  while [ $# -gt 0 ] && [ "$1" != "--" ]; do
    words="$words $1"
    shift
  done
  [ $# -eq 0 ] || shift
  # The variable words are separate arguments to env on purpose.
  # shellcheck disable=SC2086
  env -i PATH="$PATH" $words "$dotnet" "$dll" "$@"
}

# expect <name> <wanted exit> <command...>
expect() {
  name=$1
  wanted=$2
  shift 2
  "$@" >/dev/null 2>&1
  got=$?
  if [ "$got" -eq "$wanted" ]; then ok "$name"; else bad "$name: exit $got, want $wanted"; fi
}

# check_run <label> <env words...>: one run under the given environment must be a valid, quiet, one-line run.
check_run() {
  label=$1
  shift
  run_probe "$@" >"$tmp/out.txt" 2>"$tmp/err.txt"
  rc=$?
  [ "$rc" -eq 0 ] || { bad "$label: exit status $rc, want 0"; return 1; }
  [ ! -s "$tmp/err.txt" ] || { bad "$label: something on stderr: $(cat "$tmp/err.txt")"; return 1; }
  [ "$(wc -l <"$tmp/out.txt" | tr -d ' ')" -eq 1 ] || { bad "$label: stdout is not exactly one line: $(cat "$tmp/out.txt")"; return 1; }
  grep -Eq "$line_rx" "$tmp/out.txt" || { bad "$label: the line is not in the six-field form: $(cat "$tmp/out.txt")"; return 1; }
  cat "$tmp/out.txt" >>"$tmp/lines.txt"
  return 0
}

: >"$tmp/lines.txt"
i=0
good=0
while [ "$i" -lt "$runs" ]; do
  check_run "run $i" "DOTNET_ROOT=$root" && good=$((good + 1))
  i=$((i + 1))
done
[ "$good" -eq "$runs" ] && ok "$runs runs: exit 0, empty stderr, one line in the six-field form"

for name in lock_order arrival rand extra; do
  awk -v name="$name" '{ for (k = 1; k <= NF; k++) if (index($k, name "=") == 1 && !($k in seen)) { seen[$k] = 1; n++ } } END { print "information: " n " distinct " name " of " NR " runs" }' "$tmp/lines.txt"
done

for arg in x --switch=1.0 ""; do
  run_probe "DOTNET_ROOT=$root" -- "$arg" >"$tmp/out.txt" 2>"$tmp/err.txt"
  rc=$?
  if [ "$rc" -eq 64 ] && [ ! -s "$tmp/out.txt" ] && [ -s "$tmp/err.txt" ]; then
    ok "the argument '$arg' is a usage error: exit 64, a usage line on stderr, nothing on stdout"
  else
    bad "the argument '$arg' is not a usage error (exit $rc, stdout '$(cat "$tmp/out.txt")')"
  fi
done

if [ -w /dev/full ]; then
  run_probe "DOTNET_ROOT=$root" >/dev/full 2>"$tmp/err.txt"
  rc=$?
  if [ "$rc" -eq 3 ] && [ -s "$tmp/err.txt" ]; then
    ok "an unwritable stdout is an internal failure: exit 3 and a message on stderr"
  else
    bad "an unwritable stdout gave exit $rc, want 3 with a message on stderr"
  fi
else
  echo "skip an unwritable stdout (no /dev/full here)"
fi

# The profiles: the experiment's exact variables, nothing added, nothing missing.
profile="sh $here/profile-env.sh"
expected_a="DOTNET_ROOT=$root"
expected_b="DOTNET_ROOT=$root
DOTNET_TieredCompilation=0
DOTNET_EnableWriteXorExecute=0
DOTNET_gcServer=0
DOTNET_PROCESSOR_COUNT=1"
# shellcheck disable=SC2086  # $profile is a command and its first argument on purpose
got_a=$($profile a "$root" 2>/dev/null)
# shellcheck disable=SC2086
got_b=$($profile b "$root" 2>/dev/null)
[ "$got_a" = "$expected_a" ] && ok "arm a sets DOTNET_ROOT only" || bad "arm a is not DOTNET_ROOT only: $got_a"
[ "$got_b" = "$expected_b" ] && ok "arm b sets DOTNET_ROOT plus the four named variables" || bad "arm b differs from the named variables: $got_b"
expect "an unknown arm is a usage error" 64 sh "$here/profile-env.sh" c "$root"
expect "a missing runtime directory is a usage error" 64 sh "$here/profile-env.sh" a

# The program under each profile: the variables are valid for this runtime and the line is still the contract line.
i=0
while [ "$i" -lt "$runs" ]; do
  # The words are separate arguments to env on purpose.
  # shellcheck disable=SC2046,SC2086
  check_run "arm a, run $i" $($profile a "$root") || true
  # shellcheck disable=SC2046,SC2086
  check_run "arm b, run $i" $($profile b "$root") || true
  i=$((i + 1))
done
[ "$failures" -eq 0 ] && ok "both profiles run the probe and print a valid line"

if [ "$failures" -eq 0 ]; then
  echo "all native tests passed"
else
  echo "$failures native test(s) failed"
fi
[ "$failures" -eq 0 ]
