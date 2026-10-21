#!/bin/sh
# Checks that a Rust source file reads the clock exactly once, with the plain name.
#
#   check-instant-use.sh <source-file>
#
# check-sources.sh --allow-instant lets `Instant::now` through for the probe. This is the other half: the file that holds it
# must contain exactly one `Instant::now` and no renamed import of `Instant` (`Instant as ...`), which would hide a second
# read from the text search.
# Exits 0 when it holds, 1 when not, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <source-file>" >&2
  exit 64
fi
file=$1
if [ ! -r "$file" ]; then
  echo "FAIL $file cannot be read" >&2
  exit 1
fi
if grep -Eq 'Instant[[:space:]]+as[[:space:]]' "$file"; then
  echo "FAIL $file renames Instant on import" >&2
  exit 1
fi
count=$(grep -o 'Instant::now' "$file" | wc -l | tr -d ' ')
if [ "$count" -ne 1 ]; then
  echo "FAIL $file has $count uses of Instant::now, want exactly one" >&2
  exit 1
fi
echo "ok   $file has exactly one Instant::now and no renamed import"
