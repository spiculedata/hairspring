#!/bin/sh
# Source check of the Java service fixture's seeded bug: the order of the steps in Store.handle.
#
#   check-svc-window.sh <Store.java>
#
# The racy branch (the else branch of `if (mode.atomicClaim())`) must hold, each exactly once and in this order: the check
# `applied.containsKey(t.key())`, the validation `validate(mode.window())`, the `apply(t)` and the record
# `applied.put(t.key()`. The atomic branch must hold `applied.putIfAbsent(t.key(), CLAIMED)`, then the same three. A
# change that records the key before validating, or claims it with a plain check in the fixed build, fails. This is a text
# check of the source, not of the bytecode or the JIT.
# Prints OK or FAIL <why>. Exits 0 when both branches hold, 1 when not or when the file cannot be read, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <Store.java>" >&2
  exit 64
fi
if [ ! -r "$1" ]; then
  echo "FAIL $1 cannot be read" >&2
  exit 1
fi
awk '
  function order(text, a, b, c, d,   pa, pb, pc, pd) {
    pa = index(text, a); pb = index(text, b); pc = index(text, c); pd = index(text, d)
    if (!pa || !pb || !pc || !pd) return 0
    if (count(text, a) != 1 || count(text, b) != 1 || count(text, c) != 1 || count(text, d) != 1) return 0
    return pa < pb && pb < pc && pc < pd
  }
  function count(text, s,   n, i) { n = 0; while ((i = index(text, s)) > 0) { n++; text = substr(text, i + length(s)) } return n }
  /if \(mode\.atomicClaim\(\)\) \{/ { state = "atomic"; next }
  state == "atomic" && /^        \} else \{$/ { state = "racy"; next }
  state == "racy" && /^        \}$/ { state = "done"; next }
  state == "atomic" { atomic = atomic $0 "\n" }
  state == "racy" { racy = racy $0 "\n" }
  END {
    if (state != "done") { print "FAIL the two branches of handle() were not found"; exit 1 }
    if (!order(racy, "applied.containsKey(t.key())", "validate(mode.window())", "apply(t)", "applied.put(t.key()")) {
      print "FAIL racy branch: want containsKey, validate, apply, put, each once, in that order"; exit 1 }
    if (!order(atomic, "applied.putIfAbsent(t.key(), CLAIMED)", "validate(mode.window())", "apply(t)", "applied.put(t.key()")) {
      print "FAIL atomic branch: want putIfAbsent, validate, apply, put, each once, in that order"; exit 1 }
    print "OK window shape: racy containsKey < validate < apply < put; atomic putIfAbsent < validate < apply < put"
  }' "$1"
