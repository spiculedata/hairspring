#!/bin/sh
# Source check of a lost-update fixture: it must not read the scheduler seed, /proc, the CPU count or the clock.
#
#   check-sources.sh <source-file>...
#
# The fixture contract forbids these inputs outside the language runtime and the stated random source: any of them
# would make a run depend on something the deterministic scheduler does not control. This is a plain text search, so
# a forbidden word in a comment is reported too; keep the sources free of them.
# Exits 0 when every file is clean, 1 when a forbidden token is found, 64 for a usage error.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <source-file>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|sysconf|nproc|NumCPU|hardware_concurrency|clock_gettime|gettimeofday|time\.Now'
status=0
for file in "$@"; do
  if grep -En "$forbidden" "$file" >&2; then
    echo "FAIL $file reads the seed, /proc, the CPU count or the clock" >&2
    status=1
  fi
done
exit $status
