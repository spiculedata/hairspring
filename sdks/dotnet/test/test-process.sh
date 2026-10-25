#!/bin/sh
# Process-level tests of the .NET SDK: the sample programs (samples/Sample) run as real processes, and what they write on
# standard error, in an events file and as an exit status is checked with grep. The reference parser (cargo test -p
# hairspring-props --test dotnet_sdk) judges the same programs more closely. No Hermit, no engine.
#
#   test-process.sh <Sample.dll>
#
# Environment: DOTNET is the dotnet command (default dotnet). Exits 0 when every test passed, 1 when one failed, 64 for a
# usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <Sample.dll>" >&2
  exit 64
fi
dll=$1
dotnet=${DOTNET:-dotnet}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hs-dotnet-process.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0
# A run that hangs is a failure, not a stuck test.
limit=""
for t in timeout gtimeout; do
  if command -v "$t" >/dev/null 2>&1; then
    limit="$t 120"
    break
  fi
done

ok() { echo "ok   $1"; }
bad() {
  echo "FAIL $1"
  failures=$((failures + 1))
}

# The SDK's own variables are removed once; a test that needs one passes it to runner_with. A deliberate crash writes no core dump.
unset HAIRSPRING_PROC HAIRSPRING_PROC_PID HAIRSPRING_EVENTS HAIRSPRING_EVENTS_FILE
export DOTNET_DbgEnableMiniDump=0 DOTNET_CLI_TELEMETRY_OPTOUT=1

# runner <dotnet arguments...>
runner() {
  # shellcheck disable=SC2086  # $limit is a command and its argument on purpose
  $limit "$dotnet" "$@"
}

# runner_with <NAME=value> <dotnet arguments...>
runner_with() {
  setting=$1
  shift
  # shellcheck disable=SC2086
  env "$setting" $limit "$dotnet" "$@"
}

# sample <scenario> [args...]: stdout in $tmp/out, stderr in $tmp/err; prints the exit status.
sample() {
  runner "$dll" "$@" >"$tmp/out" 2>"$tmp/err"
  echo $?
}

lines_of() { grep -c "\"kind\":\"$1\"" "$tmp/err"; }

# expect <name> <condition-command...>
expect() {
  name=$1
  shift
  if "$@"; then ok "$name"; else bad "$name"; fi
}

has() { grep -q -- "$1" "$tmp/err"; }
hasnt() { ! grep -q -- "$1" "$tmp/err"; }
is() { [ "$1" = "$2" ]; }
whole_lines() { is "$(grep -c '^HAIRSPRING_EVENT {.*}$' "$tmp/err")" "$(grep -c '^HAIRSPRING_EVENT' "$tmp/err")"; }

# basic: a complete report, only on standard error.
rc=$(sample basic)
expect "basic: exit 0" is "$rc" 0
expect "basic: nothing on stdout" test ! -s "$tmp/out"
first=$(head -1 "$tmp/err")
case "$first" in
  'HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"dotnet"'*) ok "basic: the first line is hello from the dotnet sdk" ;;
  *) bad "basic: the first line is hello from the dotnet sdk: $first" ;;
esac
expect "basic: seven catalog lines" is "$(lines_of catalog)" 7
expect "basic: seven assert lines" is "$(lines_of assert)" 7
expect "basic: one work line, one thread line, one end line" is "$(lines_of work)$(lines_of thread)$(lines_of end)" 111
expect "basic: end says seven sites, eleven evals, nothing dropped, one thread" has '"sites":7,"evals": *11,"dropped": *0,"threads":1}'
expect "basic: the end line is last" is "$(tail -1 "$tmp/err" | cut -c1-48)" 'HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1"'
expect "basic: the false always carries its witness" has '"id":"basic.always.bad".*"first_false":{"thread":"t#1","ord": *6,"vals":\[7,8,0,0\]}'
expect "basic: the location is the call site's file and line" has '"id":"basic.always.bad".*"loc":{"file":"Sample/Scenarios.cs","line":[0-9][0-9]*}'
expect "basic: every protocol line is whole" whole_lines

