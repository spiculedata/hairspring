#!/bin/sh
# Build-record check B5 for a probe program, on `strace -ff -s 4096 -e trace=write,writev,fcntl,dup -o <prefix>` output of one
# run (the default mode needs only write and writev).
#
#   check-probe-writes.sh [--dup-of-stdout] <strace-file>...
#
# `strace -ff -o` writes one file per thread (<prefix>.<pid>), and a line has no pid column. A call that blocks is printed as
# `write(..., N <unfinished ...>` and finished later in the same file as `<... write resumed>) = N`; the two halves are
# joined within one file (a leading pid column, as `strace -f` prints it, is understood too). Pass every file of the run.
#
# The probe prints one line on stdout and nothing else: exactly one write to descriptor 1 across all the files, which begins
# with `PROBE v1 `, ends in a newline, is not truncated and is written whole (the return value equals the requested
# length). A writev to descriptor 1 is a failure (the line must be one write(2)). The probe has no SDK, so no protocol line
# (HAIRSPRING_EVENT) may appear anywhere in the trace.
#
# --dup-of-stdout is for the .NET probe: the runtime duplicates descriptor 1 for its console stream (`fcntl(1,
# F_DUPFD_CLOEXEC, 0) = 24`), so the line is written to the duplicate (`write(24, "PROBE v1 ...")`). With the flag the same
# rules hold for the line, which must be written to a descriptor that a traced `fcntl(1, F_DUPFD..., ...)` or `dup(1)`
# created (the dups of every file are collected first, so the order of the files does not matter), and in addition any
# write or writev to descriptor 1 or 2 is a failure: the runtime writes nothing there, so the line cannot have gone to
# standard error or to a duplicate of it. Writes of other things (thread names, wake-up bytes) to other descriptors are
# ignored. That the duplicate really is the program's standard output is for the caller to show (the captured stdout holds
# exactly the line, stderr is empty); a trace alone cannot tell a duplicate of 1 that was later redirected.
# -s 4096 matters: strace's default 32-character strings would hide the line end.
# Exits 0 when everything holds, 1 when it does not, 64 for a usage error.

set -u

dup_mode=0
if [ "${1:-}" = "--dup-of-stdout" ]; then
  dup_mode=1
  shift
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--dup-of-stdout] <strace-file>..." >&2
  exit 64
fi
for file in "$@"; do
  if [ ! -r "$file" ]; then
    echo "FAIL $file cannot be read" >&2
    exit 1
  fi
done

# The descriptors that a traced dup of descriptor 1 returned, as " 6 24 39 " (empty when there are none).
dups=" "
if [ "$dup_mode" -eq 1 ]; then
  found=$(cat "$@" | grep -E '(fcntl\(1, F_DUPFD(_CLOEXEC)?, [0-9]+\)|dup\(1\)) += [0-9]+' | sed -E 's/^.*= *([0-9]+).*$/\1/' | tr '\n' ' ')
  dups=" $found"
fi

awk -v dupmode="$dup_mode" -v dups="$dups" '
  # The key of a line: its file, and its pid column when it has one.
  function key() { return FILENAME SUBSEP (($1 ~ /^[0-9]+$/) ? $1 : "") }
  # The descriptor of the write( or writev( call that starts at the match.
  function fd_of(text) { sub(/^[^(]*\(/, "", text); sub(/[,)].*$/, "", text); return text }
  /<unfinished \.\.\.>$/ {
    pending[key()] = $0
    sub(/ <unfinished \.\.\.>$/, "", pending[key()])
    next
  }
  /<\.\.\. [a-z]+ resumed>/ {
    if (!(key() in pending)) next
    rest = $0
    sub(/^.*resumed>/, "", rest)
    $0 = pending[key()] rest
    delete pending[key()]
  }
  /HAIRSPRING_EVENT/ { events++ }
  dupmode == 0 && /(^|[^a-z])writev\(1,/ { vectors++ }
  dupmode == 1 && /(^|[^a-z])writev\([0-9]+, .*PROBE v1 / { vectors++ }
  dupmode == 1 && /(^|[^a-z])writev?\([12],/ { print "a write to descriptor 1 or 2: " substr($0, 1, 100) > "/dev/stderr"; bad = 1 }
  match($0, dupmode ? "(^|[^a-z])write\\([0-9]+, \"" : "(^|[^a-z])write\\(1, \"") {
    call = substr($0, RSTART, RLENGTH)
    rest = substr($0, RSTART + RLENGTH)
    if (!match(rest, /"(\.\.\.)?, [0-9]+\) += -?[0-9]+/)) next
    body = substr(rest, 1, RSTART - 1)
    if (dupmode && body !~ /^PROBE v1 /) next
    tail = substr(rest, RSTART)
    count = tail; sub(/^"(\.\.\.)?, /, "", count); sub(/\).*$/, "", count)
    ret = tail; sub(/^.*= */, "", ret)
    results++
    if (dupmode && index(dups, " " fd_of(call) " ") == 0) { print "the line is on descriptor " fd_of(call) ", which no traced dup of descriptor 1 created" > "/dev/stderr"; bad = 1 }
    if (body !~ /^PROBE v1 /) { print "a write to stdout that does not begin with PROBE v1: " body > "/dev/stderr"; bad = 1 }
    if (tail ~ /^"\.\.\./) { print "a truncated string in the trace: raise -s" > "/dev/stderr"; bad = 1 }
    if (body !~ /\\n$/) { print "a write that does not end in a newline: " body > "/dev/stderr"; bad = 1 }
    if (count != ret) { print "a short write: asked " count ", wrote " ret > "/dev/stderr"; bad = 1 }
  }
  END {
    if (vectors > 0) { print vectors " writev call(s) of the line: it must be one write(2)" > "/dev/stderr"; bad = 1 }
    if (events > 0) { print events " protocol line(s) in the trace, but the probe has no SDK" > "/dev/stderr"; bad = 1 }
    if (results != 1) { print results + 0 " writes of the line, want exactly one" > "/dev/stderr"; bad = 1 }
    print results + 0 " result line write, " vectors + 0 " writev, " events + 0 " protocol lines"
    exit bad
  }
' "$@"
