#!/bin/sh
# Evidence for the fixture alignment checklist, captured from the built artifact and its source. Each check prints what a
# reviewer needs to read and exits 0 only when the evidence passes, so a build record can tick the line it proves.
#
#   check-evidence.sh capture    <artifact> <mode> <prefix>   run once; keep <prefix>.out, <prefix>.err, <prefix>.rc
#   check-evidence.sh usage      <lang> <artifact>            line 2: no argument and a bad argument: usage line, exit 64
#   check-evidence.sh barrier    <source-file>...             line 4: the start barrier in the source
#   check-evidence.sh witness    <lang> <prefix>              line 6: the always and sometimes event lines of a run
#   check-evidence.sh ids        <lang> <prefix>              line 7: the property ids in those lines
#   check-evidence.sh stdout-line <lang> <mode> <prefix>      line 8: the one LANGAT v1 stdout line
#   check-evidence.sh exit-codes <lang> <artifact>            line 9: exit 0 (every mode), 3 (unwritable stdout), 64
#
# A captured run is made with the SDK's environment removed, as check-native.sh does. The protocol lines are padded with
# spaces by the SDKs (fixed-width fields); the printed copies squeeze them, the checks read the raw lines.
# Exits 0 when the evidence passes, 1 when it does not, 64 for a usage error.

set -u

EXPECTED=4000
USAGE_STATUS=64
INVARIANT_STATUS=3

die_usage() {
  echo "usage: $0 capture <artifact> <mode> <prefix> | usage <lang> <artifact> | barrier <source>... | witness <lang> <prefix> | ids <lang> <prefix> | stdout-line <lang> <mode> <prefix> | exit-codes <lang> <artifact>" >&2
  exit 64
}

limit=""
for t in timeout gtimeout; do
  if command -v "$t" >/dev/null 2>&1; then
    limit="$t ${RUN_TIMEOUT:-60}"
    break
  fi
done

# run_artifact <out> <err> <args...>: the artifact without the SDK's environment; the exit status is the result.
run_artifact() {
  run_out=$1
  run_err=$2
  shift 2
  # shellcheck disable=SC2086  # $limit is a command and its argument on purpose
  env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $limit "$artifact" "$@" >"$run_out" 2>"$run_err"
}

squeeze() { sed 's/  */ /g'; }

count_lines() { if [ -z "$1" ]; then echo 0; else printf '%s\n' "$1" | wc -l | tr -d ' '; fi; }

