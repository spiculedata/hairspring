#!/bin/sh
# Build-record check B5 on `strace -f -s 4096 -e trace=write` text of one run.
#
#   check-writes.sh <strace-file>
#
# Every protocol line (HAIRSPRING_EVENT) must be one whole write(2)-style call: the string ends in a newline, is not
# truncated, and the return value equals the requested length. A line split across two writes shows as a write on the
# events descriptor that does not begin with HAIRSPRING_EVENT, or one that does not end in a newline. The result line
# on stdout must be exactly one such write, beginning with `LANGAT v1`.
# -s 4096 matters: strace's default 32-character strings would hide the line ends.
# Exits 0 when everything holds, 1 when it does not, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <strace-file>" >&2
  exit 64
fi
file=$1

awk '
  # write(<fd>, "<text>", <count>) = <ret>   (after an optional pid)
  match($0, /write\([0-9]+, "/) {
    head = substr($0, RSTART, RLENGTH)
    fd = head; sub(/^write\(/, "", fd); sub(/,.*$/, "", fd)
    rest = substr($0, RSTART + RLENGTH)
    # the string ends at the last quote before ", <count>) = <ret>"
    if (!match(rest, /"(\.\.\.)?, [0-9]+\) += -?[0-9]+/)) next
    body = substr(rest, 1, RSTART - 1)
    tail = substr(rest, RSTART)
    truncated = (tail ~ /^"\.\.\./)
    count = tail; sub(/^"(\.\.\.)?, /, "", count); sub(/\).*$/, "", count)
    ret = tail; sub(/^.*= */, "", ret)
    ends_in_newline = (body ~ /\\n$/)
    n++
    fds[n] = fd; bodies[n] = body; counts[n] = count; rets[n] = ret; trunc[n] = truncated; nl[n] = ends_in_newline
    if (body ~ /^HAIRSPRING_EVENT/) event_fd[fd] = 1
  }
  END {
    bad = 0; events = 0; results = 0
    for (i = 1; i <= n; i++) {
      isevent = (bodies[i] ~ /^HAIRSPRING_EVENT/) || (fds[i] in event_fd)
      if (isevent) {
        events++
        if (bodies[i] !~ /^HAIRSPRING_EVENT/) { print "a write on the events descriptor that is not a whole protocol line: " bodies[i] > "/dev/stderr"; bad = 1 }
      }
      if (fds[i] == "1") {
        results++
        if (bodies[i] !~ /^LANGAT v1 /) { print "a write to stdout that does not begin with LANGAT v1: " bodies[i] > "/dev/stderr"; bad = 1 }
      }
      if (isevent || fds[i] == "1") {
        if (trunc[i]) { print "a truncated string in the trace: raise -s" > "/dev/stderr"; bad = 1 }
        if (!nl[i]) { print "a write that does not end in a newline: " bodies[i] > "/dev/stderr"; bad = 1 }
        if (counts[i] != rets[i]) { print "a short write: asked " counts[i] ", wrote " rets[i] > "/dev/stderr"; bad = 1 }
      }
    }
    if (events == 0) { print "no protocol line written" > "/dev/stderr"; bad = 1 }
    if (results != 1) { print results " writes to stdout, want exactly one" > "/dev/stderr"; bad = 1 }
    print events " protocol line writes, " results " result line write"
    exit bad
  }
' "$file"
