#!/bin/sh
# Build-record check B5 for a probe program, on `strace -ff -s 4096 -e trace=write` text of one run.
#
#   check-probe-writes.sh <strace-file>
#
# The probe prints one line on stdout and nothing else: exactly one write to descriptor 1, which begins with `PROBE v1 `,
# ends in a newline, is not truncated and is written whole (the return value equals the requested length). The probe has no
# SDK, so no protocol line (HAIRSPRING_EVENT) may appear anywhere in the trace.
# -s 4096 matters: strace's default 32-character strings would hide the line end.
# Exits 0 when everything holds, 1 when it does not, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <strace-file>" >&2
  exit 64
fi
file=$1
if [ ! -r "$file" ]; then
  echo "FAIL $file cannot be read" >&2
  exit 1
fi

awk '
  # strace prints a call that blocks as "<pid> write(..., N <unfinished ...>" and finishes it later as
  # "<pid> <... write resumed>) = N". Join the two halves by pid into one ordinary line.
  /<unfinished \.\.\.>$/ {
    pending[$1] = $0
    sub(/ <unfinished \.\.\.>$/, "", pending[$1])
    next
  }
  /<\.\.\. [a-z]+ resumed>/ {
    if (!($1 in pending)) next
    rest = $0
    sub(/^.*resumed>/, "", rest)
    $0 = pending[$1] rest
    delete pending[$1]
  }
  /HAIRSPRING_EVENT/ { events++ }
  match($0, /write\([0-9]+, "/) {
    head = substr($0, RSTART, RLENGTH)
    fd = head; sub(/^write\(/, "", fd); sub(/,.*$/, "", fd)
    rest = substr($0, RSTART + RLENGTH)
    if (!match(rest, /"(\.\.\.)?, [0-9]+\) += -?[0-9]+/)) next
    body = substr(rest, 1, RSTART - 1)
    tail = substr(rest, RSTART)
    count = tail; sub(/^"(\.\.\.)?, /, "", count); sub(/\).*$/, "", count)
    ret = tail; sub(/^.*= */, "", ret)
    if (fd == "1") {
      results++
      if (body !~ /^PROBE v1 /) { print "a write to stdout that does not begin with PROBE v1: " body > "/dev/stderr"; bad = 1 }
      if (tail ~ /^"\.\.\./) { print "a truncated string in the trace: raise -s" > "/dev/stderr"; bad = 1 }
      if (body !~ /\\n$/) { print "a write that does not end in a newline: " body > "/dev/stderr"; bad = 1 }
      if (count != ret) { print "a short write: asked " count ", wrote " ret > "/dev/stderr"; bad = 1 }
    }
  }
  END {
    if (events > 0) { print events " protocol line(s) in the trace, but the probe has no SDK" > "/dev/stderr"; bad = 1 }
    if (results != 1) { print results + 0 " writes to stdout, want exactly one" > "/dev/stderr"; bad = 1 }
    print results + 0 " result line write, " events + 0 " protocol lines"
    exit bad
  }
' "$file"