cmd_capture() {
  [ $# -eq 3 ] || die_usage
  artifact=$1
  mode=$2
  prefix=$3
  run_artifact "$prefix.out" "$prefix.err" "$mode"
  rc=$?
  echo "$rc" >"$prefix.rc"
  if [ "$rc" -eq 126 ] || [ "$rc" -eq 127 ]; then
    echo "cannot run $artifact (exit status $rc)" >&2
    return 1
  fi
  echo "\$ $(basename "$artifact") $mode  (exit status $rc)"
}

# usage_run <label> <args...>: runs with these arguments and shows stderr, stdout and the exit status.
usage_run() {
  label=$1
  shift
  tmp_out=$(mktemp "${TMPDIR:-/tmp}/evidence.XXXXXX")
  tmp_err=$(mktemp "${TMPDIR:-/tmp}/evidence.XXXXXX")
  run_artifact "$tmp_out" "$tmp_err" "$@"
  got=$?
  # The SDKs also report to stderr at exit, so the usage line is the stderr line that is not a protocol line.
  usage_line=$(grep -v '^HAIRSPRING_EVENT' "$tmp_err" | head -3 | tr '\n' ' ')
  echo "\$ $(basename "$artifact") $label"
  echo "usage line: $usage_line"
  echo "(stderr also holds $(grep -c '^HAIRSPRING_EVENT' "$tmp_err") protocol lines)"
  if [ -s "$tmp_out" ]; then echo "stdout: $(head -c 300 "$tmp_out" | tr '\n' ' ')"; else echo "stdout: (empty)"; fi
  echo "exit status: $got"
  good=0
  if [ "$got" -ne "$USAGE_STATUS" ]; then
    echo "FAIL ($label): exit status $got, want $USAGE_STATUS" >&2
    good=1
  elif [ -s "$tmp_out" ] || ! printf '%s\n' "$usage_line" | grep -qi usage; then
    echo "FAIL ($label): want a usage line on stderr and nothing on stdout" >&2
    good=1
  fi
  rm -f "$tmp_out" "$tmp_err"
  return $good
}

cmd_usage() {
  [ $# -eq 2 ] || die_usage
  lang=$1
  artifact=$2
  echo "usage errors of the $lang artifact: a bad argument and no argument"
  status=0
  usage_run "racy (a bad argument)" racy || status=1
  usage_run "(no argument)" || status=1
  return $status
}

BARRIER_INCREMENT='(started[^;]*(fetch_add|\.Add\(|\+= *1|\+\+)|(fetch_add|\.Add)\(&?[A-Za-z_.>-]*started)'
BARRIER_WAIT='(started[^;]*(<|>=|!=|==) *(LANG_AT_THREADS|Workers|THREADS|4)|barrier\.wait\(\))'

cmd_barrier() {
  [ $# -ge 1 ] || die_usage
  incs=0
  waits=0
  for file in "$@"; do
    [ -f "$file" ] || { echo "no such source file: $file" >&2; return 1; }
    echo "--- $file: the lines that count the started workers and wait for all of them (with two lines of context)"
    grep -n -E -B2 -A2 -e "$BARRIER_INCREMENT" -e "$BARRIER_WAIT" "$file"
    incs=$((incs + $(grep -c -E "$BARRIER_INCREMENT" "$file")))
    waits=$((waits + $(grep -c -E "$BARRIER_WAIT" "$file")))
  done
  echo "--- increments of started found: $incs; waits for all workers found: $waits"
  if [ "$incs" -eq 0 ] || [ "$waits" -eq 0 ]; then
    echo "FAIL no start barrier: want an increment of started and a wait until every worker has started" >&2
    return 1
  fi
}

# witness_id <id> <type> <final-or-empty>: one catalog line of that type, exactly one assert line evaluated once, and,
# when a final value is given, the witness values [final, 4000, 4, 1000] in it.
witness_id() {
  id=$1
  type=$2
  want_final=$3
  bad=0
  catalog_lines=$(grep "\"kind\":\"catalog\".*\"id\":\"$id\"" "$err")
  assert_lines=$(grep "\"kind\":\"assert\".*\"id\":\"$id\"" "$err")
  printf '%s\n' "$catalog_lines" | squeeze
  printf '%s\n' "$assert_lines" | squeeze
  if [ "$(count_lines "$catalog_lines")" -ne 1 ] || ! printf '%s\n' "$catalog_lines" | grep -q "\"type\":\"$type\""; then
    echo "FAIL $id: want exactly one catalog line of type $type" >&2
    bad=1
  fi
  if [ "$(count_lines "$assert_lines")" -ne 1 ]; then
    echo "FAIL $id: want exactly one assert line, found $(count_lines "$assert_lines")" >&2
    bad=1
  elif ! printf '%s\n' "$assert_lines" | grep -Eq '"hits": *1,'; then
    echo "FAIL $id: not evaluated exactly once" >&2
    bad=1
  fi
  if [ -n "$want_final" ] && ! printf '%s\n' "$assert_lines" | grep -Eq "\"vals\":\[$want_final,$EXPECTED,4,1000\]"; then
    echo "FAIL $id: the witness values are not [$want_final,$EXPECTED,4,1000]" >&2
    bad=1
  fi
  return $bad
}

captured() {
  prefix=$1
  err=$prefix.err
  out=$prefix.out
  [ -f "$err" ] && [ -f "$out" ] || { echo "no captured run at $prefix" >&2; return 1; }
}

cmd_witness() {
  [ $# -eq 2 ] || die_usage
  lang=$1
  captured "$2" || return 1
  final=$(sed -n 's/^.* final=\([0-9]*\) expect=.*/\1/p' "$out" | sed 's/^0*\(.\)/\1/')
  echo "one native run; its stdout reports final=$final"
  status=0
  witness_id "$lang.counter_exact" always "$final" || status=1
  witness_id "$lang.all_started" sometimes "" || status=1
  [ "$status" -ne 0 ] || echo "ok   each property is evaluated once, with the witness values [$final, $EXPECTED, 4, 1000] for $lang.counter_exact"
  return $status
}

cmd_ids() {
  [ $# -eq 2 ] || die_usage
  lang=$1
  captured "$2" || return 1
  found=$(sed -n 's/.*"kind":"assert".*"id":"\([^"]*\)".*/\1/p' "$err" | sort)
  want=$(printf '%s\n%s\n' "$lang.all_started" "$lang.counter_exact" | sort)
  echo "ids in the assert lines of that run:"
  printf '%s\n' "$found"
  if [ "$found" != "$want" ]; then
    echo "FAIL the ids are not exactly $lang.counter_exact and $lang.all_started" >&2
    return 1
  fi
  echo "ok   exactly $lang.counter_exact and $lang.all_started"
}

cmd_stdout_line() {
  [ $# -eq 3 ] || die_usage
  lang=$1
  mode=$2
  captured "$3" || return 1
  echo "stdout of one $mode run:"
  cat "$out"
  shape="^LANGAT v1 lang=$lang mode=$mode final=[0-9]{20} expect=0{16}$EXPECTED order=[0-9a-f]{16} rand=[0-9a-f]{16}$"
  if [ "$(wc -l <"$out" | tr -d ' ')" -ne 1 ] || ! grep -Eq "$shape" "$out"; then
    echo "FAIL stdout is not the one contract line" >&2
    return 1
  fi
  echo "ok   one line in the contract form ($(wc -c <"$out" | tr -d ' ') bytes)"
}

# exit_status <label> <want> <got>
exit_status() {
  echo "$1: exit status $3"
  if [ "$3" -ne "$2" ]; then
    echo "FAIL $1: exit status $3, want $2" >&2
    return 1
  fi
}

cmd_exit_codes() {
  [ $# -eq 2 ] || die_usage
  artifact=$2
  tmp_out=$(mktemp "${TMPDIR:-/tmp}/evidence.XXXXXX")
  tmp_err=$(mktemp "${TMPDIR:-/tmp}/evidence.XXXXXX")
  status=0
  for mode in buggy fixed wide; do
    run_artifact "$tmp_out" "$tmp_err" "$mode"
    exit_status "$mode run (exit 0 whatever the property says)" 0 $? || status=1
  done
  if [ -w /dev/full ]; then
    # shellcheck disable=SC2086  # $limit is a command and its argument on purpose
    env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $limit "$artifact" fixed >/dev/full 2>"$tmp_err"
    exit_status "fixed run with stdout on /dev/full (the result line cannot be written)" "$INVARIANT_STATUS" $? || status=1
  else
    # shellcheck disable=SC2086
    env -u HAIRSPRING_PROC -u HAIRSPRING_PROC_PID -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $limit "$artifact" fixed >&- 2>"$tmp_err"
    exit_status "fixed run with stdout closed (the result line cannot be written)" "$INVARIANT_STATUS" $? || status=1
  fi
  echo "stderr of that run, without protocol lines: $(grep -v '^HAIRSPRING_EVENT' "$tmp_err" | head -3 | tr '\n' ' ')"
  run_artifact "$tmp_out" "$tmp_err" racy
  exit_status "run with a bad argument" "$USAGE_STATUS" $? || status=1
  rm -f "$tmp_out" "$tmp_err"
  return $status
}

[ $# -ge 1 ] || die_usage
check=$1
shift
case "$check" in
  capture) cmd_capture "$@" ;;
  usage) cmd_usage "$@" ;;
  barrier) cmd_barrier "$@" ;;
  witness) cmd_witness "$@" ;;
  ids) cmd_ids "$@" ;;
  stdout-line) cmd_stdout_line "$@" ;;
  exit-codes) cmd_exit_codes "$@" ;;
  *) die_usage ;;
esac
