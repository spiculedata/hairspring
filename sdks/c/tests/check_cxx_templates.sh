#!/bin/sh
# check_cxx_templates.sh <events-file> <strict|either>
#
# Reads the report of tests/fixtures/cxx_templates.cc. Every id must either have a catalog line (the compiler put the
# record in the section) or the report must say "incomplete catalog" and count it in end.dropped (the compiler did
# not). "strict" accepts only the first. Prints one line saying which happened; exits 1 on a silent loss.
ev=$1
mode=$2
ids="tmpl.zero tmpl.member tmpl.inline tmpl.main"
missing=""
for id in $ids; do
  grep -q "\"kind\":\"catalog\".*\"id\":\"$id\"" "$ev" || missing="$missing $id"
done
announced=no
if grep -q 'incomplete catalog' "$ev" && grep '"kind":"end"' "$ev" | grep -vq '"dropped":                   0'; then
  announced=yes
fi
if [ -z "$missing" ]; then
  if [ "$announced" = yes ]; then
    echo "FAIL every id is catalogued but an incomplete catalog was announced"
    exit 1
  fi
  echo "ok   template call sites are in the catalog"
  exit 0
fi
if [ "$mode" = strict ]; then
  echo "FAIL not in the catalog:$missing"
  exit 1
fi
if [ "$announced" = yes ]; then
  echo "ok   template call sites missing from the catalog ($missing ) were announced"
  exit 0
fi
echo "FAIL silent loss, not in the catalog:$missing"
exit 1
