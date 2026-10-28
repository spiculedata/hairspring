#!/bin/sh
# Process-level tests of the Java SDK: the sample programs (samples/hssample) run as real JVMs, and what they write on
# standard error, in an events file and as an exit status is checked with grep. The reference parser (cargo test -p
# hairspring-props --test java_sdk) judges the same programs more closely. No Hermit, no engine.
#
#   test-process.sh <classes-dir> <samples-dir> <test-classes-dir>
#
# Environment: JAVA is the java command (default java), JAVA_FLAGS are extra JVM flags put before -cp (default none).
# Exits 0 when every test passed, 1 when one failed, 64 for a usage error.

set -u

if [ $# -ne 3 ]; then
  echo "usage: $0 <classes-dir> <samples-dir> <test-classes-dir>" >&2
  exit 64
fi
classes=$1
samples=$2
java=${JAVA:-java}
flags=${JAVA_FLAGS:-}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hs-java-process.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0

ok() { echo "ok   $1"; }
bad() {
  echo "FAIL $1"
  failures=$((failures + 1))
}

# sample <scenario> [args...]: stdout in $tmp/out, stderr in $tmp/err; prints the exit status. The SDK's environment is removed.
sample() {
  # shellcheck disable=SC2086  # flags is a list of words on purpose
  env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE \
    "$java" $flags -cp "$classes:$samples" hssample.Sample "$@" >"$tmp/out" 2>"$tmp/err"
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

# basic: a complete report, only on standard error.
rc=$(sample basic)
expect "basic: exit 0" is "$rc" 0
expect "basic: nothing on stdout" test ! -s "$tmp/out"
first=$(head -1 "$tmp/err")
case "$first" in
  'HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"java"'*) ok "basic: the first line is hello from the java sdk" ;;
  *) bad "basic: the first line is hello from the java sdk: $first" ;;
esac
expect "basic: seven catalog lines" is "$(lines_of catalog)" 7
expect "basic: seven assert lines" is "$(lines_of assert)" 7
expect "basic: one work line, one thread line, one end line" is "$(lines_of work)$(lines_of thread)$(lines_of end)" 111
expect "basic: end says seven sites, eleven evals, nothing dropped, one thread" has '"sites":7,"evals": *11,"dropped": *0,"threads":1}'
expect "basic: the end line is last" is "$(tail -1 "$tmp/err" | cut -c1-48)" 'HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1"'
expect "basic: the false always carries its witness" has '"id":"basic.always.bad".*"first_false":{"thread":"t#1","ord": *6,"vals":\[7,8,0,0\]}'

# the exit status of the program is its own, whatever the report says
rc=$(sample exit_status)
expect "exit: Hairspring.exit(3) exits 3" is "$rc" 3
expect "exit: the end line carries status 3" has '"reason":"exit","status": *3,'
rc=$(sample plain_exit)
expect "exit: System.exit(7) exits 7 and the shutdown hook still reports" is "$rc" 7
expect "exit: that report has an end line" is "$(lines_of end)" 1
rc=$(sample raises)
expect "exit: an uncaught exception exits 1 and the report is still written" is "$rc$(lines_of end)" 11
rc=$(sample halt)
expect "exit: Runtime.halt exits with its status and writes no report" is "$rc$(lines_of hello)$(lines_of end)" 500

# more threads than the table holds
rc=$(sample threads 100)
expect "threads: 100 threads and the main thread are 101 thread lines" is "$rc$(lines_of thread)" 0101
expect "threads: a worker's key is stable" has '"thread":"w-99"'
rc=$(sample threads 300)
expect "threads: 300 threads are capped at 256 thread lines" is "$rc$(lines_of thread)" 0256
expect "threads: the cap is counted as dropped" hasnt '"dropped": *0,"threads":256'

# one report per process
rc=$(sample explicit)
expect "explicit: one hello and one end although the program went on" is "$(lines_of hello)$(lines_of end)" 11
expect "explicit: the site reached before the report is there" has '"id":"explicit.before"'
expect "explicit: the site reached after the report is not" hasnt 'explicit.after'
expect "explicit: the end line says explicit" has '"reason":"explicit"'

# a daemon thread still evaluating while the JVM exits does not corrupt the report
rc=$(sample daemon)
expect "daemon: exit 0 and a complete report" is "$rc$(lines_of hello)$(lines_of end)" 011
expect "daemon: every protocol line is whole" is "$(grep -c '^HAIRSPRING_EVENT {.*}$' "$tmp/err")" "$(grep -c '^HAIRSPRING_EVENT' "$tmp/err")"

# a program that closes System.err does not lose the report when standard error is a pipe or a terminal: the SDK keeps its
# own channel to it. (A regular file behind descriptor 2 is never reopened, see the next tests.)
# shellcheck disable=SC2086
( "$java" $flags -cp "$classes:$samples" hssample.Sample close_err >"$tmp/out"; echo $? >"$tmp/rc" ) 2>&1 | cat >"$tmp/err"
expect "close_err: exit 0 and a complete report on a pipe" is "$(cat "$tmp/rc")$(lines_of hello)$(lines_of end)" 011
expect "close_err: both sites are in the report" is "$(lines_of catalog)" 2

# the program's own writes to standard error after an explicit report do not overwrite it (standard error is a file)
rc=$(sample err_after_report)
expect "err_after_report: exit 0" is "$rc" 0
expect "err_after_report: the whole report survives the program's later writes" is "$(lines_of hello)$(lines_of end)$(grep -c '^HAIRSPRING_EVENT' "$tmp/err")" 116
expect "err_after_report: so do the program's own lines" is "$(grep -c 'the program writes to standard error' "$tmp/err")" 20

