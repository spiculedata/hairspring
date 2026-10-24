#!/bin/sh
# Build-record check B5 for a probe program, on `strace -ff -s 4096 -e trace=write,writev -o <prefix>` output of one run.
#
#   check-probe-writes.sh [--any-fd] <strace-file>...
#
# `strace -ff -o` writes one file per thread (<prefix>.<pid>), and a line has no pid column. A call that blocks is printed as
# `write(..., N <unfinished ...>` and finished later in the same file as `<... write resumed>) = N`; the two halves are
# joined within one file (a leading pid column, as `strace -f` prints it, is understood too). Pass every file of the run.
#
# The probe prints one line on stdout and nothing else: exactly one write to descriptor 1 across all the files, which begins
# with `PROBE v1 `, ends in a newline, is not truncated and is written whole (the return value equals the requested
# length). A writev to descriptor 1 is a failure (the line must be one write(2)). The probe has no SDK, so no protocol line
# (HAIRSPRING_EVENT) may appear anywhere in the trace.
# -s 4096 matters: strace's default 32-character strings would hide the line end.
# Exits 0 when everything holds, 1 when it does not, 64 for a usage error.

set -u

# --any-fd is for the .NET probe: the runtime duplicates descriptor 1 for its console stream, so the line is written to
# another descriptor (write(24, "PROBE v1 ...")). With it the line is looked for on any descriptor, a write that does not
# begin with PROBE v1 (the runtime's own thread names and wake-up bytes) is ignored, and every other rule is unchanged.
any_fd=0
if [ "${1:-}" = "--any-fd" ]; then
  any_fd=1
  shift
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--any-fd] <strace-file>..." >&2
  exit 64
fi
for file in "$@"; do
  if [ ! -r "$file" ]; then
    echo "FAIL $file cannot be read" >&2
    exit 1
  fi
done

awk -v anyfd="$any_fd" '
  # The key of a line: its file, and its pid column when it has one.
  function key() { return FILENAME SUBSEP (($1 ~ /^[0-9]+$/) ? $1 : "") }
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
  anyfd == 0 && /(^|[^a-z])writev\(1,/ { vectors++ }
  anyfd == 1 && /(^|[^a-z])writev\([0-9]+, .*PROBE v1 / { vectors++ }
  match($0, anyfd ? "(^|[^a-z])write\\([0-9]+, \"" : "(^|[^a-z])write\\(1, \"") {
    rest = substr($0, RSTART + RLENGTH)
    if (!match(rest, /"(\.\.\.)?, [0-9]+\) += -?[0-9]+/)) next
    body = substr(rest, 1, RSTART - 1)
    if (anyfd && body !~ /^PROBE v1 /) next
    tail = substr(rest, RSTART)
    count = tail; sub(/^"(\.\.\.)?, /, "", count); sub(/\).*$/, "", count)
    ret = tail; sub(/^.*= */, "", ret)
    results++
    if (body !~ /^PROBE v1 /) { print "a write to stdout that does not begin with PROBE v1: " body > "/dev/stderr"; bad = 1 }
    if (tail ~ /^"\.\.\./) { print "a truncated string in the trace: raise -s" > "/dev/stderr"; bad = 1 }
    if (body !~ /\\n$/) { print "a write that does not end in a newline: " body > "/dev/stderr"; bad = 1 }
    if (count != ret) { print "a short write: asked " count ", wrote " ret > "/dev/stderr"; bad = 1 }
  }
  END {
    if (vectors > 0) { print vectors " writev call(s) to stdout: the line must be one write(2)" > "/dev/stderr"; bad = 1 }
    if (events > 0) { print events " protocol line(s) in the trace, but the probe has no SDK" > "/dev/stderr"; bad = 1 }
    if (results != 1) { print results + 0 " writes to stdout, want exactly one" > "/dev/stderr"; bad = 1 }
    print results + 0 " result line write, " vectors + 0 " writev, " events + 0 " protocol lines"
    exit bad
  }
' "$@"
