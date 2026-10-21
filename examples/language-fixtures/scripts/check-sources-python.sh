#!/bin/sh
# Source check of the Python fixture files: they must not read the scheduler seed, /proc, the CPU count, the
# environment or the clock.
#
#   check-sources-python.sh [--allow-monotonic-ns] <source-file>...
#
# The fixture contract forbids these inputs outside the language runtime and the stated random source: the seed
# (HAIRSPRING_SEED), /proc, the CPU count (cpu_count, process_cpu_count, sched_getaffinity, nproc), the environment
# (os.environ, os.getenv) and the clock (time.time, time.monotonic, time.perf_counter, time.process_time, clock_gettime,
# datetime). `--allow-monotonic-ns` is for probe_py.py alone: its output line carries the monotonic clock before and after
# the run (vtime_ns, t_start_ns, t_end_ns), so `time.monotonic_ns` is allowed there and nothing else of the clock is.
# at_py.py is checked without the flag: it reads no clock at all. The SDK package is not checked: it is the runtime side
# (it reads the environment and the auxiliary vector by design). This is a plain text search, so a forbidden word in a
# comment is reported too; keep the sources free of them.
# Exits 0 when every file is clean, 1 when a forbidden token is found, 64 for a usage error.

set -u

allow_ns=0
if [ "${1:-}" = "--allow-monotonic-ns" ]; then
  allow_ns=1
  shift
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--allow-monotonic-ns] <source-file>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|cpu_count|sched_getaffinity|nproc|os\.environ|environb|getenv|time\.time|time\.monotonic|perf_counter|process_time|clock_gettime|datetime'
status=0
for file in "$@"; do
  # A missing or unreadable file is a failure, never a pass.
  if [ ! -r "$file" ] || [ -d "$file" ]; then
    echo "FAIL $file is missing or unreadable" >&2
    status=1
    continue
  fi
  if [ "$allow_ns" -eq 1 ]; then
    found=$(sed 's/time\.monotonic_ns//g' "$file" | grep -En "$forbidden")
  else
    found=$(grep -En "$forbidden" "$file")
  fi
  if [ -n "$found" ]; then
    printf '%s\n' "$found" >&2
    echo "FAIL $file reads the seed, /proc, the CPU count, the environment or the clock" >&2
    status=1
  fi
done
exit $status