# the exit status of the program is its own, and the report knows it
rc=$(sample exit_status)
expect "exit: Hs.Exit(3) exits 3" is "$rc" 3
expect "exit: the end line carries status 3" has '"reason":"exit","status": *3,'
rc=$(sample plain_exit)
expect "exit: Environment.Exit(7) exits 7 and the exit event still reports" is "$rc" 7
expect "exit: that report has one end line, with the status" is "$(lines_of end)" 1
expect "exit: and the status is the one the process exits with" has '"reason":"exit","status": *7,'
rc=$(sample return_status)
expect "exit: a status returned from Main is in the end line" is "$rc$(grep -c '"reason":"exit","status": *9,' "$tmp/err")" 91
rc=$(sample raises)
expect "crash: an unhandled exception ends the process abnormally" test "$rc" -ne 0
expect "crash: and the report is written as a signal (the runtime aborts), once" is "$(lines_of end)$(grep -c '"reason":"signal","status": *6,' "$tmp/err")" 11
rc=$(sample halt)
expect "halt: _exit skips every exit handler and writes no report" is "$rc$(lines_of hello)$(lines_of end)" 500

# more threads than the table holds
rc=$(sample threads 100)
expect "threads: 100 threads and the main thread are 101 thread lines" is "$rc$(lines_of thread)" 0101
expect "threads: a worker's key is stable" has '"thread":"w-99"'
rc=$(sample threads 300)
expect "threads: 300 threads are capped at 256 thread lines" is "$rc$(lines_of thread)" 0256
expect "threads: the cap is counted as dropped" hasnt '"dropped": *0,"threads":256'
rc=$(sample pool)
expect "pool: the main thread and the pool thread each have a thread line" is "$rc$(lines_of thread)" 02

# one report per process
rc=$(sample explicit)
expect "explicit: one hello and one end although the program went on" is "$(lines_of hello)$(lines_of end)" 11
expect "explicit: the site reached before the report is there" has '"id":"explicit.before"'
expect "explicit: the site reached after the report is not" hasnt 'explicit.after'
expect "explicit: the end line says explicit" has '"reason":"explicit"'

# a background thread still evaluating while the process exits does not corrupt the report
rc=$(sample background)
expect "background: exit 0 and a complete report" is "$rc$(lines_of hello)$(lines_of end)" 011
expect "background: every protocol line is whole" whole_lines

# an explicit report from a background thread is finished before the process exits
rc=$(sample background_report)
expect "background_report: exit 0 and a complete report" is "$rc$(lines_of hello)$(lines_of end)" 011
expect "background_report: every protocol line is whole" whole_lines

# a forked copy of the process never reports; the parent reports once
rc=$(sample fork)
expect "fork: exit 0, one hello and one end" is "$rc$(lines_of hello)$(lines_of end)" 011
expect "fork: the report is the parent's" has '"kind":"hello".*"proc":"p1","catalog_hash"'

# a program that closes descriptor 2 does not lose the report when standard error is a pipe or a terminal: the SDK keeps
# its own duplicate. (A file behind descriptor 2 is never reopened, see the next tests.)
# shellcheck disable=SC2086
( runner "$dll" close_err >"$tmp/out"; echo $? >"$tmp/rc" ) 2>&1 | cat >"$tmp/err"
expect "close_err: exit 0 and a complete report on a pipe" is "$(cat "$tmp/rc")$(lines_of hello)$(lines_of end)" 011
expect "close_err: both sites are in the report" is "$(lines_of catalog)" 2

# the program's own writes to standard error after an explicit report do not overwrite it (standard error is a file)
rc=$(sample err_after_report)
expect "err_after_report: exit 0" is "$rc" 0
expect "err_after_report: the whole report survives the program's later writes" is "$(lines_of hello)$(lines_of end)$(grep -c '^HAIRSPRING_EVENT' "$tmp/err")" 116
expect "err_after_report: so do the program's own lines" is "$(grep -c 'the program writes to standard error' "$tmp/err")" 20

# standard error is a file open for reading only: nothing may be written to it, and the program still exits 0
printf 'a file that must not change\n' >"$tmp/readonly"
chmod 444 "$tmp/readonly"
cp "$tmp/readonly" "$tmp/readonly.before"
runner "$dll" basic >"$tmp/out" 2<"$tmp/readonly"
expect "read-only fd 2: the program exits 0" is "$?" 0
expect "read-only fd 2: the file is untouched" cmp -s "$tmp/readonly" "$tmp/readonly.before"
runner "$dll" basic >"$tmp/out" 2>&-
expect "closed fd 2: the program exits 0" is "$?" 0

# ids, messages and sites at their limits
rc=$(sample limits)
expect "limits: exit 0" is "$rc" 0
expect "limits: an id of 128 bytes" has '"id":"x\{128\}"'
expect "limits: an odd id is sent in its sanitised form" has '"id":"bad_id_with:odd/chars"'
rc=$(sample big)
expect "big: 64 sites report cleanly" is "$rc$(lines_of catalog)$(lines_of assert)" 06464
expect "big: no line is over 4096 bytes" is "$(awk 'length($0) > 4096' "$tmp/err" | wc -l | tr -d ' ')" 0

