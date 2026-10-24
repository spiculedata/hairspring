#!/bin/sh
# Build-record check B2 for the Java fixture, on the bytecode: the text of `javap -c -p -cp <classes> langat.Race`.
#
#   check-window-java.sh <javap-output-file>
#
# The contract's window is a load of the shared counter, the busy loop and a store of the counter, with nothing between
# them that calls, locks or allocates. This reads the two worker loops, racyIterations and lockedIterations, and requires
# that between every `getfield counter` and the `putfield counter` that follows it there is exactly one call, the
# invokestatic of `spin:(I)J`, and no other invoke, no monitor instruction, no allocation and no throw. In
# racyIterations there is no monitor instruction at all; in lockedIterations a monitorenter comes before the load and a
# monitorexit after the store. It prints the instructions of each window.
#
# This proves what the source compiles to, not what the JIT makes of it: the JVM may keep the counter in a register or
# move the plain load past the busy loop in compiled code (see the fixture's README). The interpreter and the C1 compiler,
# which the experiment's two flag profiles use, do not.
# Exits 0 when both loops hold, 1 when not or when the file cannot be read, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <javap-output-file>" >&2
  exit 64
fi
file=$1
if [ ! -r "$file" ]; then
  echo "FAIL $file cannot be read" >&2
  exit 1
fi

awk '
  function fail(why) { print "FAIL " method ": " why > "/dev/stderr"; bad = 1 }
  /^  [a-z][^(]*\(.*\);$/ {
    name = $0
    sub(/\(.*/, "", name)
    n = split(name, parts, " ")
    method = parts[n]
    inwin = 0
    next
  }
  method != "racyIterations" && method != "lockedIterations" { next }
  $1 ~ /^[0-9]+:$/ {
    op = $2
    line = $0
    if (op == "monitorenter") { enters[method]++; if (!(method in firstenter)) firstenter[method] = $1 + 0 }
    if (op == "monitorexit") { exits[method]++; lastexit[method] = $1 + 0 }
    if (op == "getfield" && line ~ /Field counter:J/) {
      if (inwin) fail("a load of the counter inside a window")
      inwin = 1; spins = 0; text = line "\n"; startat[method] = $1 + 0
      next
    }
    if (inwin) {
      text = text line "\n"
      if (op == "putfield" && line ~ /Field counter:J/) {
        inwin = 0
        if (spins != 1) fail("the window holds " spins " calls of spin, want exactly one")
        windows[method]++
        endat[method] = $1 + 0
        shown[method] = shown[method] text
      } else if (op == "invokestatic" && line ~ /Method spin:\(I\)J/) {
        spins++
      } else if (op ~ /^(invoke|monitor|new|anewarray|multianewarray|athrow)/) {
        fail("the window holds " op ": " line)
      }
    }
  }
  END {
    split("racyIterations lockedIterations", names, " ")
    for (i = 1; i <= 2; i++) {
      method = names[i]
      if (!(method in windows)) { fail("no window found (no load of the counter followed by its store)") ; continue }
      print "--- " method ": " windows[method] " window(s), between the load and the store of the counter"
      printf "%s", shown[method]
    }
    method = "racyIterations"
    if (enters[method] > 0) fail("a monitor is taken in the unlocked loop")
    method = "lockedIterations"
    if (enters[method] < 1 || exits[method] < 1) fail("the locked loop takes no monitor")
    else if (firstenter[method] > startat[method] || lastexit[method] < endat[method]) fail("the monitor does not enclose the window")
    exit bad
  }
' "$file"
status=$?
if [ "$status" -eq 0 ]; then
  echo "ok   in both loops the window is a load of the counter, one call of spin and a store of the counter; the unlocked loop takes no monitor and the locked loop holds one across the window"
fi
exit $status
