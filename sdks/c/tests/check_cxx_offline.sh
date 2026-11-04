#!/bin/sh
# check_cxx_offline.sh <binary> <strict|either>
#
# The offline catalog is the hs_catalog_str_v1 section of the binary, so a tool can read the sites without running it.
# In "strict" mode every id of tests/fixtures/cxx_templates*.cc must be in that section. Reading a section needs
# objdump, so this is skipped where objdump or ELF is not available (macOS keeps it in __DATA,__hs_catstr).
bin=$1
mode=$2
if [ "$mode" != strict ]; then echo "skip: offline catalog not held to a rule for this compiler"; exit 0; fi
if [ "$(uname -s)" != Linux ] || ! command -v objdump >/dev/null 2>&1; then
  echo "skip: objdump on Linux is needed to read the section"
  exit 0
fi
dump=$(objdump -s -j hs_catalog_str_v1 "$bin") || { echo "FAIL no hs_catalog_str_v1 section in $bin"; exit 1; }
# objdump prints the bytes as hex and as text; the text column turns unprintable bytes into dots, which is enough to
# find an id once the text columns are joined (an id never contains a dot-sensitive split: ids are matched whole).
text=$(printf '%s\n' "$dump" | sed -n 's/^ [0-9a-f][0-9a-f]* [0-9a-f ]\{35\}  *//p' | tr -d '\n')
for id in tmpl.zero tmpl.member tmpl.inline tmpl.main; do
  case $text in
    *"id\":\"$id\""*) ;;
    *) echo "FAIL $id is not in the offline catalog section"; exit 1 ;;
  esac
done
echo "ok   every template call site is in the offline catalog section"
