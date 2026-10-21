#!/bin/sh
# Build-record check B2 on `objdump -d --no-show-raw-insn` text of x86-64 code.
#
#   check-disasm.sh loop   <disassembly of the busy loop function>
#   check-disasm.sh window <disassembly of the worker function>
#
# loop:   the busy loop survived the optimiser: there is a backward jump (a jump to an address at or below its own).
# window: the counter's load and store are separate instructions: within WINDOW_LINES (default 8) lines of the call
#         to the busy loop there is no locked read-modify-write (lock xadd, cmpxchg, add, sub, inc, dec).
# Exits 0 when the check holds, 1 when it does not, 2 when the text has nothing to check (no jump at all, or no call
# to the busy loop), 64 for a usage error.

set -u

if [ $# -ne 2 ] || { [ "$1" != loop ] && [ "$1" != window ]; }; then
  echo "usage: $0 loop|window <disassembly-file>" >&2
  exit 64
fi
check=$1
file=$2

if [ "$check" = loop ]; then
  awk '
    function hex(s,   i, n, c) {
      n = 0
      gsub(/[ \t]/, "", s)
      for (i = 1; i <= length(s); i++) { c = index("0123456789abcdef", substr(s, i, 1)) - 1; n = n * 16 + c }
      return n
    }
    match($0, /^ *[0-9a-f]+:/) {
      here = hex(substr($0, RSTART, RLENGTH - 1))
      if (match($0, /[ \t]j[a-z]+[ \t]+[0-9a-f]+/)) {
        jumps++
        text = substr($0, RSTART, RLENGTH)
        sub(/^[ \t]j[a-z]+[ \t]+/, "", text)
        if (hex(text) <= here) backward++
      }
    }
    END { if (backward > 0) exit 0; if (jumps == 0) exit 2; exit 1 }
  ' "$file"
  exit $?
fi

lines=${WINDOW_LINES:-8}
awk -v span="$lines" '
  { text[NR] = $0 }
  /call.*spin/ && !call { call = NR }
  END {
    if (!call) exit 2
    first = call - span; if (first < 1) first = 1
    last = call + span; if (last > NR) last = NR
    for (i = first; i <= last; i++) {
      if (text[i] ~ /lock[ \t]+(xadd|cmpxchg|add|sub|inc|dec)/) exit 1
    }
    exit 0
  }
' "$file"