# arguments the SDK refuses are refused every time and count nothing
rc=$(sample bad_input)
expect "bad_input: five calls are refused" is "$rc$(cat "$tmp/out")" "0refused 5"
expect "bad_input: only the two good evaluations are counted" has '"evals": *2,'

# a catalog conflict is two catalog lines
rc=$(sample conflict)
expect "conflict: one id with two kinds is two catalog lines and one assert line" is "$(lines_of catalog)$(lines_of assert)" 21

# a child process that uses the SDK takes the parent's tag with its own pid
rc=$(sample spawn)
expect "spawn: exit 0" is "$rc" 0
expect "spawn: two hello lines" is "$(lines_of hello)" 2
expect "spawn: the child's tag is the parent's tag and its pid" has '"kind":"hello".*"proc":"p1\.s[0-9][0-9]*".*"tag_stable":false}'
expect "spawn: the parent keeps its tag" has '"kind":"hello".*"proc":"p1","catalog_hash"'

# the events file
events="$tmp/events"
runner_with HAIRSPRING_EVENTS_FILE=$events "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events file: standard error stays empty" test ! -s "$tmp/err"
expect "events file: the report is in the file" is "$(grep -c '^HAIRSPRING_EVENT' "$events")" 18
rm -f "$events"
runner_with HAIRSPRING_EVENTS_FILE=/nonexistent-dir-for-hairspring/events "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events file: one that cannot be opened falls back to standard error" is "$(lines_of end)" 1
expect "events file: and the fallback is counted as dropped" has '"dropped": *1,'
printf 'keep\n' >"$tmp/target"
ln -s "$tmp/target" "$tmp/link"
runner_with HAIRSPRING_EVENTS_FILE=$tmp/link "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events file: a symbolic link is refused and the target untouched" is "$(cat "$tmp/target")$(lines_of end)" keep1
ln -s "$tmp/nowhere" "$tmp/dangling"
runner_with HAIRSPRING_EVENTS_FILE=$tmp/dangling "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events file: a dangling link is refused and creates nothing" is "$(test -e "$tmp/nowhere" && echo made)$(lines_of end)" 1
if command -v mkfifo >/dev/null 2>&1 && mkfifo "$tmp/fifo" 2>/dev/null; then
  runner_with HAIRSPRING_EVENTS_FILE=$tmp/fifo "$dll" basic >"$tmp/out" 2>"$tmp/err"
  expect "events file: a FIFO is refused without hanging, and the fallback is counted" is "$(lines_of end)$(grep -c '"dropped": *1,' "$tmp/err")" 11
fi
if [ -e /dev/null ]; then
  runner_with HAIRSPRING_EVENTS_FILE=/dev/null "$dll" basic >"$tmp/out" 2>"$tmp/err"
  expect "events file: a device is refused and the fallback is counted" is "$(lines_of end)$(grep -c '"dropped": *1,' "$tmp/err")" 11
fi
runner_with HAIRSPRING_EVENTS_FILE=$tmp "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events file: a directory is refused and the fallback is counted" is "$(lines_of end)$(grep -c '"dropped": *1,' "$tmp/err")" 11
runner_with HAIRSPRING_EVENTS=off "$dll" basic >"$tmp/out" 2>"$tmp/err"
expect "events off: nothing is written" test ! -s "$tmp/err"

# a reader that has gone away costs the report, not the process
( runner "$dll" basic >/dev/null; echo $? >"$tmp/rc" ) 2>&1 | true
expect "closed stderr pipe: the program still exits 0" is "$(cat "$tmp/rc")" 0

# the harness's own tag is used
runner_with HAIRSPRING_PROC=job-7 "$dll" empty >"$tmp/out" 2>"$tmp/err"
expect "tag: HAIRSPRING_PROC names the root process" has '"kind":"hello".*"proc":"job-7"'
runner_with "HAIRSPRING_PROC=bad tag" "$dll" empty >"$tmp/out" 2>"$tmp/err"
expect "tag: an invalid tag is refused, p1 is used and the refusal is counted" is "$(grep -c '"proc":"p1"' "$tmp/err")$(grep -c '"dropped": *1,' "$tmp/err")" 41

# an unknown scenario is a usage error
rc=$(sample nonsense)
expect "usage: an unknown scenario exits 64 with a usage line" is "$rc$(grep -c usage "$tmp/err")" 641

exit $failures
