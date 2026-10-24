#!/bin/sh
# Checks that a C# source file reads the monotonic clock exactly once, with the plain name.
#
#   check-stopwatch-use.sh <source-file>
#
# check-sources-dotnet.sh --allow-stopwatch lets `Stopwatch.GetTimestamp(` through for the probe. This is the other half:
# the file that holds it must contain exactly one, and no aliased or static import of Stopwatch (`using S = ...Stopwatch;`,
# `using static ...Stopwatch;`), which would hide a second read from the text search.
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
if grep -Eq 'using[[:space:]]+(static[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[[:space:]]*)?[A-Za-z0-9_.]*Stopwatch' "$file"; then
  echo "FAIL $file imports Stopwatch under another name or statically" >&2
  exit 1
fi
count=$(grep -o 'Stopwatch\.GetTimestamp(' "$file" | wc -l | tr -d ' ')
if [ "$count" -ne 1 ]; then
  echo "FAIL $file has $count uses of Stopwatch.GetTimestamp, want exactly one" >&2
  exit 1
fi
echo "ok   $file has exactly one Stopwatch.GetTimestamp and no aliased import"
