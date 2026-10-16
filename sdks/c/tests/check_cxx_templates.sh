#!/bin/sh
# check_cxx_templates.sh <events-file> <strict|either>
#
# Reads the report of tests/fixtures/cxx_templates.cc (events on stderr by default, so one plain-text diagnostic line
# can sit among the protocol lines). Every id must either have a catalog line (the compiler put the record in the
# section) or the report must announce the loss: the line "incomplete catalog: <n> evaluation(s)" with n equal to the
# evaluations the fixture makes at the ids that are absent, and a non-zero end.dropped. "strict" accepts only the
# first. Prints one line saying which happened; exits 1 on a silent or miscounted loss.
ev=$1
mode=$2
# id and the number of evaluations the fixture makes there
sites="tmpl.zero:3 tmpl.member:2 tmpl.inline:1 tmpl.main:1"
missing=""
expected=0
for entry in $sites; do
  id=${entry%%:*}
  n=${entry##*:}
  if ! grep -q "\"kind\":\"catalog\".*\"id\":\"$id\"" "$ev"; then
    missing="$missing $id"
    expected=$((expected + n))
  fi
done
announced=$(sed -n 's/^hairspring: incomplete catalog: \([0-9][0-9]*\) evaluation.*/\1/p' "$ev" | head -1)
counted=no
if grep '"kind":"end"' "$ev" | grep -vq '"dropped":                   0'; then counted=yes; fi
if [ -z "$missing" ]; then
  if [ -n "$announced" ]; then
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
if [ "$announced" = "$expected" ] && [ "$counted" = yes ]; then
  echo "ok   template call sites missing from the catalog ($missing ) were announced: $expected evaluation(s)"
  exit 0
fi
echo "FAIL loss not announced correctly: missing$missing, expected $expected, announced '${announced:-none}', dropped counted: $counted"
exit 1
