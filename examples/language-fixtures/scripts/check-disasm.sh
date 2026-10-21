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

lines=${WINDOW_LINES:-12}
awk -v span="$lines" -v want_unlocked="${UNLOCKED_PATH:-0}" '
  # Splits an operand list at its top-level comma into SRC and DST (commas inside parentheses stay).
  function split_operands(o,   i, c, depth) {
    SRC = o; DST = ""; depth = 0
    for (i = 1; i <= length(o); i++) {
      c = substr(o, i, 1)
      if (c == "(") depth++
      else if (c == ")") depth--
      else if (c == "," && depth == 0) { SRC = substr(o, 1, i - 1); DST = substr(o, i + 1); return }
    }
  }
  # Parses line n: MNEM, SRC, DST and KEY (the memory operand, or the address in the objdump comment).
  function parse(n,   t, o) {
    t = text[n]
    sub(/^[ \t]*[0-9a-f]+:[ \t]+/, "", t)
    MNEM = t; sub(/[ \t].*$/, "", MNEM)
    o = t; sub(/^[^ \t]+[ \t]+/, "", o); sub(/[ \t]*#.*$/, "", o)
    split_operands(o)
    KEY = ""
    if (match(text[n], /# [0-9a-f]+/)) KEY = substr(text[n], RSTART + 2, RLENGTH - 2)
  }
  # Memory operands on the stack are spill slots, not the counter.
  function is_counter_operand(op) { return op ~ /\(/ && op !~ /\(%r[sb]p\)/ }
  function is_load(n) { parse(n); if (MNEM ~ /^mov/ && SRC ~ /\(/ && DST ~ /^%/ && is_counter_operand(SRC)) { LOADKEY = KEY != "" ? KEY : SRC; return 1 } return 0 }
  function is_store(n) { parse(n); if (MNEM ~ /^(mov|xchg)/ && DST ~ /\(/ && SRC !~ /\(/ && is_counter_operand(DST)) { STOREKEY = KEY != "" ? KEY : DST; return 1 } return 0 }
  # True if lines a..b hold a call other than the one at line c, or a locked read-modify-write.
  function dirty(a, b, c,   k) {
    for (k = a; k <= b; k++) {
      if (k != c && text[k] ~ /[ \t]call[ \t]/) return 1
      if (text[k] ~ /lock[ \t]+(xadd|cmpxchg|add|sub|inc|dec)/) return 1
    }
    return 0
  }
  # True if no mutex lock is called in the 8 lines before line n.
  function unlocked_before(n,   k) {
    for (k = (n > 8 ? n - 8 : 1); k < n; k++) if (text[k] ~ /call.*mutex_lock/) return 0
    return 1
  }
  { text[NR] = $0 }
  END {
    for (n = 1; n <= NR; n++) if (text[n] ~ /call.*spin/) calls[++ncalls] = n
    if (ncalls == 0) exit 2
    bad = 0; unlocked = 0
    for (k = 1; k <= ncalls; k++) {
      c = calls[k]; ok = 0
      for (i = c - 1; i >= 1 && i >= c - span && !ok; i--) {
        if (!is_load(i)) continue
        loadkey = LOADKEY
        for (j = c + 1; j <= NR && j <= c + span && !ok; j++) {
          if (is_store(j) && STOREKEY == loadkey && !dirty(i + 1, j - 1, c)) { ok = 1; loadline = i }
        }
      }
      if (!ok) { print "call to the busy loop at line " c ": no load -> call -> store of the same address" > "/dev/stderr"; bad = 1; continue }
      if (unlocked_before(loadline)) unlocked++
    }
    if (bad) exit 1
    if (want_unlocked == "1" && unlocked == 0) { print "no copy of the window outside a mutex" > "/dev/stderr"; exit 1 }
    exit 0
  }
' "$file"