# standard error is a file the program has open for reading only (the JVM launched with 2>&- can get such a file as descriptor
# 2): nothing may be written to it, and the program still exits 0
printf 'a file that must not change\n' >"$tmp/readonly"
chmod 444 "$tmp/readonly"
cp "$tmp/readonly" "$tmp/readonly.before"
# shellcheck disable=SC2086
"$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2<"$tmp/readonly"
expect "read-only fd 2: the program exits 0" is "$?" 0
expect "read-only fd 2: the file is untouched" cmp -s "$tmp/readonly" "$tmp/readonly.before"
# shellcheck disable=SC2086
"$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2>&-
expect "closed fd 2: the program exits 0" is "$?" 0

# standard error is a regular file the program has open for reading only (mode 644, so the path would allow a write), and
# the program closes System.err: the SDK must not reopen the file, whatever the platform
printf 'a file that must not change\n' >"$tmp/readonly644"
chmod 644 "$tmp/readonly644"
cp "$tmp/readonly644" "$tmp/readonly644.before"
# shellcheck disable=SC2086
"$java" $flags -cp "$classes:$samples" hssample.Sample close_err >"$tmp/out" 2<"$tmp/readonly644"
expect "read-only fd 2, close_err: the program exits 0" is "$?" 0
expect "read-only fd 2, close_err: the file (mode 644) is unchanged" cmp -s "$tmp/readonly644" "$tmp/readonly644.before"

# standard error is a regular file opened for writing and the program closes System.err. Where Linux shows the open flags
# (/proc/self/fdinfo/2) the report is kept; elsewhere it is not reopened, so only the exit status is checked.
: >"$tmp/err"
# shellcheck disable=SC2086
"$java" $flags -cp "$classes:$samples" hssample.Sample close_err >"$tmp/out" 2>"$tmp/err"
expect "close_err on a file: the program exits 0" is "$?" 0
if [ -r /proc/self/fdinfo/2 ]; then
  expect "close_err on a file (Linux): the report is kept, hello to end" is "$(lines_of hello)$(lines_of end)$(lines_of catalog)" 112
else
  echo "skip close_err on a file: no /proc/self/fdinfo here, so the report is not kept (the unit tests simulate Linux)"
fi

# an explicit report from a daemon thread is finished before the JVM exits
rc=$(sample daemon_report)
expect "daemon_report: exit 0 and a complete report" is "$rc$(lines_of hello)$(lines_of end)" 011
expect "daemon_report: every protocol line is whole" is "$(grep -c '^HAIRSPRING_EVENT {.*}$' "$tmp/err")" "$(grep -c '^HAIRSPRING_EVENT' "$tmp/err")"

# ids, messages and sites at their limits
rc=$(sample limits)
expect "limits: exit 0" is "$rc" 0
expect "limits: an id of 128 bytes" has '"id":"x\{128\}"'
expect "limits: an odd id is sent in its sanitised form" has '"id":"bad_id_with:odd/chars"'
rc=$(sample big)
expect "big: 64 sites report cleanly" is "$rc$(lines_of catalog)$(lines_of assert)" 06464
expect "big: no line is over 4096 bytes" is "$(awk 'length($0) > 4096' "$tmp/err" | wc -l | tr -d ' ')" 0

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
# shellcheck disable=SC2086
HAIRSPRING_EVENTS_FILE=$events "$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2>"$tmp/err"
expect "events file: standard error stays empty" test ! -s "$tmp/err"
expect "events file: the report is in the file" is "$(grep -c '^HAIRSPRING_EVENT' "$events")" 18
rm -f "$events"
# shellcheck disable=SC2086
HAIRSPRING_EVENTS_FILE=/nonexistent-dir-for-hairspring/events "$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2>"$tmp/err"
expect "events file: one that cannot be opened falls back to standard error" is "$(lines_of end)" 1
expect "events file: and the fallback is counted as dropped" has '"dropped": *1,'
printf 'keep\n' >"$tmp/target"
ln -s "$tmp/target" "$tmp/link"
# shellcheck disable=SC2086
HAIRSPRING_EVENTS_FILE=$tmp/link "$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2>"$tmp/err"
expect "events file: a symbolic link is refused and the target untouched" is "$(cat "$tmp/target")$(lines_of end)" keep1
# shellcheck disable=SC2086
HAIRSPRING_EVENTS=off "$java" $flags -cp "$classes:$samples" hssample.Sample basic >"$tmp/out" 2>"$tmp/err"
expect "events off: nothing is written" test ! -s "$tmp/err"

# a reader that has gone away costs the report, not the process
# shellcheck disable=SC2086
( "$java" $flags -cp "$classes:$samples" hssample.Sample basic >/dev/null; echo $? >"$tmp/rc" ) 2>&1 | true
expect "closed stderr pipe: the program still exits 0" is "$(cat "$tmp/rc")" 0

# the harness's own tag is used
# shellcheck disable=SC2086
HAIRSPRING_PROC=job-7 "$java" $flags -cp "$classes:$samples" hssample.Sample empty >"$tmp/out" 2>"$tmp/err"
expect "tag: HAIRSPRING_PROC names the root process" has '"kind":"hello".*"proc":"job-7"'

# an unknown scenario is a usage error
rc=$(sample nonsense)
expect "usage: an unknown scenario exits 64 with a usage line" is "$rc$(grep -c usage "$tmp/err")" 641

exit $failures
