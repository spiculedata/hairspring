#!/bin/sh
# check_cxx_mixed.sh <compile-log> <events-file-or-> <clang|other>
#
# tests/fixtures/cxx_mixed.cc puts an inline-function site and an ordinary-function site in one file.
#   clang: it must have compiled, and both ids must have catalog lines and no incomplete-catalog diagnostic.
#   other (g++): it must have FAILED to compile with "section type conflict". This is a known limitation, observed
#   with g++ 13.3 on Linux x86-64 in this layout; if a compiler accepts the file this reports it, so the
#   documentation can be revisited. The general rule behind it is an inference.
log=$1
ev=$2
family=$3
if [ "$family" = clang ]; then
  for id in mixed.inline mixed.ordinary; do
    grep -q "\"kind\":\"catalog\".*\"id\":\"$id\"" "$ev" || { echo "FAIL $id is not in the catalog"; exit 1; }
  done
  if grep -q 'incomplete catalog' "$ev"; then echo "FAIL clang announced an incomplete catalog"; exit 1; fi
  echo "ok   mixed inline and ordinary call sites compile and are catalogued"
  exit 0
fi
if grep -q 'section type conflict' "$log"; then
  echo "ok   known limitation: mixed inline and ordinary call sites do not compile (section type conflict)"
  exit 0
fi
echo "FAIL the mixed file did not fail with a section type conflict (limitation not reproduced); see $log"
exit 1
